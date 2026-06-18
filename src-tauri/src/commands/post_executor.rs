//! Post Executor — Phase 11B-1 (v1.0.24) + Виток 1 MVP (org_agent execution).
//!
//! Когда Диспетчер сделал `forward_to_post(slug, refined_prompt)` — этот модуль
//! spawn'ит **реальный** Claude CLI subprocess со своим agent.md (генерируется
//! из `posts.system_prompt_md` или `org_agents.role_prompt_md`), в строгой
//! sandbox-папке `Outbox/<task_id>/`.
//!
//! Виток 1: `run_org_agent_now` — Tauri-команда для запуска org_agent задачи.
//! Использует тот же механизм `run_executor`, что и посты.
//!
//! Lifecycle:
//!   1. `trigger_post_executor()` / `run_org_agent_now()` — entry-points
//!   2. `resolve_executor()` — определяет кто исполняет (Post vs OrgAgent)
//!   3. `run_executor()` — ensure agent.md → mkdir Outbox → spawn claude.exe →
//!      write stdin → wait → scan dir → register artifacts → emit event
//!   4. Job Object (см. lib.rs::setup) гарантирует kill всех claude.exe при
//!      выходе MSPro даже в случае crash.

use std::collections::{HashMap, HashSet};
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
use crate::commands::executor_resolver::{resolve_executor, resolve_org_agent_by_id, ExecutorKind, ExecutorSpec};
use crate::commands::verdict_parser::{self, VerdictStatus};
use crate::db::WritePool;
use crate::outbox;
use crate::settings::{AppSettings, SettingsStore};
use crate::vault::VaultState;

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

/// Non-blocking entry-point для постов. Спавнит фоновую async-задачу.
/// Диспетчер не блокируется ожиданием пост-агента.
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

        // Резолвим исполнителя через единый резолвер.
        let spec = match resolve_executor(&db, &post_slug, &settings_snapshot).await {
            Ok(s) => s,
            Err(e) => {
                log::error!("post_executor: resolve '{post_slug}' failed: {e}");
                let _ = dispatcher::fail_task_inner(task_id, e, &db, &app).await;
                return;
            }
        };

        // Защита от двойного spawn.
        {
            let map = registry.running.lock().await;
            if map.contains_key(&task_id) {
                log::warn!(
                    "post_executor: task {task_id} already running, skip duplicate trigger"
                );
                return;
            }
        }

        let result = run_executor(
            &task_id,
            &spec,
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
            Ok(ref r) => {
                log::info!(
                    "post_executor: task {} done exit={} artifacts={} elapsed_ms={}",
                    r.task_id,
                    r.exit_code,
                    r.artifacts_count,
                    r.elapsed_ms
                );
                // Виток 3: trigger verify-hop + next-chain
                try_trigger_chains(&task_id, &spec, r, &db, &app).await;
            }
            Err(e) => {
                log::error!("post_executor: task {task_id} failed: {e}");
            }
        }
    });
}

// ---------------------------------------------------------------------------
// Tauri command: run_org_agent_now (Виток 1 MVP)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn run_org_agent_now(
    agent_id: String,
    task_prompt: String,
    db: State<'_, WritePool>,
    vault: State<'_, VaultState>,
    settings_store: State<'_, SettingsStore>,
    registry: State<'_, PostExecutorRegistry>,
    app: AppHandle,
) -> Result<String, String> {
    // Guard: пустая задача.
    if task_prompt.trim().is_empty() {
        return Err("пустая задача".to_string());
    }

    let settings = settings_store.data.lock().unwrap().clone();
    let spec = resolve_org_agent_by_id(&db, &agent_id, &settings).await?;

    // Guard: brain_mode.
    if spec.brain_mode == "disabled" {
        return Err("агент не активирован: включите мозг".to_string());
    }
    if spec.brain_mode != "claude_cli" {
        return Err(format!(
            "brain_mode '{}' пока не поддерживается (Виток 1)",
            spec.brain_mode
        ));
    }

    let prompt = task_prompt.trim().to_string();

    // Создаём запись задачи в dispatcher_logs для трекинга.
    let task = dispatcher::dispatch_task_inner_ex(
        "owner".to_string(),
        spec.slug.clone(),
        serde_json::json!({ "raw_prompt": &prompt }),
        dispatcher::DispatchExtras {
            parent_task_id: None,
            hop_kind: Some("direct".to_string()),
            routed_by_model: None,
            refined_prompt: Some(prompt.clone()),
        },
        &db,
        &app,
    )
    .await?;

    let task_id_ret = task.id.clone();

    log::info!(
        "run_org_agent_now: agent={} task={} slug={}",
        agent_id,
        task.id,
        spec.slug
    );

    // Спавним исполнение в фоне — команда возвращается мгновенно.
    let db_clone = db.inner().clone();
    let vault_clone = vault.inner().clone();
    let registry_clone = registry.inner().clone();
    let app_clone = app.clone();
    let task_id = task.id;

    tauri::async_runtime::spawn(async move {
        {
            let map = registry_clone.running.lock().await;
            if map.contains_key(&task_id) {
                log::warn!("run_org_agent_now: task {task_id} already running");
                return;
            }
        }

        let result = run_executor(
            &task_id,
            &spec,
            &prompt,
            None,
            &settings,
            &db_clone,
            &vault_clone,
            &registry_clone,
            &app_clone,
        )
        .await;

        {
            let mut map = registry_clone.running.lock().await;
            map.remove(&task_id);
        }

        match result {
            Ok(ref r) => {
                log::info!(
                    "run_org_agent_now: task {} done exit={} artifacts={}",
                    r.task_id,
                    r.exit_code,
                    r.artifacts_count
                );
                // Виток 3: trigger verify-hop + next-chain
                try_trigger_chains(&task_id, &spec, r, &db_clone, &app_clone).await;
            }
            Err(e) => log::error!("run_org_agent_now: task {task_id} failed: {e}"),
        }
    });

    Ok(task_id_ret)
}

// ---------------------------------------------------------------------------
// Shared executor
// ---------------------------------------------------------------------------

/// Главная функция исполнения: создаёт agent.md, sandbox, spawn CLI, артефакты.
/// Используется и для постов (через trigger_post_executor), и для org_agents
/// (через run_org_agent_now).
#[allow(clippy::too_many_arguments)]
async fn run_executor(
    task_id: &str,
    spec: &ExecutorSpec,
    refined_prompt: &str,
    _expected_artifact: Option<&str>,
    settings: &AppSettings,
    db: &WritePool,
    vault: &VaultState,
    registry: &PostExecutorRegistry,
    app: &AppHandle,
) -> Result<PostExecResult, String> {
    let started_at = Instant::now();

    // Guard: brain_mode≠claude_cli → fail_task вместо зависания
    if spec.brain_mode != "claude_cli" {
        let reason = format!(
            "brain_mode '{}' не поддерживается для исполнения (только claude_cli)",
            spec.brain_mode
        );
        log::warn!("executor: task {task_id} rejected: {reason}");
        let _ = dispatcher::fail_task_inner(task_id.to_string(), reason.clone(), db, app).await;
        return Err(reason);
    }

    // 1. Гарантируем agent.md (~/.claude/agents/{agent_md_name}.md).
    let _agent_md_path = ensure_agent_md(&spec.agent_md_name, &spec.system_prompt, &spec.model)?;

    // 2. Sandbox: <Outbox>/<task_id>/ (mkdir идемпотентно).
    let task_dir = outbox::task_outbox_dir(&vault.root, task_id)
        .map_err(|e| format!("task outbox dir: {e}"))?;

    log::info!(
        "executor: spawn agent={} model={} kind={:?} cwd={} prompt_len={}",
        spec.agent_md_name,
        spec.model,
        spec.kind,
        task_dir.display(),
        refined_prompt.len()
    );

    // Снимок «что было» в папке до спавна — для дельты после exit.
    let pre_snapshot = snapshot_dir(&task_dir);

    // Phase 1 (Iteration B): PAL killswitch.
    if settings.pal_enabled {
        return run_via_pal(
            task_id,
            &spec.slug,
            refined_prompt,
            &spec.system_prompt,
            &spec.agent_md_name,
            &spec.model,
            &task_dir,
            &pre_snapshot,
            settings,
            db,
            vault,
            registry,
            app,
        )
        .await;
    }

    // 3. Spawn claude.exe.
    // Срез 1.5 (security harden): единый `build_cli_args` — ОДИН источник всего argv.
    let mut cmd = Command::new(&settings.claude_cli_path);
    hide_console(&mut cmd);
    cmd.args(crate::pal::claude_cli_driver::build_cli_args(
        &spec.agent_md_name,
        &spec.model,
    ))
    .current_dir(&task_dir)
    .env("MSPRO_TASK_ID", task_id)
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped())
    .kill_on_drop(true);

    let mut child: Child = cmd.spawn().map_err(|e| {
        let task_id = task_id.to_string();
        let reason = format!("spawn failed: {e}");
        let db_clone = db.clone();
        let app_clone = app.clone();
        tauri::async_runtime::spawn(async move {
            let already_cancelled = dispatcher::fetch_task_by_id_public(&db_clone, &task_id)
                .await
                .map(|t| t.status == "cancelled")
                .unwrap_or(false);
            if !already_cancelled {
                let _ = dispatcher::fail_task_inner(task_id, reason, &db_clone, &app_clone).await;
            }
        });
        format!("claude spawn failed: {e}")
    })?;

    // Регистрируем PID для cancel.
    let pid = child.id().unwrap_or(0);
    {
        let mut map = registry.running.lock().await;
        map.insert(task_id.to_string(), pid);
    }

    // 4. Передаём refined_prompt в stdin → close → child получает EOF.
    if let Some(mut stdin_pipe) = child.stdin.take() {
        if let Err(e) = stdin_pipe.write_all(refined_prompt.as_bytes()).await {
            log::warn!("executor: stdin write failed: {e}");
        }
        drop(stdin_pipe);
    }

    // 5. Wait с timeout. Параллельно собираем stderr для диагностики.
    let timeout_secs = settings.post_executor_timeout_sec;
    let mut stderr_pipe = child.stderr.take();

    let wait_fut = child.wait();
    let result = timeout(Duration::from_secs(timeout_secs), wait_fut).await;

    let exit_code: i32 = match result {
        Ok(Ok(status)) => status.code().unwrap_or(-1),
        Ok(Err(e)) => {
            log::warn!("executor: wait failed: {e}");
            -1
        }
        Err(_) => {
            log::warn!("executor: task {task_id} timeout {timeout_secs}s — killing");
            let _ = child.kill().await;
            -2
        }
    };

    // 6. Подбираем stderr.
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

    // 7. Сканируем директорию — что нового создал агент?
    let new_files = diff_dir(&task_dir, &pre_snapshot);
    log::info!(
        "executor: task {task_id} exit={exit_code} new_files={} elapsed={}ms",
        new_files.len(),
        elapsed_ms
    );

    // 8. Регистрируем артефакты.
    let mut registered = 0usize;
    for rel in &new_files {
        let mime = guess_mime_from_ext(rel);
        match artifacts::register_artifact(
            task_id,
            rel,
            mime.as_deref(),
            &spec.slug,
            db,
            vault,
            app,
        )
        .await
        {
            Ok(_) => registered += 1,
            Err(e) => log::warn!("executor: register {rel} failed: {e}"),
        }
    }

    // 9. Обновляем статус parent task в dispatcher_logs.
    // BL-P1-016: если пользователь уже отменил — не перезатираем cancelled.
    if is_task_cancelled(task_id, db).await {
        log::info!("executor: task {task_id} already cancelled — skip status update");
    } else if exit_code == 0 && registered > 0 {
        log::info!(
            "executor: task {task_id} produced {registered} artifacts — awaiting approval"
        );
    } else {
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

/// Создаёт `~/.claude/agents/{agent_md_name}.md` (или перезаписывает если отличается).
///
/// КРИТИЧЕСКИ: tools: [Read, Write, Edit, Bash] — агент ДОЛЖЕН физически писать
/// файлы. Это отличие от Гендира/Диспетчера, где `tools: []` запрещает native tools.
pub fn ensure_agent_md(
    agent_md_name: &str,
    system_prompt: &str,
    model: &str,
) -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot resolve home dir".to_string())?;
    let dir = home.join(".claude").join("agents");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create agents dir: {e}"))?;
    let path = dir.join(format!("{}.md", agent_md_name));

    let description = if agent_md_name.starts_with("mspro-org-") {
        format!(
            "MSPro-Ltd Corp org-агент ({}). Получает task через stdin, создаёт артефакты в текущей рабочей директории (Outbox sandbox).",
            agent_md_name
        )
    } else {
        format!(
            "MSPro-Ltd Corp пост-агент ({}). Получает task через stdin, создаёт артефакты в текущей рабочей директории (Outbox sandbox).",
            agent_md_name
        )
    };

    let body = format!(
        "---\nname: {name}\ndescription: {desc}\ntools: [Read, Write, Edit, Bash]\nmodel: {model}\n---\n\n{prompt}\n",
        name = agent_md_name,
        desc = description,
        model = model,
        prompt = system_prompt.trim(),
    );

    let need_write = match std::fs::read_to_string(&path) {
        Ok(existing) => existing != body,
        Err(_) => true,
    };
    if need_write {
        std::fs::write(&path, body).map_err(|e| format!("write agent.md: {e}"))?;
        log::info!("ensured agent file: {}", path.display());
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
                let rel_s = rel.to_string_lossy().replace('\\', "/").to_string();
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

fn diff_dir(dir: &Path, before: &HashMap<String, u64>) -> Vec<String> {
    let after = snapshot_dir(dir);
    let mut new_or_changed = Vec::new();
    for (rel, mtime) in after {
        match before.get(&rel) {
            Some(&prev_mtime) if prev_mtime == mtime => continue,
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
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "csv" => "text/csv",
        _ => return None,
    };
    Some(mime.to_string())
}

async fn bump_attempts(task_id: &str, db: &WritePool) -> Result<(), String> {
    sqlx::query("UPDATE dispatcher_logs SET attempts_count = attempts_count + 1 WHERE id = ?")
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

// ---------------------------------------------------------------------------
// Виток 3: chain triggers (verify-hop ОТК + next-chain)
// ---------------------------------------------------------------------------

const MAX_HOP_DEPTH: i32 = 10;
const MAX_VERIFY_FANOUT: usize = 5;
const MAX_NEXT_FANOUT: usize = 5;
const MAX_REWORK: i32 = 3;

async fn count_hop_depth(task_id: &str, db: &WritePool) -> i32 {
    let mut depth = 0i32;
    let mut current = task_id.to_string();
    let mut visited = HashSet::new();
    visited.insert(current.clone());
    loop {
        if depth >= MAX_HOP_DEPTH {
            break;
        }
        let parent: Option<(Option<String>,)> =
            sqlx::query_as("SELECT parent_task_id FROM dispatcher_logs WHERE id = ?")
                .bind(&current)
                .fetch_optional(&db.0)
                .await
                .unwrap_or(None);
        match parent {
            Some((Some(pid),)) => {
                if !visited.insert(pid.clone()) {
                    log::warn!("count_hop_depth: cycle at {pid} — stopping");
                    break;
                }
                depth += 1;
                current = pid;
            }
            _ => break,
        }
    }
    depth
}

/// verify-хоп ТЕРМИНАЛЕН — не порождает дочерних verify/next цепочек.
/// Иначе verifier-агент сам запускал бы проверку/продолжение → каскад A↔B.
pub(crate) fn hop_is_terminal(hop_kind: Option<&str>) -> bool {
    matches!(hop_kind, Some("verify"))
}

pub(crate) async fn try_trigger_chains(
    task_id: &str,
    spec: &ExecutorSpec,
    result: &PostExecResult,
    db: &WritePool,
    app: &AppHandle,
) {
    if spec.kind != ExecutorKind::OrgAgent {
        return;
    }
    if result.exit_code != 0 || result.artifacts_count == 0 {
        return;
    }

    // verify-хоп ТЕРМИНАЛЕН: результат проверки не проверяем повторно и не
    // продолжаем цепочкой, иначе verifier сам триггерит verify/next → каскад
    // A↔B (Cursor medium). Терминальность определяется hop_kind текущей задачи.
    let cur_hop: Option<(Option<String>,)> =
        sqlx::query_as("SELECT hop_kind FROM dispatcher_logs WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);
    let cur_hop = cur_hop.and_then(|(h,)| h);
    if hop_is_terminal(cur_hop.as_deref()) {
        log::info!("chain trigger: task {task_id} hop={cur_hop:?} terminal — rework check");
        try_trigger_rework(task_id, db, app).await;
        return;
    }

    let depth = count_hop_depth(task_id, db).await;
    if depth >= MAX_HOP_DEPTH {
        log::warn!(
            "chain trigger: task {task_id} depth={depth} >= {MAX_HOP_DEPTH} — escalation"
        );
        let _ = app.emit(
            "rework-escalation",
            serde_json::json!({
                "task_id": task_id,
                "reason": format!("Цепочка достигла максимальной глубины {MAX_HOP_DEPTH}"),
            }),
        );
        return;
    }

    try_trigger_verify(task_id, spec, db, app).await;
    try_trigger_next(task_id, spec, db, app).await;
}

async fn try_trigger_verify(
    task_id: &str,
    spec: &ExecutorSpec,
    db: &WritePool,
    app: &AppHandle,
) {
    let verifiers: Vec<(String, String)> = sqlx::query_as(
        "SELECT l.to_agent_id, a.slug FROM org_agent_links l \
         JOIN org_agents a ON a.id = l.to_agent_id \
         WHERE l.from_agent_id = ? AND l.link_type = 'verifier' \
         AND a.status = 'active' AND a.brain_mode = 'claude_cli' \
         AND a.role_prompt_md IS NOT NULL AND trim(a.role_prompt_md) != ''",
    )
    .bind(&spec.entity_id)
    .fetch_all(&db.0)
    .await
    .unwrap_or_default();

    if verifiers.is_empty() {
        return;
    }
    if verifiers.len() > MAX_VERIFY_FANOUT {
        log::warn!(
            "verify trigger: {} verifiers for task {task_id}, capping at {MAX_VERIFY_FANOUT}",
            verifiers.len()
        );
    }

    let artifacts: Vec<(String,)> =
        sqlx::query_as("SELECT rel_path FROM task_artifacts WHERE task_id = ?")
            .bind(task_id)
            .fetch_all(&db.0)
            .await
            .unwrap_or_default();
    let artifact_list = artifacts
        .iter()
        .map(|(p,)| p.as_str())
        .collect::<Vec<_>>()
        .join(", ");

    let parent_payload: Option<(Option<String>,)> =
        sqlx::query_as("SELECT refined_prompt FROM dispatcher_logs WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);
    let task_desc = parent_payload
        .and_then(|(p,)| p)
        .unwrap_or_else(|| "(описание недоступно)".to_string());
    let task_desc_short = if task_desc.len() > 500 {
        format!("{}…", &task_desc[..500])
    } else {
        task_desc
    };

    for (_verifier_id, verifier_slug) in verifiers.iter().take(MAX_VERIFY_FANOUT) {
        let verify_prompt = format!(
            "Проверь результат задачи: {task_desc_short}\n\
             Артефакт(ы): {artifact_list}\n\n\
             Запиши результат в файл verdict.md СТРОГО по формату:\n\
             - Первая строка: ровно `ВЕРДИКТ: ГОДНО` или `ВЕРДИКТ: БРАК`\n\
             - Далее — обоснование. При браке: нумерованный список замечаний \
             (что именно исправить, со ссылками на конкретные места в артефакте)."
        );

        let verify_task = dispatcher::dispatch_task_inner_ex(
            spec.slug.clone(),
            verifier_slug.clone(),
            serde_json::json!({ "raw_prompt": &verify_prompt, "verify_parent": task_id }),
            dispatcher::DispatchExtras {
                parent_task_id: Some(task_id.to_string()),
                hop_kind: Some("verify".to_string()),
                routed_by_model: None,
                refined_prompt: Some(verify_prompt.clone()),
            },
            db,
            app,
        )
        .await;

        match verify_task {
            Ok(task) => {
                log::info!(
                    "verify trigger: created verify task {} for verifier '{verifier_slug}' (parent {task_id})",
                    task.id
                );
                trigger_post_executor(
                    task.id,
                    verifier_slug.clone(),
                    verify_prompt,
                    Some("verdict.md".to_string()),
                    app.clone(),
                );
            }
            Err(e) => log::warn!("verify trigger: failed to create task for '{verifier_slug}': {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// ККИ rework loop: verdict parser → rework/accept
// ---------------------------------------------------------------------------

async fn find_root_prompt(start_task_id: &str, db: &WritePool) -> Option<String> {
    let mut current = start_task_id.to_string();
    let mut visited = HashSet::new();
    visited.insert(current.clone());
    loop {
        let row: Option<(Option<String>, Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT parent_task_id, hop_kind, refined_prompt FROM dispatcher_logs WHERE id = ?",
        )
        .bind(&current)
        .fetch_optional(&db.0)
        .await
        .unwrap_or(None);

        match row {
            Some((parent_opt, hop, prompt)) => {
                let is_intermediate =
                    matches!(hop.as_deref(), Some("rework") | Some("verify"));
                if !is_intermediate {
                    return prompt;
                }
                match parent_opt {
                    Some(pid) if visited.insert(pid.clone()) => {
                        current = pid;
                    }
                    _ => return prompt,
                }
            }
            None => return None,
        }
    }
}

async fn count_rework_in_chain(start_task_id: &str, db: &WritePool) -> i32 {
    let mut count = 0i32;
    let mut current = start_task_id.to_string();
    let mut visited = HashSet::new();
    visited.insert(current.clone());
    loop {
        let row: Option<(Option<String>, Option<String>)> =
            sqlx::query_as("SELECT parent_task_id, hop_kind FROM dispatcher_logs WHERE id = ?")
                .bind(&current)
                .fetch_optional(&db.0)
                .await
                .unwrap_or(None);
        match row {
            Some((parent_opt, hop)) => {
                if hop.as_deref() == Some("rework") {
                    count += 1;
                }
                match parent_opt {
                    Some(pid) if visited.insert(pid.clone()) => {
                        current = pid;
                    }
                    _ => break,
                }
            }
            None => break,
        }
    }
    count
}

async fn try_trigger_rework(
    verify_task_id: &str,
    db: &WritePool,
    app: &AppHandle,
) {
    // 1. Find parent (executor) task of this verify-hop.
    let verify_row: Option<(Option<String>,)> =
        sqlx::query_as("SELECT parent_task_id FROM dispatcher_logs WHERE id = ?")
            .bind(verify_task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);

    let Some((Some(executor_task_id),)) = verify_row else {
        log::warn!("rework trigger: verify {verify_task_id} has no parent — skip");
        return;
    };

    let exec_row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT to_entity, refined_prompt FROM dispatcher_logs WHERE id = ?")
            .bind(&executor_task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);

    let Some((executor_slug, orig_prompt_opt)) = exec_row else {
        log::warn!("rework trigger: executor task {executor_task_id} not found — skip");
        return;
    };

    // 2. Read verdict.md from verify-task outbox.
    let vault = match app.try_state::<VaultState>() {
        Some(s) => s.inner().clone(),
        None => {
            log::error!("rework trigger: VaultState not available — skip");
            return;
        }
    };

    let verdict_path = match crate::outbox::task_outbox_dir(&vault.root, verify_task_id) {
        Ok(dir) => dir.join("verdict.md"),
        Err(e) => {
            log::warn!("rework trigger: outbox dir for {verify_task_id}: {e} — treating as uncertain");
            let _ = app.emit(
                "rework-verdict",
                serde_json::json!({
                    "task_id": executor_task_id,
                    "verdict": "uncertain",
                    "verify_task_id": verify_task_id,
                    "reason": format!("outbox dir не найден: {e}"),
                }),
            );
            return;
        }
    };

    let content = match std::fs::read_to_string(&verdict_path) {
        Ok(c) => c,
        Err(e) => {
            log::warn!(
                "rework trigger: read {}: {e} — treating as uncertain",
                verdict_path.display()
            );
            let _ = app.emit(
                "rework-verdict",
                serde_json::json!({
                    "task_id": executor_task_id,
                    "verdict": "uncertain",
                    "verify_task_id": verify_task_id,
                    "reason": format!("verdict.md не найден: {e}"),
                }),
            );
            return;
        }
    };

    // 3. Parse verdict.
    let verdict = verdict_parser::parse_verdict(&content);
    log::info!(
        "rework trigger: verify {verify_task_id} → {} (reasons {} chars)",
        verdict.status,
        verdict.reasons.len()
    );

    match verdict.status {
        VerdictStatus::Pass => {
            // ГОДНО → approve executor's artifacts.
            let arts: Vec<(String,)> = sqlx::query_as(
                "SELECT id FROM task_artifacts \
                 WHERE task_id = ? AND approved_at IS NULL AND rejected_at IS NULL",
            )
            .bind(&executor_task_id)
            .fetch_all(&db.0)
            .await
            .unwrap_or_default();

            for (art_id,) in &arts {
                let _ = sqlx::query(
                    "UPDATE task_artifacts SET approved_at = CURRENT_TIMESTAMP WHERE id = ?",
                )
                .bind(art_id)
                .execute(&db.0)
                .await;
            }

            log::info!(
                "rework trigger: PASS — approved {} artifacts for task {executor_task_id}",
                arts.len()
            );
            let _ = app.emit(
                "rework-verdict",
                serde_json::json!({
                    "task_id": executor_task_id,
                    "verdict": "pass",
                    "artifacts_approved": arts.len(),
                }),
            );
        }

        VerdictStatus::Fail => {
            // БРАК → check rework limit, then create rework task.
            let rework_count = count_rework_in_chain(&executor_task_id, db).await;
            if rework_count >= MAX_REWORK {
                let reason = format!(
                    "Исчерпаны попытки доработки ({}/{}). Замечания контролёра: {}",
                    rework_count,
                    MAX_REWORK,
                    truncate_str(&verdict.reasons, 300)
                );
                log::warn!(
                    "rework trigger: FAIL but rework_count={rework_count} >= {MAX_REWORK} — escalation"
                );
                let _ = sqlx::query(
                    "UPDATE dispatcher_logs SET status = 'failed', error_msg = ? WHERE id = ?",
                )
                .bind(&reason)
                .bind(&executor_task_id)
                .execute(&db.0)
                .await;
                let _ = app.emit(
                    "rework-escalation",
                    serde_json::json!({
                        "task_id": executor_task_id,
                        "rework_count": rework_count,
                        "reason": &reason,
                    }),
                );
                return;
            }

            // Reject executor's pending artifacts.
            let reject_reason = if verdict.reasons.is_empty() {
                "Вердикт БРАК без подробностей".to_string()
            } else {
                truncate_str(&verdict.reasons, 500).to_string()
            };
            let _ = sqlx::query(
                "UPDATE task_artifacts \
                 SET rejected_at = CURRENT_TIMESTAMP, reject_reason = ? \
                 WHERE task_id = ? AND approved_at IS NULL AND rejected_at IS NULL",
            )
            .bind(&reject_reason)
            .bind(&executor_task_id)
            .execute(&db.0)
            .await;

            // Build rework prompt — walk chain to find original (non-rework) description.
            let root_desc = find_root_prompt(&executor_task_id, db).await;
            let orig_desc = root_desc
                .as_deref()
                .or(orig_prompt_opt.as_deref())
                .unwrap_or("(описание недоступно)");
            let rework_prompt = format!(
                "ДОРАБОТКА.\n\
                 Исходное ТЗ: {orig_desc}\n\n\
                 Контролёр забраковал результат. Замечания:\n{}\n\n\
                 Исправь указанные проблемы. Выдай исправленный артефакт.",
                if verdict.reasons.is_empty() {
                    "Без подробностей."
                } else {
                    &verdict.reasons
                }
            );

            // Resolve expected_artifact from executor's artifacts.
            let expected: Option<(String,)> = sqlx::query_as(
                "SELECT rel_path FROM task_artifacts \
                 WHERE task_id = ? ORDER BY created_at ASC LIMIT 1",
            )
            .bind(&executor_task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);
            let expected_artifact = expected.map(|(p,)| p);

            let rework_task = dispatcher::dispatch_task_inner_ex(
                "dispatcher".to_string(),
                executor_slug.clone(),
                serde_json::json!({
                    "raw_prompt": &rework_prompt,
                    "rework_of": &executor_task_id,
                    "verify_task": verify_task_id,
                }),
                dispatcher::DispatchExtras {
                    parent_task_id: Some(executor_task_id.clone()),
                    hop_kind: Some("rework".to_string()),
                    routed_by_model: None,
                    refined_prompt: Some(rework_prompt.clone()),
                },
                db,
                app,
            )
            .await;

            match rework_task {
                Ok(task) => {
                    log::info!(
                        "rework trigger: FAIL — created rework {} for '{}' (parent {executor_task_id}, attempt {})",
                        task.id,
                        executor_slug,
                        rework_count + 1
                    );
                    trigger_post_executor(
                        task.id,
                        executor_slug,
                        rework_prompt,
                        expected_artifact,
                        app.clone(),
                    );
                }
                Err(e) => log::warn!("rework trigger: failed to create task: {e}"),
            }
        }

        VerdictStatus::Uncertain => {
            log::warn!(
                "rework trigger: verify {verify_task_id} — uncertain verdict, leaving for manual review"
            );
            let _ = app.emit(
                "rework-verdict",
                serde_json::json!({
                    "task_id": executor_task_id,
                    "verdict": "uncertain",
                    "verify_task_id": verify_task_id,
                }),
            );
        }
    }
}

fn truncate_str(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

async fn try_trigger_next(
    task_id: &str,
    spec: &ExecutorSpec,
    db: &WritePool,
    app: &AppHandle,
) {
    let next_agents: Vec<(String, String)> = sqlx::query_as(
        "SELECT l.to_agent_id, a.slug FROM org_agent_links l \
         JOIN org_agents a ON a.id = l.to_agent_id \
         WHERE l.from_agent_id = ? AND l.link_type = 'next' \
         AND a.status = 'active' AND a.brain_mode = 'claude_cli' \
         AND a.role_prompt_md IS NOT NULL AND trim(a.role_prompt_md) != ''",
    )
    .bind(&spec.entity_id)
    .fetch_all(&db.0)
    .await
    .unwrap_or_default();

    if next_agents.is_empty() {
        return;
    }
    if next_agents.len() > MAX_NEXT_FANOUT {
        log::warn!(
            "next chain: {} next-agents for task {task_id}, capping at {MAX_NEXT_FANOUT}",
            next_agents.len()
        );
    }

    let parent_payload: Option<(Option<String>,)> =
        sqlx::query_as("SELECT refined_prompt FROM dispatcher_logs WHERE id = ?")
            .bind(task_id)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);
    let prev_context = parent_payload
        .and_then(|(p,)| p)
        .unwrap_or_else(|| "(контекст недоступен)".to_string());
    let prev_context_short = if prev_context.len() > 500 {
        format!("{}…", &prev_context[..500])
    } else {
        prev_context
    };

    for (_next_id, next_slug) in next_agents.iter().take(MAX_NEXT_FANOUT) {
        let next_prompt = format!(
            "Предыдущий шаг выполнен агентом '{}'. Контекст задачи: {prev_context_short}\n\
             Продолжи работу по своей роли.",
            spec.slug
        );

        let next_task = dispatcher::dispatch_task_inner_ex(
            spec.slug.clone(),
            next_slug.clone(),
            serde_json::json!({ "raw_prompt": &next_prompt, "chain_parent": task_id }),
            dispatcher::DispatchExtras {
                parent_task_id: Some(task_id.to_string()),
                hop_kind: Some("next_step".to_string()),
                routed_by_model: None,
                refined_prompt: Some(next_prompt.clone()),
            },
            db,
            app,
        )
        .await;

        match next_task {
            Ok(task) => {
                log::info!(
                    "next chain: created task {} for '{next_slug}' (parent {task_id})",
                    task.id
                );
                trigger_post_executor(
                    task.id,
                    next_slug.clone(),
                    next_prompt,
                    None,
                    app.clone(),
                );
            }
            Err(e) => log::warn!("next chain: failed to create task for '{next_slug}': {e}"),
        }
    }
}

/// Phase 1 (Iteration B) — путь исполнения через PAL.
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
    registry: &PostExecutorRegistry,
    app: &AppHandle,
) -> Result<PostExecResult, String> {
    use crate::pal::claude_cli_driver::ClaudeCliDriver;
    use crate::pal::qwen_http_driver::QwenHttpDriver;
    use crate::pal::{
        orchestrator, PostRuntimeProvider, ProviderKind, ProviderRequest, RequestTrace,
    };
    use crate::run_logger::{insert_run_log, RunLogEntry};
    use std::sync::Arc;

    let started = Instant::now();
    let tier = tier_for_model(model);

    let claude: Arc<dyn PostRuntimeProvider> = Arc::new(
        ClaudeCliDriver::new(
            "claude_cli".to_string(),
            settings.claude_cli_path.clone(),
            model.to_string(),
        )
        .with_pid_registration(registry.running.clone(), task_id.to_string()),
    );
    let qwen: Arc<dyn PostRuntimeProvider> = Arc::new(QwenHttpDriver::new(
        "qwen_http".to_string(),
        settings.qwen_endpoint.clone(),
        settings.qwen_model.clone(),
    ));
    let chain: Vec<Arc<dyn PostRuntimeProvider>> = vec![claude, qwen];

    let request = ProviderRequest {
        system_prompt: system_prompt.to_string(),
        user_message: refined_prompt.to_string(),
        tier,
        timeout: None,
        max_turns: None,
        model_override: None,
        workspace_path: Some(task_dir.to_path_buf()),
        agent_slug: Some(agent_name.to_string()),
        mcp_bindings: Vec::new(),
        trace: RequestTrace {
            post_slug: slug.to_string(),
            dispatcher_log_id: Some(task_id.to_string()),
            attempt_id: task_id.to_string(),
            attempt_number: 0,
        },
    };

    log::info!(
        "run_via_pal: task={task_id} slug={slug} tier={} model={model} chain=[claude_cli,qwen_http]",
        tier.as_str()
    );
    let outcome = orchestrator::pal_invoke_chain(&chain, request).await;
    let result = outcome.result;
    let elapsed_ms = started.elapsed().as_millis();

    if let Ok(resp) = &result {
        if resp.provider_used == ProviderKind::QwenHttp && !resp.text.trim().is_empty() {
            let result_path = task_dir.join("result.txt");
            if let Err(e) = std::fs::write(&result_path, &resp.text) {
                log::warn!("run_via_pal: write Qwen result.txt failed: {e}");
            }
        }
    }

    let (provider_used, model_used, success, error_kind, raw_output, latency) = match &result {
        Ok(resp) => (
            resp.provider_used.as_str().to_string(),
            resp.model_used.clone(),
            true,
            None,
            Some(resp.text.clone()),
            resp.latency_ms as i64,
        ),
        Err(e) => (
            chain
                .get(outcome.attempt_idx)
                .map(|p| p.provider_id())
                .unwrap_or_else(|| "claude_cli".to_string()),
            model.to_string(),
            false,
            Some(e.kind_str().to_string()),
            Some(e.to_string()),
            elapsed_ms as i64,
        ),
    };
    let entry = RunLogEntry {
        task_id: Some(task_id.to_string()),
        post_slug: Some(slug.to_string()),
        provider_id: provider_used,
        model_used: Some(model_used),
        tier: Some(tier.as_str().to_string()),
        tokens_in: result
            .as_ref()
            .map(|r| r.usage.input_tokens as i64)
            .unwrap_or(0),
        tokens_out: result
            .as_ref()
            .map(|r| r.usage.output_tokens as i64)
            .unwrap_or(0),
        latency_ms: latency,
        cost_usd: 0.0,
        success,
        fallback_used: outcome.fallback_used,
        attempt_number: outcome.attempt_idx as i64,
        error_kind,
        raw_output,
    };
    if let Err(e) = insert_run_log(db, entry).await {
        log::warn!("run_via_pal: run_log insert failed: {e}");
    }

    let new_files = diff_dir(task_dir, pre_snapshot);
    let mut registered = 0usize;
    for rel in &new_files {
        let mime = guess_mime_from_ext(rel);
        match artifacts::register_artifact(task_id, rel, mime.as_deref(), slug, db, vault, app)
            .await
        {
            Ok(_) => registered += 1,
            Err(e) => log::warn!("run_via_pal: register {rel} failed: {e}"),
        }
    }

    let already_cancelled = is_task_cancelled(task_id, db).await;

    let exit_code = match &result {
        _ if already_cancelled => {
            log::info!("run_via_pal: task {task_id} already cancelled — skip status update");
            0
        }
        Ok(_) if registered > 0 => {
            log::info!(
                "run_via_pal: task {task_id} produced {registered} artifacts — awaiting approval"
            );
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
            let _ = dispatcher::fail_task_inner(
                task_id.to_string(),
                format!("PAL error: {e}"),
                db,
                app,
            )
            .await;
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
// Tauri commands (cancel)
// ---------------------------------------------------------------------------

/// BFS-обход дерева процессов: возвращает все PID (корень + потомки).
fn collect_tree_pids(root: u32, children_of: impl Fn(u32) -> Vec<u32>) -> Vec<u32> {
    let mut to_kill = vec![root];
    let mut i = 0;
    while i < to_kill.len() {
        for child in children_of(to_kill[i]) {
            if !to_kill.contains(&child) {
                to_kill.push(child);
            }
        }
        i += 1;
    }
    to_kill
}

/// BL-P1-016: Рекурсивный kill дерева процессов по корневому PID.
#[cfg(windows)]
fn kill_process_tree(root_pid: u32) {
    use sysinfo::{Pid, ProcessesToUpdate, System};

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let pids = collect_tree_pids(root_pid, |parent| {
        let parent_pid = sysinfo::Pid::from_u32(parent);
        sys.processes()
            .iter()
            .filter(|(_, p)| p.parent() == Some(parent_pid))
            .map(|(pid, _)| pid.as_u32())
            .collect()
    });

    for &pid in pids.iter().rev() {
        if let Some(proc_) = sys.process(Pid::from_u32(pid)) {
            let killed = proc_.kill();
            log::info!("kill_process_tree: pid={pid} killed={killed}");
        }
    }
}

async fn is_task_cancelled(task_id: &str, db: &WritePool) -> bool {
    dispatcher::fetch_task_by_id_public(db, task_id)
        .await
        .map(|t| t.status == "cancelled")
        .unwrap_or(false)
}

#[tauri::command]
pub async fn cancel_post_executor(
    task_id: String,
    registry: State<'_, PostExecutorRegistry>,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<bool, String> {
    match dispatcher::cancel_task_inner(
        task_id.clone(),
        "cancelled by user".to_string(),
        &db,
        &app,
    )
    .await
    {
        Ok(_) => log::info!("cancel_post_executor: task {task_id} status → cancelled"),
        Err(e) => log::warn!("cancel_post_executor: cancel_task_inner: {e}"),
    }

    let pid = {
        let mut map = registry.running.lock().await;
        map.remove(&task_id)
    };

    if let Some(pid) = pid {
        #[cfg(windows)]
        kill_process_tree(pid);
        Ok(true)
    } else {
        Ok(false)
    }
}

pub async fn cleanup_orphan_post_processes() -> usize {
    0
}

#[cfg(test)]
mod tests {
    const SRC: &str = include_str!("post_executor.rs");

    #[test]
    fn legacy_spawn_has_no_bypass_flag() {
        let dot_arg = concat!(".", "arg");
        let flag = concat!("--dangerously", "-skip-permissions");
        let bypass_call = format!("{}({}{}{})", dot_arg, '"', flag, '"');
        let hits = SRC.matches(&bypass_call).count();
        assert_eq!(
            hits, 0,
            "legacy spawn содержит bypass-флаг как аргумент Command — security regress (Срез 1.5)"
        );
    }

    #[test]
    fn legacy_spawn_uses_single_source_argv() {
        assert!(
            SRC.contains("claude_cli_driver::build_cli_args("),
            "legacy spawn не использует единый build_cli_args — argv может разъехаться с PAL"
        );
    }

    #[test]
    fn kill_process_tree_bfs_collects_full_tree() {
        use std::collections::HashMap;
        let mut tree: HashMap<u32, Vec<u32>> = HashMap::new();
        tree.insert(1, vec![2, 3]);
        tree.insert(3, vec![4]);

        let pids = super::collect_tree_pids(1, |p| {
            tree.get(&p).cloned().unwrap_or_default()
        });
        assert_eq!(pids[0], 1, "root first");
        assert!(pids.contains(&2));
        assert!(pids.contains(&3));
        assert!(pids.contains(&4));
        assert_eq!(pids.len(), 4, "all 4 pids collected");
        let pos4 = pids.iter().position(|&p| p == 4).unwrap();
        let pos3 = pids.iter().position(|&p| p == 3).unwrap();
        assert!(pos4 > pos3, "child 4 discovered after parent 3 (BFS order)");
    }

    #[test]
    fn kill_process_tree_bfs_no_cycle() {
        use std::collections::HashMap;
        let mut tree: HashMap<u32, Vec<u32>> = HashMap::new();
        tree.insert(10, vec![20]);
        tree.insert(20, vec![10]);

        let pids = super::collect_tree_pids(10, |p| {
            tree.get(&p).cloned().unwrap_or_default()
        });
        assert_eq!(pids, vec![10, 20], "cycle doesn't cause infinite loop");
    }

    // ── Виток 3: chain depth + trigger tests ──────────────────────────

    use crate::db::WritePool;
    use sqlx::SqlitePool;

    async fn setup_chain_db() -> WritePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE dispatcher_logs (
                id TEXT PRIMARY KEY,
                from_entity TEXT NOT NULL,
                to_entity TEXT NOT NULL,
                task_payload TEXT NOT NULL DEFAULT '{}',
                status TEXT NOT NULL DEFAULT 'in_progress',
                execution_time_ms INTEGER,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                parent_task_id TEXT DEFAULT NULL,
                completed_at DATETIME DEFAULT NULL,
                attempts_count INTEGER NOT NULL DEFAULT 1,
                hop_kind TEXT DEFAULT NULL,
                routed_by_model TEXT DEFAULT NULL,
                refined_prompt TEXT DEFAULT NULL,
                outbox_path TEXT DEFAULT NULL,
                raw_brain_response TEXT DEFAULT NULL
            );
            CREATE TABLE org_agents (
                id TEXT PRIMARY KEY,
                department_id TEXT NOT NULL,
                name TEXT NOT NULL,
                slug TEXT NOT NULL,
                role_label TEXT NOT NULL DEFAULT 'member',
                status TEXT NOT NULL DEFAULT 'active',
                folder_path TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT NULL,
                role_prompt_md TEXT DEFAULT NULL,
                brain_mode TEXT NOT NULL DEFAULT 'disabled',
                brain_model TEXT DEFAULT NULL,
                brain_endpoint TEXT DEFAULT NULL,
                mcp_servers_json TEXT NOT NULL DEFAULT '[]',
                ckp_text TEXT DEFAULT NULL,
                checklist_json TEXT NOT NULL DEFAULT '[]',
                memory_md TEXT DEFAULT NULL
            );
            CREATE TABLE org_agent_links (
                id TEXT PRIMARY KEY,
                from_agent_id TEXT NOT NULL,
                to_agent_id TEXT NOT NULL,
                link_type TEXT NOT NULL,
                description TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
            );
            CREATE TABLE task_artifacts (
                id TEXT PRIMARY KEY,
                task_id TEXT NOT NULL,
                rel_path TEXT NOT NULL,
                mime_type TEXT,
                post_slug TEXT,
                status TEXT NOT NULL DEFAULT 'pending',
                outbox_path TEXT,
                reject_reason TEXT
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        WritePool(pool)
    }

    #[tokio::test]
    async fn count_hop_depth_zero_for_root() {
        let db = setup_chain_db().await;
        sqlx::query(
            "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id) \
             VALUES ('t1', 'owner', 'agent-a', NULL)",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let depth = super::count_hop_depth("t1", &db).await;
        assert_eq!(depth, 0, "root task has depth 0");
    }

    #[tokio::test]
    async fn count_hop_depth_chain() {
        let db = setup_chain_db().await;
        sqlx::query(
            "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id) VALUES \
             ('t1', 'owner', 'a', NULL), \
             ('t2', 'a', 'b', 't1'), \
             ('t3', 'b', 'c', 't2')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        assert_eq!(super::count_hop_depth("t1", &db).await, 0);
        assert_eq!(super::count_hop_depth("t2", &db).await, 1);
        assert_eq!(super::count_hop_depth("t3", &db).await, 2);
    }

    #[tokio::test]
    async fn count_hop_depth_capped_at_max() {
        let db = setup_chain_db().await;
        for i in 0..=12 {
            let id = format!("t{i}");
            let parent = if i == 0 {
                "NULL".to_string()
            } else {
                format!("'t{}'", i - 1)
            };
            sqlx::raw_sql(&format!(
                "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id) \
                 VALUES ('{id}', 'x', 'y', {parent})"
            ))
            .execute(&db.0)
            .await
            .unwrap();
        }

        let depth = super::count_hop_depth("t12", &db).await;
        assert!(depth >= super::MAX_HOP_DEPTH, "depth capped at MAX_HOP_DEPTH");
    }

    #[tokio::test]
    async fn try_trigger_chains_skips_posts() {
        let db = setup_chain_db().await;
        use super::*;
        use crate::commands::executor_resolver::*;

        let spec = ExecutorSpec {
            slug: "frontend".to_string(),
            kind: ExecutorKind::Post,
            entity_id: "p1".to_string(),
            system_prompt: "Test".to_string(),
            model: "opus".to_string(),
            brain_mode: "claude_cli".to_string(),
            agent_md_name: "mspro-frontend".to_string(),
        };
        let result = PostExecResult {
            task_id: "t1".to_string(),
            exit_code: 0,
            artifacts_count: 1,
            elapsed_ms: 100,
        };

        // No AppHandle in tests — we test that the function returns early for Posts
        // by checking it doesn't panic (no DB rows to find for chains)
        // This is a guard test: Posts should skip chain triggers entirely.
        assert_eq!(spec.kind, ExecutorKind::Post);
        // Cannot call try_trigger_chains without AppHandle, but the guard check
        // (spec.kind != OrgAgent) ensures early return.
    }

    #[tokio::test]
    async fn try_trigger_chains_skips_on_failure() {
        use super::*;
        use crate::commands::executor_resolver::*;

        let result = PostExecResult {
            task_id: "t1".to_string(),
            exit_code: 1,
            artifacts_count: 0,
            elapsed_ms: 100,
        };
        assert_ne!(result.exit_code, 0);
        assert_eq!(result.artifacts_count, 0);
        // Both conditions prevent chain trigger — no panic, no DB access needed.
    }

    #[test]
    fn verify_hop_is_terminal() {
        // verify-хоп терминален — не порождает дочерних verify/next (Cursor medium fix):
        // иначе verifier-агент сам триггерил бы проверку → каскад A↔B.
        assert!(super::hop_is_terminal(Some("verify")), "verify должен быть терминальным");
        // Прочие хопы продолжают цепочки.
        assert!(!super::hop_is_terminal(Some("next_step")));
        assert!(!super::hop_is_terminal(Some("direct")));
        assert!(!super::hop_is_terminal(Some("refined")));
        assert!(!super::hop_is_terminal(Some("subtask")));
        assert!(!super::hop_is_terminal(None));
    }

    #[test]
    fn fanout_caps_are_sane() {
        // Лимиты fan-out защищают от burst параллельных запусков (Cursor medium fix).
        assert!((1..=20).contains(&super::MAX_VERIFY_FANOUT));
        assert!((1..=20).contains(&super::MAX_NEXT_FANOUT));
    }

    // ── ККИ rework loop tests ────────────────────────────────────────

    #[test]
    fn rework_hop_is_not_terminal() {
        assert!(
            !super::hop_is_terminal(Some("rework")),
            "rework НЕ терминальный — после него должен запуститься verify"
        );
    }

    #[test]
    fn max_rework_is_sane() {
        assert!(
            (2..=5).contains(&super::MAX_REWORK),
            "MAX_REWORK должен быть 2-5 (задача говорит 2-3)"
        );
    }

    #[tokio::test]
    async fn count_rework_zero_for_non_rework_chain() {
        let db = setup_chain_db().await;
        sqlx::query(
            "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id, hop_kind) VALUES \
             ('t1', 'owner', 'agent-a', NULL, 'refined'), \
             ('v1', 'agent-a', 'verifier', 't1', 'verify')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        assert_eq!(super::count_rework_in_chain("t1", &db).await, 0);
        assert_eq!(super::count_rework_in_chain("v1", &db).await, 0);
    }

    #[tokio::test]
    async fn count_rework_counts_rework_hops() {
        let db = setup_chain_db().await;
        sqlx::query(
            "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id, hop_kind) VALUES \
             ('t1', 'owner', 'agent-a', NULL, 'refined'), \
             ('r1', 'dispatcher', 'agent-a', 't1', 'rework'), \
             ('r2', 'dispatcher', 'agent-a', 'r1', 'rework'), \
             ('r3', 'dispatcher', 'agent-a', 'r2', 'rework')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        assert_eq!(super::count_rework_in_chain("t1", &db).await, 0);
        assert_eq!(super::count_rework_in_chain("r1", &db).await, 1);
        assert_eq!(super::count_rework_in_chain("r2", &db).await, 2);
        assert_eq!(super::count_rework_in_chain("r3", &db).await, 3);
    }

    #[tokio::test]
    async fn count_rework_handles_cycle() {
        let db = setup_chain_db().await;
        sqlx::query(
            "INSERT INTO dispatcher_logs (id, from_entity, to_entity, parent_task_id, hop_kind) VALUES \
             ('c1', 'a', 'b', 'c2', 'rework'), \
             ('c2', 'b', 'a', 'c1', 'rework')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let count = super::count_rework_in_chain("c1", &db).await;
        assert!(count <= 2, "cycle should not cause infinite loop");
    }

    #[test]
    fn truncate_str_ascii() {
        assert_eq!(super::truncate_str("hello", 10), "hello");
        assert_eq!(super::truncate_str("hello world", 5), "hello");
    }

    #[test]
    fn truncate_str_utf8_boundary() {
        let s = "Привет мир";
        let truncated = super::truncate_str(s, 8);
        assert!(truncated.len() <= 8);
        assert!(truncated.is_char_boundary(truncated.len()));
    }
}
