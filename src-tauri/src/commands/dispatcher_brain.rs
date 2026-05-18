//! Dispatcher Brain — двухконтурный мозг Диспетчера (v1.0.22 Phase 11C).
//!
//! Поток `process_pending(task_id)`:
//!   1. Загружает raw_request из dispatcher_logs
//!   2. pick_brain_for_routing() — Qwen (default, дёшево) или Claude Sonnet
//!      (если сложная задача с decomposition/conflict-keywords)
//!   3. build_dispatcher_system_prompt — собирает контекст:
//!      org structure + post knowledge + DISPATCHER_TOOLS_PREAMBLE
//!   4. run_qwen_for_dispatcher / run_claude_cli_for_dispatcher
//!   5. parse_tool_calls — ровно один tool_call в ответе
//!   6. Исполняет:
//!      forward_to_post → INSERT row #2 (refined, parent_task_id)
//!      decompose_task → INSERT N rows (subtask, общий parent_task_id)
//!      escalate_to_ceo → отмечает raw_request как escalated
//!      reject_task → fail_task_inner
//!      clarify → INSERT row (clarification, target=ceo)
//!   7. record_decision() — журналит решение в dispatcher_decisions
//!   8. Если Qwen упал/выдал мусор и dispatcher_auto_fallback_claude=true —
//!      второй проход на Claude Sonnet

use std::time::Instant;

use serde_json::Value;
use tauri::{AppHandle, Manager};

use crate::commands::claude_bridge::{self, DispatcherLifecycle};
use crate::commands::dispatcher::{
    self, DispatchExtras, DispatcherTask,
};
use crate::commands::dispatcher_tools::DISPATCHER_TOOLS_PREAMBLE;
use crate::commands::qwen_bridge;
use crate::commands::tool_calls::parse_tool_calls;
use crate::db::WritePool;
use crate::settings::AppSettings;

/// Какой brain используется для конкретной задачи.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrainPick {
    Qwen,
    Claude,
}

impl BrainPick {
    pub fn model_label(&self, settings: &AppSettings) -> String {
        match self {
            BrainPick::Qwen => settings.dispatcher_qwen_model.clone(),
            BrainPick::Claude => settings.dispatcher_claude_model.clone(),
        }
    }
    pub fn complexity_label(&self) -> &'static str {
        match self {
            BrainPick::Qwen => "simple",
            BrainPick::Claude => "complex",
        }
    }
}

/// Heuristic — решает Qwen или Claude по тексту raw_prompt + наличию target_hint.
pub fn pick_brain_for_routing(
    raw_prompt: &str,
    target_hint: Option<&str>,
    settings: &AppSettings,
) -> BrainPick {
    if settings.dispatcher_brain_mode == "claude_primary" {
        return BrainPick::Claude;
    }
    let prompt_short = raw_prompt.chars().count() < 500;
    let has_hint = target_hint.map(|s| !s.trim().is_empty()).unwrap_or(false);
    let conflict = contains_any(raw_prompt, &[
        "конфликт", "спор", "не согласн", "разногласи", "противореч",
    ]);
    let decomp = contains_any(raw_prompt, &[
        "несколько", "сначала", "затем", "потом", "после этого",
        " + ", " и ", " а также ", " плюс ",
    ]);
    if has_hint && prompt_short && !conflict && !decomp {
        BrainPick::Qwen
    } else {
        BrainPick::Claude
    }
}

fn contains_any(haystack: &str, needles: &[&str]) -> bool {
    let lower = haystack.to_lowercase();
    needles.iter().any(|n| lower.contains(*n))
}

/// Собирает system+user prompt для Диспетчера. system — что он за агент.
/// user — конкретная задача с контекстом поста (если target_hint указан).
pub async fn build_dispatcher_prompt(
    raw_task: &DispatcherTask,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(String, String), String> {
    // Парсим payload чтобы вытащить raw_prompt, target_hint, expected_artifact
    let payload: Value = serde_json::from_str(&raw_task.task_payload).unwrap_or(Value::Null);
    let raw_prompt = payload.get("raw_prompt").and_then(Value::as_str).unwrap_or("");
    let target_hint = payload.get("target_hint").and_then(Value::as_str);
    let expected_artifact = payload.get("expected_artifact").and_then(Value::as_str);

    // Подгружаем список постов (для list of valid target_slugs).
    let posts: Vec<(String, String, Option<String>)> = sqlx::query_as(
        "SELECT slug, title, central_product FROM posts WHERE status='active' ORDER BY slug",
    )
    .fetch_all(&db.0)
    .await
    .unwrap_or_default();

    let posts_block = if posts.is_empty() {
        "(нет активных постов)".to_string()
    } else {
        let mut s = String::new();
        for (slug, title, cp) in &posts {
            s.push_str(&format!(
                "- `{slug}` — {title}{}\n",
                cp.as_deref()
                    .map(|c| format!(" (ЦКП: {c})"))
                    .unwrap_or_default()
            ));
        }
        s
    };

    // Если target_hint указан и пост существует — подгружаем его знания
    // (system_prompt_md + Vault context до POST_CONTEXT_BYTES).
    let post_knowledge = if let Some(slug) = target_hint {
        load_post_knowledge_block(slug, db, app).await
    } else {
        String::new()
    };

    let system_prompt = format!(
        "# SYSTEM CONTEXT (DISPATCHER)\n\
        Ты — Интеллектуальный Диспетчер MSPro-Ltd Corp. Hub-and-Spoke брокер задач.\n\
        Прямое общение между Гендиром и постами запрещено — всё через тебя.\n\
        \n\
        ## Текущие посты (можно использовать как target_slug)\n\
        {posts_block}\n\
        {DISPATCHER_TOOLS_PREAMBLE}\n\
        ---\n\
        Помни: ОДИН tool_call в ответе. Не выполняй задачу сам — переписывай и адресуй.\n"
    );

    let user_prompt = format!(
        "# USER (raw task)\n\
        \n\
        **Источник:** {from}\n\
        **Сырой запрос (raw_prompt):**\n\
        \n\
        {raw_prompt}\n\
        \n\
        **target_hint (предложенный исполнитель):** {hint}\n\
        **expected_artifact:** {art}\n\
        \n\
        {post_knowledge}\n\
        \n\
        Прими решение: какой tool_call (forward_to_post / decompose_task / escalate_to_ceo \
        / reject_task / clarify) лучше всего подходит. Выведи РОВНО ОДИН <tool_call> блок.",
        from = raw_task.from_entity,
        hint = target_hint.unwrap_or("(не указано — выбери сам по содержанию)"),
        art = expected_artifact.unwrap_or("(не указано — выбери подходящий)"),
    );

    Ok((system_prompt, user_prompt))
}

async fn load_post_knowledge_block(slug: &str, db: &WritePool, app: &AppHandle) -> String {
    let row: Option<(String, Option<String>)> =
        sqlx::query_as("SELECT title, system_prompt_md FROM posts WHERE slug = ?")
            .bind(slug)
            .fetch_optional(&db.0)
            .await
            .unwrap_or(None);
    let Some((title, system_prompt)) = row else {
        return format!("**Знания поста `{slug}`:** _пост не найден_");
    };

    let vault_ctx = if let Some(vs) = app.try_state::<crate::vault::VaultState>() {
        crate::vault::read_post_context(
            vs.root.clone(),
            slug.to_string(),
            crate::vault::POST_CONTEXT_BYTES,
        )
        .await
        .unwrap_or_default()
    } else {
        String::new()
    };

    let mut out = format!("**Знания поста `{slug}` ({title}):**\n\n");
    if let Some(p) = system_prompt {
        if !p.trim().is_empty() {
            out.push_str("### Системный промпт поста\n\n");
            out.push_str(&p);
            out.push_str("\n\n");
        }
    }
    if !vault_ctx.trim().is_empty() {
        out.push_str("### Vault-опыт (первые 5 KB)\n\n");
        out.push_str(&vault_ctx);
        out.push('\n');
    }
    out
}

/// Главный orchestrator. Вызывается из `execute_send_to_dispatcher` через
/// `tokio::spawn` чтобы не блокировать chat-stream Гендира.
pub async fn process_pending(
    task_id: String,
    db: WritePool,
    settings: AppSettings,
    lifecycle: std::sync::Arc<DispatcherLifecycle>,
    app: AppHandle,
) -> Result<(), String> {
    let started = Instant::now();
    let task = match dispatcher::fetch_task_by_id_public(&db, &task_id).await {
        Ok(t) => t,
        Err(e) => {
            log::warn!("dispatcher process_pending: task {task_id} not found: {e}");
            return Err(e);
        }
    };

    // Только для raw_request: остальные hops уже обработаны.
    if task.hop_kind.as_deref() != Some("raw_request") {
        log::info!(
            "dispatcher process_pending: skip task {} (hop_kind={:?})",
            task.id, task.hop_kind
        );
        return Ok(());
    }

    // Парсим target_hint для выбора brain
    let payload: Value = serde_json::from_str(&task.task_payload).unwrap_or(Value::Null);
    let raw_prompt = payload.get("raw_prompt").and_then(Value::as_str).unwrap_or("");
    let target_hint = payload.get("target_hint").and_then(Value::as_str);

    let pick = pick_brain_for_routing(raw_prompt, target_hint, &settings);
    log::info!(
        "dispatcher: task {} picked brain={:?} (mode={})",
        task.id, pick, settings.dispatcher_brain_mode
    );

    let (system_prompt, user_prompt) = build_dispatcher_prompt(&task, &db, &app).await?;

    let routed_model = pick.model_label(&settings);

    let routing_result = match pick {
        BrainPick::Qwen => {
            qwen_bridge::run_qwen_for_dispatcher(
                &system_prompt,
                &user_prompt,
                &settings,
                &lifecycle,
                &app,
            )
            .await
        }
        BrainPick::Claude => {
            let full = format!("{system_prompt}\n\n{user_prompt}");
            claude_bridge::run_claude_cli_for_dispatcher(&full, &settings, &lifecycle, &app).await
        }
    };

    // Auto-fallback Qwen → Claude если включено и Qwen упал
    let (raw_response, final_model, used_fallback) = match routing_result {
        Ok(text) => (text, routed_model.clone(), false),
        Err(e) if pick == BrainPick::Qwen && settings.dispatcher_auto_fallback_claude => {
            log::warn!(
                "dispatcher: Qwen failed ({e}), auto-fallback to Claude for task {}",
                task.id
            );
            let full = format!("{system_prompt}\n\n{user_prompt}");
            match claude_bridge::run_claude_cli_for_dispatcher(
                &full,
                &settings,
                &lifecycle,
                &app,
            )
            .await
            {
                Ok(t) => (t, settings.dispatcher_claude_model.clone(), true),
                Err(e2) => {
                    log::error!(
                        "dispatcher: both Qwen+Claude failed for task {}: {e2}",
                        task.id
                    );
                    let _ = dispatcher::fail_task_inner(
                        task.id.clone(),
                        format!("dispatcher routing failed: {e2}"),
                        &db,
                        &app,
                    )
                    .await;
                    return Err(e2);
                }
            }
        }
        Err(e) => {
            log::error!("dispatcher: routing failed for task {}: {e}", task.id);
            let _ = dispatcher::fail_task_inner(
                task.id.clone(),
                format!("dispatcher routing failed: {e}"),
                &db,
                &app,
            )
            .await;
            return Err(e);
        }
    };

    // Парсим tool_call из ответа Диспетчера
    let (_cleaned, calls) = parse_tool_calls(&raw_response);
    let elapsed_ms = started.elapsed().as_millis() as i64;
    let complexity = if used_fallback || pick == BrainPick::Claude {
        "complex"
    } else {
        "simple"
    };

    if calls.is_empty() {
        log::warn!(
            "dispatcher: no tool_call from brain ({}). raw: {}...",
            final_model,
            raw_response.chars().take(200).collect::<String>()
        );
        let _ = dispatcher::record_decision(
            &task.id,
            None,
            "reject",
            Some(&format!(
                "no tool_call parsed from {final_model} response. \
                 raw: {}...",
                raw_response.chars().take(500).collect::<String>()
            )),
            &final_model,
            Some(complexity),
            Some(elapsed_ms),
            &db,
        )
        .await;
        let _ = dispatcher::fail_task_inner(
            task.id.clone(),
            "dispatcher: no tool_call in brain response".into(),
            &db,
            &app,
        )
        .await;
        return Err("no tool_call".into());
    }

    // Берём ПЕРВЫЙ tool_call (правило: один в ответе)
    let call = &calls[0];
    let args = if call.arguments.is_object() || call.arguments.is_array() {
        &call.arguments
    } else {
        &call.args
    };

    log::info!(
        "dispatcher: task {} decision={} model={}",
        task.id, call.name, final_model
    );

    let decision_outcome = match call.name.as_str() {
        "forward_to_post" => {
            execute_forward(&task, args, &final_model, &db, &app).await
        }
        "decompose_task" => {
            execute_decompose(&task, args, &final_model, &db, &app).await
        }
        "escalate_to_ceo" => {
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("(no reason)");
            execute_escalate(&task, reason, &final_model, &db, &app).await
        }
        "reject_task" => {
            let reason = args
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("(no reason)");
            execute_reject(&task, reason, &final_model, &db, &app).await
        }
        "clarify" => {
            let q = args
                .get("question_to_source")
                .and_then(Value::as_str)
                .unwrap_or("(no question)");
            execute_clarify(&task, q, &final_model, &db, &app).await
        }
        unknown => Err(format!("unknown dispatcher tool '{unknown}'")),
    };

    match &decision_outcome {
        Ok((kind, result_id, reasoning)) => {
            let _ = dispatcher::record_decision(
                &task.id,
                result_id.as_deref(),
                kind,
                Some(reasoning.as_str()),
                &final_model,
                Some(complexity),
                Some(elapsed_ms),
                &db,
            )
            .await;
            // v1.0.23: parent raw_request обработан — закрываем completed,
            // дальнейшая работа продолжается в refined/subtask потомках.
            // Исключение: clarify/escalate не закрывают (ждём ответа).
            if matches!(*kind, "forward" | "decompose" | "reject") {
                let close_msg = format!("dispatcher decision: {kind} ({reasoning})");
                if *kind == "reject" {
                    let _ = dispatcher::fail_task_inner(
                        task.id.clone(),
                        close_msg,
                        &db,
                        &app,
                    )
                    .await;
                } else {
                    let _ = dispatcher::complete_task_inner(
                        task.id.clone(),
                        Some(elapsed_ms as i64),
                        &db,
                        &app,
                    )
                    .await;
                }
            }
        }
        Err(e) => {
            let _ = dispatcher::record_decision(
                &task.id,
                None,
                "reject",
                Some(&format!("execution failed: {e}")),
                &final_model,
                Some(complexity),
                Some(elapsed_ms),
                &db,
            )
            .await;
            // v1.0.23: закрываем parent task (иначе застрянет in_progress навечно)
            let _ = dispatcher::fail_task_inner(
                task.id.clone(),
                format!("dispatcher rejected: {e}"),
                &db,
                &app,
            )
            .await;
        }
    }

    decision_outcome.map(|_| ()).map_err(|e| e.to_string())
}

// ---------------------------------------------------------------------------
// Executor helpers — каждый возвращает (decision_kind, result_task_id?, reasoning)
// ---------------------------------------------------------------------------

async fn execute_forward(
    parent: &DispatcherTask,
    args: &Value,
    model: &str,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(&'static str, Option<String>, String), String> {
    let refined = args
        .get("refined_prompt")
        .and_then(Value::as_str)
        .ok_or_else(|| "forward_to_post: refined_prompt missing".to_string())?;

    // v1.0.23: если Opus забыл target_slug — пробуем три fallback'а по очереди:
    // 1) parent.payload.target_hint (если Гендир указал)
    // 2) Любой реальный slug встретившийся в refined_prompt или raw_prompt
    // 3) Если только один активный пост в БД — берём его
    let target_owned: String;
    let target: &str = match args.get("target_slug").and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => s,
        _ => {
            // 1) target_hint
            let hint = serde_json::from_str::<Value>(&parent.task_payload)
                .ok()
                .and_then(|v| {
                    v.get("target_hint")
                        .and_then(Value::as_str)
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                });
            if let Some(h) = hint {
                log::warn!(
                    "dispatcher: forward target_slug missing, fallback to parent target_hint='{h}'"
                );
                target_owned = h;
                &target_owned
            } else {
                // 2) ищем слаг любого active поста в refined_prompt / raw_prompt
                let raw = serde_json::from_str::<Value>(&parent.task_payload)
                    .ok()
                    .and_then(|v| {
                        v.get("raw_prompt")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    })
                    .unwrap_or_default();
                let haystack = format!("{} {}", refined.to_lowercase(), raw.to_lowercase());

                let slugs: Vec<(String,)> =
                    sqlx::query_as("SELECT slug FROM posts WHERE status = 'active'")
                        .fetch_all(&db.0)
                        .await
                        .map_err(|e| format!("posts lookup: {e}"))?;

                let mut matched: Option<String> = None;
                for (s,) in &slugs {
                    if haystack.contains(&s.to_lowercase()) {
                        matched = Some(s.clone());
                        break;
                    }
                }

                if let Some(m) = matched {
                    log::warn!(
                        "dispatcher: forward target_slug missing, extracted slug='{m}' from prompts"
                    );
                    target_owned = m;
                    &target_owned
                } else if slugs.len() == 1 {
                    // 3) одиночный пост → fallback на него
                    log::warn!(
                        "dispatcher: forward target_slug missing, only-one-post fallback to '{}'",
                        slugs[0].0
                    );
                    target_owned = slugs[0].0.clone();
                    &target_owned
                } else {
                    return Err(format!(
                        "forward_to_post: target_slug missing; не нашёл ни target_hint, ни упоминание slug в prompts. Активных постов: {}",
                        slugs.len()
                    ));
                }
            }
        }
    };
    let expected_artifact = args.get("expected_artifact").and_then(Value::as_str);

    // Проверка существования поста
    let exists: Option<(String,)> = sqlx::query_as("SELECT slug FROM posts WHERE slug = ?")
        .bind(target)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("post lookup: {e}"))?;
    if exists.is_none() {
        return Err(format!(
            "post '{target}' не существует — нельзя forward"
        ));
    }

    let payload = serde_json::json!({
        "from_parent_task_id": parent.id,
        "refined_prompt": refined,
        "expected_artifact": expected_artifact,
    });

    let new_task = dispatcher::dispatch_task_inner_ex(
        "dispatcher".to_string(),
        target.to_string(),
        payload,
        DispatchExtras {
            parent_task_id: Some(parent.id.clone()),
            hop_kind: Some("refined".to_string()),
            routed_by_model: Some(model.to_string()),
            refined_prompt: Some(refined.to_string()),
        },
        db,
        app,
    )
    .await?;

    // v1.0.24 Phase 11B-1: auto-trigger пост-агента. Non-blocking — refined task
    // получает реальный мозг (claude.exe со своим agent.md в Outbox sandbox).
    // Без этого refined task висел бы in_progress вечно (старый bug 11C).
    crate::commands::post_executor::trigger_post_executor(
        new_task.id.clone(),
        target.to_string(),
        refined.to_string(),
        expected_artifact.map(str::to_string),
        app.clone(),
    );

    Ok((
        "forward",
        Some(new_task.id.clone()),
        format!("forward → `{target}` + spawned post-agent (refined len {} chars)", refined.len()),
    ))
}

async fn execute_decompose(
    parent: &DispatcherTask,
    args: &Value,
    model: &str,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(&'static str, Option<String>, String), String> {
    let subtasks = args
        .get("subtasks")
        .and_then(Value::as_array)
        .ok_or_else(|| "decompose_task: subtasks (array) missing".to_string())?;
    if subtasks.is_empty() {
        return Err("decompose_task: empty subtasks".into());
    }
    if subtasks.len() > 8 {
        return Err(format!(
            "decompose_task: too many subtasks ({}), cap=8",
            subtasks.len()
        ));
    }

    let mut created_ids: Vec<String> = Vec::with_capacity(subtasks.len());
    for (i, st) in subtasks.iter().enumerate() {
        let target = st
            .get("target_slug")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("subtask[{i}]: target_slug missing"))?;
        let refined = st
            .get("refined_prompt")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("subtask[{i}]: refined_prompt missing"))?;
        let payload = serde_json::json!({
            "from_parent_task_id": parent.id,
            "subtask_index": i,
            "refined_prompt": refined,
            "expected_artifact": st.get("expected_artifact"),
        });
        let new_task = dispatcher::dispatch_task_inner_ex(
            "dispatcher".to_string(),
            target.to_string(),
            payload,
            DispatchExtras {
                parent_task_id: Some(parent.id.clone()),
                hop_kind: Some("subtask".to_string()),
                routed_by_model: Some(model.to_string()),
                refined_prompt: Some(refined.to_string()),
            },
            db,
            app,
        )
        .await?;

        // v1.0.24 Phase 11B-1: каждый subtask тоже spawn'ит реальный пост-агент.
        let expected_artifact = st
            .get("expected_artifact")
            .and_then(Value::as_str)
            .map(str::to_string);
        crate::commands::post_executor::trigger_post_executor(
            new_task.id.clone(),
            target.to_string(),
            refined.to_string(),
            expected_artifact,
            app.clone(),
        );

        created_ids.push(new_task.id);
    }

    Ok((
        "decompose",
        created_ids.first().cloned(),
        format!("decompose → {} subtasks: {}", created_ids.len(), created_ids.join(", ")),
    ))
}

async fn execute_escalate(
    parent: &DispatcherTask,
    reason: &str,
    _model: &str,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(&'static str, Option<String>, String), String> {
    // Помечаем raw_request как failed с reason — Владелец увидит в UI Inbox
    // что Диспетчер не смог разрулить.
    let _ = dispatcher::fail_task_inner(
        parent.id.clone(),
        format!("escalated to CEO: {reason}"),
        db,
        app,
    )
    .await;
    Ok(("escalate", None, format!("escalate: {reason}")))
}

async fn execute_reject(
    parent: &DispatcherTask,
    reason: &str,
    _model: &str,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(&'static str, Option<String>, String), String> {
    let _ = dispatcher::fail_task_inner(
        parent.id.clone(),
        format!("rejected by dispatcher: {reason}"),
        db,
        app,
    )
    .await;
    Ok(("reject", None, format!("reject: {reason}")))
}

async fn execute_clarify(
    parent: &DispatcherTask,
    question: &str,
    model: &str,
    db: &WritePool,
    app: &AppHandle,
) -> Result<(&'static str, Option<String>, String), String> {
    // Создаём clarification-task назад к источнику (ceo).
    let payload = serde_json::json!({
        "from_parent_task_id": parent.id,
        "question_to_source": question,
    });
    let new_task = dispatcher::dispatch_task_inner_ex(
        "dispatcher".to_string(),
        parent.from_entity.clone(),
        payload,
        DispatchExtras {
            parent_task_id: Some(parent.id.clone()),
            hop_kind: Some("clarification".to_string()),
            routed_by_model: Some(model.to_string()),
            refined_prompt: None,
        },
        db,
        app,
    )
    .await?;
    Ok((
        "clarify",
        Some(new_task.id.clone()),
        format!("clarify: {question}"),
    ))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn default_settings() -> AppSettings {
        AppSettings::default()
    }

    #[test]
    fn pick_brain_simple_goes_to_qwen() {
        let s = default_settings();
        let pick = pick_brain_for_routing("составь короткое письмо", Some("manager"), &s);
        assert_eq!(pick, BrainPick::Qwen);
    }

    #[test]
    fn pick_brain_no_hint_escalates_to_claude() {
        let s = default_settings();
        let pick = pick_brain_for_routing("сделай что-нибудь", None, &s);
        assert_eq!(pick, BrainPick::Claude);
    }

    #[test]
    fn pick_brain_long_prompt_goes_to_claude() {
        let s = default_settings();
        let long = "a".repeat(600);
        let pick = pick_brain_for_routing(&long, Some("manager"), &s);
        assert_eq!(pick, BrainPick::Claude);
    }

    #[test]
    fn pick_brain_decomposition_signal_to_claude() {
        let s = default_settings();
        let pick = pick_brain_for_routing(
            "договор и смета",
            Some("manager"),
            &s,
        );
        assert_eq!(pick, BrainPick::Claude);
    }

    #[test]
    fn pick_brain_conflict_keyword_to_claude() {
        let s = default_settings();
        let pick = pick_brain_for_routing(
            "разреши конфликт между постами",
            Some("manager"),
            &s,
        );
        assert_eq!(pick, BrainPick::Claude);
    }

    #[test]
    fn pick_brain_claude_primary_always_claude() {
        let mut s = default_settings();
        s.dispatcher_brain_mode = "claude_primary".to_string();
        let pick = pick_brain_for_routing("короткая", Some("manager"), &s);
        assert_eq!(pick, BrainPick::Claude);
    }

    #[test]
    fn pick_brain_complexity_label() {
        assert_eq!(BrainPick::Qwen.complexity_label(), "simple");
        assert_eq!(BrainPick::Claude.complexity_label(), "complex");
    }
}
