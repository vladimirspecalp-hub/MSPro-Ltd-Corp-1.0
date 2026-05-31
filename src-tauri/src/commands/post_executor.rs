//! Post Executor — Phase 11B-1 (v1.0.24).
//!
//! Когда Диспетчер сделал `forward_to_post(slug, refined_prompt)` — этот модуль
//! spawn'ит **реальный** Claude CLI subprocess со своим agent.md (генерируется
//! из `posts.system_prompt_md`), в строгой sandbox-папке `Outbox/<task_id>/`.
//!
//! Главное отличие от `claude_bridge::run_claude_cli` (мозг Гендира/Диспетчера):
//!   * agent.md имеет `tools: [Read, Write, Edit, Bash]` — пост ДОЛЖЕН физически
//!     создавать артефакты (.docx/.xlsx/.txt), а не только генерировать XML.
//!   * cwd = `Outbox/<task_id>/` — агент пишет файлы строго туда.
//!   * После exit — мы сканируем директорию и регистрируем все новые файлы
//!     через `artifacts::register_artifact()` → они появляются в UI «Awaiting».
//!
//! Lifecycle:
//!   1. `trigger_post_executor()` — non-blocking entry-point, спавнит фоновый task
//!   2. Внутри: lookup `posts` → `ensure_post_agent_md` → mkdir Outbox →
//!      spawn claude.exe → write stdin → wait → scan dir → register artifacts →
//!      emit dispatcher-task-changed
//!   3. Job Object (см. lib.rs::setup) гарантирует kill всех claude.exe при
//!      выходе MSPro даже в случае crash.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

use crate::commands::artifacts;
use crate::commands::claude_bridge::hide_console;
use crate::commands::dispatcher;
use crate::db::WritePool;
use crate::outbox;
use crate::settings::{AppSettings, SettingsStore};
use crate::vault::{sanitize_post_slug, VaultState};

/// Registry активных пост-агентов: task_id → PID.
/// Защищает от двойного spawn-а и позволяет ручной cancel.
#[derive(Default, Clone)]
pub struct PostExecutorRegistry {
    pub running: Arc<AsyncMutex<HashMap<String, u32>>>,
}

#[derive(Debug, Serialize)]
pub struct PostExecResult {
    pub task_id: String,
    pub exit_code: i32,
    pub artifacts_count: usize,
    pub elapsed_ms: u128,
}

/// Non-blocking entry-point. Спавнит фоновую async-задачу и возвращается
/// мгновенно — Диспетчер не блокируется ожиданием пост-агента.
pub fn trigger_post_executor(
    task_id: String,
    post_slug: String,
    refined_prompt: String,
    expected_artifact: Option<String>,
    app: AppHandle,
) {
    log::info!(
        "post_executor: trigger task={task_id} slug={post_slug} expected={:?}",
        expected_artifact
    );

    tauri::async_runtime::spawn(async move {
        // Достаём всё нужное из Tauri state.
        let db = match app.try_state::<WritePool>() {
            Some(s) => s.inner().clone(),
            None => {
                log::error!("post_executor: WritePool not in state — abort");
                return;
            }
        };
        let vault = match app.try_state::<VaultState>() {
            Some(s) => s.inner().clone(),
            None => {
                log::error!("post_executor: VaultState not in state — abort");
                return;
            }
        };
        let settings_snapshot = match app.try_state::<SettingsStore>() {
            Some(s) => s.data.lock().unwrap().clone(),
            None => {
                log::error!("post_executor: SettingsStore not in state — abort");
                return;
            }
        };
        let registry = match app.try_state::<PostExecutorRegistry>() {
            Some(s) => s.inner().clone(),
            None => {
                log::error!("post_executor: PostExecutorRegistry not in state — abort");
                return;
            }
        };

        // Защита от двойного spawn — если уже бежим, выходим тихо.
        {
            let map = registry.running.lock().await;
            if map.contains_key(&task_id) {
                log::warn!(
                    "post_executor: task {task_id} already running, skip duplicate trigger"
                );
                return;
            }
        }

        let result = run_claude_cli_for_post(
            &task_id,
            &post_slug,
            &refined_prompt,
            expected_artifact.as_deref(),
            &settings_snapshot,
            &db,
            &vault,
            &registry,
            &app,
        )
        .await;

        // Cleanup registry.
        {
            let mut map = registry.running.lock().await;
            map.remove(&task_id);
        }

        match result {
            Ok(r) => log::info!(
                "post_executor: task {} done exit={} artifacts={} elapsed_ms={}",
                r.task_id, r.exit_code, r.artifacts_count, r.elapsed_ms
            ),
            Err(e) => {
                log::error!("post_executor: task {task_id} failed: {e}");
                // Fail-task мы уже звали внутри run_claude_cli_for_post — здесь
                // только лог.
            }
        }
    });
}

/// Главная функция: spawn агента и обработка результата.
#[allow(clippy::too_many_arguments)]
async fn run_claude_cli_for_post(
    task_id: &str,
    post_slug: &str,
    refined_prompt: &str,
    _expected_artifact: Option<&str>,
    settings: &AppSettings,
    db: &WritePool,
    vault: &VaultState,
    registry: &PostExecutorRegistry,
    app: &AppHandle,
) -> Result<PostExecResult, String> {
    let started_at = Instant::now();

    // 1. Lookup post row — нужны system_prompt_md + preferred_model.
    let row: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT slug, system_prompt_md, preferred_model FROM posts WHERE slug = ?",
    )
    .bind(post_slug)
    .fetch_optional(&db.0)
    .await
    .map_err(|e| format!("posts lookup: {e}"))?;

    let (slug, system_prompt_opt, preferred_model_opt) = row
        .ok_or_else(|| format!("post '{post_slug}' not found"))?;

    let system_prompt = system_prompt_opt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            format!(
                "post '{slug}' не имеет system_prompt_md — задай его в Posts Editor (🧠)"
            )
        })?;

    // Выбор модели: preferred_model → settings.claude_cli_model → дефолт.
    // ВАЖНО (Архитектор): Qwen через HTTP не умеет MCP — если preferred_model
    // указывает на qwen, в 11B-1 всё равно пускаем через claude.exe. Полноценная
    // поддержка Qwen — будущая фаза. Сейчас просто переопределяем модель.
    let model = preferred_model_opt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.to_lowercase().starts_with("qwen"))
        .map(|s| s.to_string())
        .unwrap_or_else(|| settings.claude_cli_model.clone());

    // 2. Гарантируем agent.md (~/.claude/agents/mspro-<slug>.md).
    let safe_slug = sanitize_post_slug(&slug).map_err(|e| format!("slug invalid: {e}"))?;
    let agent_name = format!("mspro-{}", safe_slug);
    let _agent_md_path = ensure_post_agent_md(&safe_slug, system_prompt, &model)?;

    // 3. Sandbox: <Outbox>/<task_id>/ (mkdir идемпотентно).
    let task_dir = outbox::task_outbox_dir(&vault.root, task_id)
        .map_err(|e| format!("task outbox dir: {e}"))?;

    log::info!(
        "post_executor: spawn agent={agent_name} model={model} cwd={} prompt_len={}",
        task_dir.display(),
        refined_prompt.len()
    );

    // Снимок «что было» в папке до спавна — для дельты после exit.
    let pre_snapshot = snapshot_dir(&task_dir);

    // Phase 1 (Iteration B): PAL killswitch. Флаг снимаем ОДИН раз (R-T-008).
    // pal_enabled → через PAL (orchestrator → ClaudeCliDriver) + run_logs.
    // false → legacy прямой spawn ниже (нетронут). default false (v1.0.34).
    if settings.pal_enabled {
        return run_via_pal(
            task_id, &slug, refined_prompt, system_prompt, &agent_name, &model,
            &task_dir, &pre_snapshot, settings, db, vault, app,
        )
        .await;
    }

    // 4. Spawn claude.exe.
    let mut cmd = Command::new(&settings.claude_cli_path);
    hide_console(&mut cmd);
    cmd.arg("--print")
        .arg("--output-format")
        .arg("text")
        .arg("--agent")
        .arg(&agent_name)
        .arg("--model")
        .arg(&model)
        // v1.0.24-fix: без этого флага Claude CLI в --print режиме отказывается
        // вызывать Write/Edit/Bash tools без интерактивного подтверждения.
        // Безопасно — cwd жёстко ограничен sandbox-папкой Outbox/<task_id>/.
        .arg("--dangerously-skip-permissions")
        .current_dir(&task_dir)
        .env("MSPRO_TASK_ID", task_id)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| {
            // Сразу помечаем задачу failed чтобы UI не висел.
            let task_id = task_id.to_string();
            let reason = format!("spawn failed: {e}");
            let db_clone = db.clone();
            let app_clone = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = dispatcher::fail_task_inner(task_id, reason, &db_clone, &app_clone).await;
            });
            format!("claude spawn failed: {e}")
        })?;

    // Регистрируем PID для cancel.
    let pid = child.id().unwrap_or(0);
    {
        let mut map = registry.running.lock().await;
        map.insert(task_id.to_string(), pid);
    }

    // 5. Передаём refined_prompt в stdin → close → child получает EOF.
    if let Some(mut stdin_pipe) = child.stdin.take() {
        if let Err(e) = stdin_pipe.write_all(refined_prompt.as_bytes()).await {
            log::warn!("post_executor: stdin write failed: {e}");
        }
        drop(stdin_pipe);
    }

    // 6. Wait с timeout. Параллельно собираем stderr для диагностики на fail.
    let timeout_secs = settings.post_executor_timeout_sec;
    let mut stderr_pipe = child.stderr.take();

    let wait_fut = child.wait();
    let result = timeout(Duration::from_secs(timeout_secs), wait_fut).await;

    let exit_code: i32 = match result {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(e)) => {
            log::warn!("post_executor: wait failed: {e}");
            -1
        }
        Err(_) => {
            log::warn!("post_executor: task {task_id} timeout {timeout_secs}s — killing");
            let _ = child.kill().await;
            -2
        }
    };

    // 7. Подбираем stderr (если процесс упал — там диагностика).
    let mut stderr_text = String::new();
    if let Some(p) = stderr_pipe.as_mut() {
        let _ = p.read_to_string(&mut stderr_text).await;
    }
    let stderr_tail = stderr_text
        .trim()
        .lines()
        .rev()
        .take(8)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n");

    let elapsed_ms = started_at.elapsed().as_millis();

    // 8. Сканируем директорию — что нового создал агент?
    let new_files = diff_dir(&task_dir, &pre_snapshot);
    log::info!(
        "post_executor: task {task_id} exit={exit_code} new_files={} elapsed={}ms",
        new_files.len(),
        elapsed_ms
    );

    // 9. Регистрируем артефакты.
    let mut registered = 0usize;
    for rel in &new_files {
        let mime = guess_mime_from_ext(rel);
        match artifacts::register_artifact(task_id, rel, mime.as_deref(), &slug, db, vault, app)
            .await
        {
            Ok(_) => registered += 1,
            Err(e) => log::warn!("post_executor: register {rel} failed: {e}"),
        }
    }

    // 10. Обновляем статус parent task в dispatcher_logs.
    if exit_code == 0 && registered > 0 {
        // ✅ Успех — task остаётся in_progress с outbox_path != null,
        // UI покажет в Awaiting (Владелец approve/reject).
        log::info!(
            "post_executor: task {task_id} produced {registered} artifacts — awaiting approval"
        );
    } else {
        // ❌ Fail / нет артефактов — bump attempts_count и fail_task.
        let _ = bump_attempts(task_id, db).await;
        let reason = if exit_code == -2 {
            format!("timeout {timeout_secs}s")
        } else if registered == 0 && exit_code == 0 {
            "claude finished but produced no artifacts".to_string()
        } else {
            format!("exit={exit_code}; stderr: {stderr_tail}")
        };
        let _ = dispatcher::fail_task_inner(task_id.to_string(), reason, db, app).await;
    }

    let _ = app.emit(
        "post-executor-finished",
        serde_json::json!({
            "task_id": task_id,
            "exit_code": exit_code,
            "artifacts": registered,
            "elapsed_ms": elapsed_ms,
        }),
    );

    Ok(PostExecResult {
        task_id: task_id.to_string(),
        exit_code,
        artifacts_count: registered,
        elapsed_ms,
    })
}

/// Создаёт `~/.claude/agents/mspro-<slug>.md` (или перезаписывает если отличается)
/// из `posts.system_prompt_md` Владельца.
///
/// КРИТИЧЕСКИ: tools: [Read, Write, Edit, Bash] — пост ДОЛЖЕН физически писать
/// файлы. Это отличие от Гендира/Диспетчера, где `tools: []` запрещает native tools.
pub fn ensure_post_agent_md(
    safe_slug: &str,
    system_prompt: &str,
    model: &str,
) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot resolve home dir".to_string())?;
    let dir = home.join(".claude").join("agents");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create agents dir: {e}"))?;
    let path = dir.join(format!("mspro-{}.md", safe_slug));

    let body = format!(
        "---\nname: mspro-{slug}\ndescription: MSPro-Ltd Corp пост-агент (slug={slug}). Получает task через stdin, создаёт артефакты в текущей рабочей директории (Outbox sandbox).\ntools: [Read, Write, Edit, Bash]\nmodel: {model}\n---\n\n{prompt}\n",
        slug = safe_slug,
        model = model,
        prompt = system_prompt.trim(),
    );

    // Идемпотентно: пишем только если содержимое отличается (избегаем лишних
    // mtime-updates и race с Claude CLI reload).
    let need_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != body,
        Err(_) => true,
    };
    if need_write {
        std::fs::write(&path, body).map_err(|e| format!("write agent.md: {e}"))?;
        log::info!("ensured post agent file: {}", path.display());
    }
    Ok(path)
}

/// Возвращает HashMap<rel_path, mtime_seconds> для всех файлов в директории.
fn snapshot_dir(dir: &Path) -> HashMap<String, u64> {
    let mut out = HashMap::new();
    if !dir.exists() {
        return out;
    }
    walk_dir(dir, dir, &mut out);
    out
}

fn walk_dir(root: &Path, cur: &Path, out: &mut HashMap<String, u64>) {
    let rd = match std::fs::read_dir(cur) {
        Ok(r) => r,
        Err(_) => return,
    };
    for entry in rd.flatten() {
        let p = entry.path();
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.is_dir() {
            walk_dir(root, &p, out);
        } else if meta.is_file() {
            if let Ok(rel) = p.strip_prefix(root) {
                let rel_s = rel
                    .to_string_lossy()
                    .replace('\\', "/")
                    .to_string();
                let mtime_secs = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                out.insert(rel_s, mtime_secs);
            }
        }
    }
}

/// Возвращает список rel_path файлов, которые появились или были изменены
/// относительно `before`.
fn diff_dir(dir: &Path, before: &HashMap<String, u64>) -> Vec<String> {
    let after = snapshot_dir(dir);
    let mut new_or_changed = Vec::new();
    for (rel, mtime) in after {
        match before.get(&rel) {
            Some(&prev_mtime) if prev_mtime == mtime => continue, // unchanged
            _ => new_or_changed.push(rel),
        }
    }
    new_or_changed.sort();
    new_or_changed
}

fn guess_mime_from_ext(rel_path: &str) -> Option<String> {
    let ext = rel_path
        .rsplit('.')
        .next()
        .map(str::to_lowercase)
        .unwrap_or_default();
    let mime = match ext.as_str() {
        "txt" | "md" => "text/plain",
        "json" => "application/json",
        "html" | "htm" => "text/html",
        "docx" => {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
        }
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => {
            "application/vnd.openxmlformats-officedocument.presentationml.presentation"
        }
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "csv" => "text/csv",
        _ => return None,
    };
    Some(mime.to_string())
}

async fn bump_attempts(task_id: &str, db: &WritePool) -> Result<(), String> {
    sqlx::query(
        "UPDATE dispatcher_logs SET attempts_count = attempts_count + 1 WHERE id = ?",
    )
    .bind(task_id)
    .execute(&db.0)
    .await
    .map(|_| ())
    .map_err(|e| format!("bump_attempts: {e}"))
}

/// Map модели → Tier (Phase 1 interim, до post_runtime в Срезе 3).
fn tier_for_model(m: &str) -> crate::pal::Tier {
    let l = m.to_lowercase();
    if l.contains("opus") {
        crate::pal::Tier::T1
    } else if l.contains("sonnet") {
        crate::pal::Tier::T2
    } else if l.contains("qwen") || l.contains("haiku") {
        crate::pal::Tier::T3
    } else {
        crate::pal::Tier::T2
    }
}

/// Phase 1 (Iteration B) — путь исполнения через PAL.
/// Повторяет downstream logic legacy (diff_dir / register_artifact / fail/success),
/// но spawn делает orchestrator → ClaudeCliDriver + пишет run_logs.
/// Срез 1: один провайдер (claude_cli), без fallback chain (Срез 2).
#[allow(clippy::too_many_arguments)]
async fn run_via_pal(
    task_id: &str,
    slug: &str,
    refined_prompt: &str,
    system_prompt: &str,
    agent_name: &str,
    model: &str,
    task_dir: &Path,
    pre_snapshot: &HashMap<String, u64>,
    settings: &AppSettings,
    db: &WritePool,
    vault: &VaultState,
    app: &AppHandle,
) -> Result<PostExecResult, String> {
    use crate::pal::claude_cli_driver::ClaudeCliDriver;
    use crate::pal::{orchestrator, ProviderRequest, RequestTrace};
    use crate::run_logger::{insert_run_log, RunLogEntry};

    let started = Instant::now();
    let tier = tier_for_model(model);
    let driver = ClaudeCliDriver::new(
        "claude_cli".to_string(),
        settings.claude_cli_path.clone(),
        model.to_string(),
    );
    let request = ProviderRequest {
        system_prompt: system_prompt.to_string(),
        user_message: refined_prompt.to_string(),
        tier,
        timeout: None, // orchestrator возьмёт Tier::default_timeout (T1=600)
        max_turns: None,
        model_override: None,
        workspace_path: Some(task_dir.to_path_buf()),
        agent_slug: Some(agent_name.to_string()),
        mcp_bindings: Vec::new(),
        trace: RequestTrace {
            post_slug: slug.to_string(),
            dispatcher_log_id: Some(task_id.to_string()),
            attempt_id: format!("{task_id}-0"),
            attempt_number: 0,
        },
    };

    log::info!("run_via_pal: task={task_id} slug={slug} tier={} model={model}", tier.as_str());
    let result = orchestrator::pal_invoke(&driver, request).await;
    let elapsed_ms = started.elapsed().as_millis();

    // run_logs запись (успех или ошибка).
    let (success, error_kind, raw_output, latency) = match &result {
        Ok(resp) => (true, None, Some(resp.text.clone()), resp.latency_ms as i64),
        Err(e) => (false, Some(e.kind_str().to_string()), Some(e.to_string()), elapsed_ms as i64),
    };
    let entry = RunLogEntry {
        task_id: Some(task_id.to_string()),
        post_slug: Some(slug.to_string()),
        provider_id: "claude_cli".to_string(),
        model_used: Some(model.to_string()),
        tier: Some(tier.as_str().to_string()),
        tokens_in: 0,
        tokens_out: 0,
        latency_ms: latency,
        cost_usd: 0.0,
        success,
        fallback_used: false,
        attempt_number: 0,
        error_kind,
        raw_output,
    };
    if let Err(e) = insert_run_log(db, entry).await {
        log::warn!("run_via_pal: run_log insert failed: {e}");
    }

    // Артефакты — тот же diff_dir + register, что и в legacy.
    let new_files = diff_dir(task_dir, pre_snapshot);
    let mut registered = 0usize;
    for rel in &new_files {
        let mime = guess_mime_from_ext(rel);
        match artifacts::register_artifact(task_id, rel, mime.as_deref(), slug, db, vault, app).await {
            Ok(_) => registered += 1,
            Err(e) => log::warn!("run_via_pal: register {rel} failed: {e}"),
        }
    }

    let exit_code = match &result {
        Ok(_) if registered > 0 => {
            log::info!("run_via_pal: task {task_id} produced {registered} artifacts — awaiting approval");
            0
        }
        Ok(_) => {
            let _ = bump_attempts(task_id, db).await;
            let _ = dispatcher::fail_task_inner(
                task_id.to_string(),
                "PAL finished but produced no artifacts".to_string(),
                db,
                app,
            )
            .await;
            0
        }
        Err(e) => {
            let _ = bump_attempts(task_id, db).await;
            let _ = dispatcher::fail_task_inner(task_id.to_string(), format!("PAL error: {e}"), db, app).await;
            -1
        }
    };

    let _ = app.emit(
        "post-executor-finished",
        serde_json::json!({
            "task_id": task_id,
            "exit_code": exit_code,
            "artifacts": registered,
            "elapsed_ms": elapsed_ms,
            "pal": true,
        }),
    );

    Ok(PostExecResult {
        task_id: task_id.to_string(),
        exit_code,
        artifacts_count: registered,
        elapsed_ms,
    })
}

// ---------------------------------------------------------------------------
// Tauri commands
// ---------------------------------------------------------------------------

/// Ручной cancel: убивает claude.exe пост-агента по task_id.
#[tauri::command]
pub async fn cancel_post_executor(
    task_id: String,
    registry: State<'_, PostExecutorRegistry>,
) -> Result<bool, String> {
    let pid = {
        let map = registry.running.lock().await;
        map.get(&task_id).copied()
    };
    let Some(pid) = pid else {
        return Ok(false);
    };

    #[cfg(windows)]
    {
        use sysinfo::Pid;
        let mut sys = sysinfo::System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
        if let Some(proc_) = sys.process(Pid::from_u32(pid)) {
            let killed = proc_.kill();
            log::info!("cancel_post_executor: kill pid={pid} task={task_id} ok={killed}");
            return Ok(killed);
        }
    }

    let _ = task_id;
    Ok(false)
}

/// Cleanup orphan claude.exe — оставшиеся от предыдущего crash MSPro.
/// Идентифицируется по env-var `MSPRO_TASK_ID`. Запускается на startup.
///
/// На Windows + Job Object этот cleanup является ИЗБЫТОЧНЫМ (job сам убивает),
/// но оставляем как defence-in-depth — если job не настроился (например,
/// process уже был breakaway).
pub async fn cleanup_orphan_post_processes() -> usize {
    // sysinfo не даёт env per-process на Windows out-of-the-box, поэтому
    // в 11B-1 — no-op stub. Полную реализацию через wmic / WMIC
    // оставим на 11B-2 (если Job Object не справится).
    0
}
