//! Dispatcher — central task bus for cross-agent / cross-system messaging.
//!
//! Lifecycle:
//!   1. `dispatch_task` (or RPC `dispatcher/submit`) creates row with
//!      `status = 'in_progress'`. A UI/WS event `dispatcher-task-changed`
//!      fires so dashboards refresh without polling.
//!   2. Producer eventually closes the task via `complete_task(id, exec_ms)`
//!      or `fail_task(id, reason)` — both emit the same change event.
//!
//! `from_entity` / `to_entity` are free-form strings. Convention:
//!   • `owner` — Бровяков
//!   • `system` — internal jobs
//!   • `n8n`   — incoming n8n workflow trigger
//!   • `tg-bot`, `claude-architect`, `hermes`, ...
//!   • `<post slug>` (e.g. `frontend`) for routing to a department post

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::{AppHandle, Emitter, State};

use crate::db::WritePool;

const MAX_ENTITY_LEN: usize = 80;
const MAX_PAYLOAD_LEN: usize = 64 * 1024; // 64 KB raw JSON cap
const ACTIVE_LIMIT: i64 = 200;

static ENTITY_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-zA-Z0-9](?:[a-zA-Z0-9_\-/:]{0,78}[a-zA-Z0-9])?$").unwrap());

#[derive(Debug, Serialize, FromRow, Clone)]
pub struct DispatcherTask {
    pub id: String,
    pub from_entity: String,
    pub to_entity: String,
    pub task_payload: String,
    pub status: String,
    pub execution_time_ms: Option<i64>,
    pub created_at: String,
    // v1.0.22 Phase 11C — Hub-and-Spoke audit columns
    pub parent_task_id: Option<String>,
    pub completed_at: Option<String>,
    #[sqlx(default)]
    pub attempts_count: Option<i64>,
    pub hop_kind: Option<String>,
    pub routed_by_model: Option<String>,
    pub refined_prompt: Option<String>,
    pub outbox_path: Option<String>,
}

const TASK_SELECT_COLS: &str = "id, from_entity, to_entity, task_payload, status, \
    execution_time_ms, created_at, parent_task_id, completed_at, attempts_count, \
    hop_kind, routed_by_model, refined_prompt, outbox_path";

#[derive(Debug, Serialize, FromRow, Clone)]
pub struct DispatcherDecision {
    pub id: String,
    pub source_task_id: String,
    pub result_task_id: Option<String>,
    pub decision_kind: String,
    pub reasoning: Option<String>,
    pub model_used: String,
    pub routing_complexity: Option<String>,
    pub elapsed_ms: Option<i64>,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct DispatchTaskInput {
    pub from_entity: String,
    pub to_entity: String,
    pub payload: serde_json::Value,
}

fn validate_entity(name: &str, field: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err(format!("{field} is empty"));
    }
    if name.len() > MAX_ENTITY_LEN {
        return Err(format!(
            "{field} too long ({} > {MAX_ENTITY_LEN})",
            name.len()
        ));
    }
    if !ENTITY_RE.is_match(name) {
        return Err(format!(
            "{field} '{name}' contains invalid chars (allowed: a-z A-Z 0-9 _ - / :)"
        ));
    }
    Ok(())
}

// ─── Public Tauri commands ──────────────────────────────────────────────

#[tauri::command]
pub async fn dispatch_task(
    input: DispatchTaskInput,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<DispatcherTask, String> {
    dispatch_task_inner(
        input.from_entity,
        input.to_entity,
        input.payload,
        &db,
        &app,
    )
    .await
}

#[tauri::command]
pub async fn complete_task(
    task_id: String,
    execution_time_ms: Option<i64>,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<DispatcherTask, String> {
    complete_task_inner(task_id, execution_time_ms, &db, &app).await
}

#[tauri::command]
pub async fn fail_task(
    task_id: String,
    reason: String,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<DispatcherTask, String> {
    fail_task_inner(task_id, reason, &db, &app).await
}

#[tauri::command]
pub async fn list_active_tasks(
    db: State<'_, WritePool>,
) -> Result<Vec<DispatcherTask>, String> {
    sqlx::query_as::<_, DispatcherTask>(
        &format!("SELECT {TASK_SELECT_COLS}
         FROM dispatcher_logs
         WHERE status IN ('in_progress', 'failed')
         ORDER BY created_at DESC
         LIMIT ?"),
    )
    .bind(ACTIVE_LIMIT)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list active tasks: {e}"))
}

#[tauri::command]
pub async fn list_recent_tasks(
    limit: u32,
    db: State<'_, WritePool>,
) -> Result<Vec<DispatcherTask>, String> {
    let limit = limit.clamp(1, 1000) as i64;
    sqlx::query_as::<_, DispatcherTask>(
        &format!("SELECT {TASK_SELECT_COLS}
         FROM dispatcher_logs
         ORDER BY created_at DESC
         LIMIT ?"),
    )
    .bind(limit)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list recent tasks: {e}"))
}

// ─── Shared inner functions (callable from WS handlers without State<>) ──

pub async fn dispatch_task_inner(
    from_entity: String,
    to_entity: String,
    payload: serde_json::Value,
    db: &WritePool,
    app: &AppHandle,
) -> Result<DispatcherTask, String> {
    validate_entity(&from_entity, "from_entity")?;
    validate_entity(&to_entity, "to_entity")?;

    let payload_str = serde_json::to_string(&payload)
        .map_err(|e| format!("payload serialize: {e}"))?;
    if payload_str.len() > MAX_PAYLOAD_LEN {
        return Err(format!(
            "payload too large ({} bytes > {MAX_PAYLOAD_LEN})",
            payload_str.len()
        ));
    }

    let id = format!("task-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO dispatcher_logs (id, from_entity, to_entity, task_payload, status)
         VALUES (?, ?, ?, ?, 'in_progress')",
    )
    .bind(&id)
    .bind(&from_entity)
    .bind(&to_entity)
    .bind(&payload_str)
    .execute(&db.0)
    .await
    .map_err(|e| format!("insert dispatcher task: {e}"))?;

    let task = fetch_task_by_id(db, &id).await?;
    log::info!(
        "dispatch_task id={} from={} to={}",
        task.id,
        task.from_entity,
        task.to_entity
    );
    let _ = app.emit("dispatcher-task-changed", &task);
    Ok(task)
}

pub async fn complete_task_inner(
    task_id: String,
    execution_time_ms: Option<i64>,
    db: &WritePool,
    app: &AppHandle,
) -> Result<DispatcherTask, String> {
    let rows = sqlx::query(
        "UPDATE dispatcher_logs
         SET status = 'completed', execution_time_ms = ?, completed_at = CURRENT_TIMESTAMP
         WHERE id = ? AND status = 'in_progress'",
    )
    .bind(execution_time_ms)
    .bind(&task_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("complete task: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err(format!(
            "task {task_id} not found or not in_progress"
        ));
    }
    let task = fetch_task_by_id(db, &task_id).await?;
    log::info!("complete_task id={} exec_ms={:?}", task.id, execution_time_ms);
    let _ = app.emit("dispatcher-task-changed", &task);
    Ok(task)
}

pub async fn fail_task_inner(
    task_id: String,
    reason: String,
    db: &WritePool,
    app: &AppHandle,
) -> Result<DispatcherTask, String> {
    // We patch the existing payload to add an "error" field. If payload was
    // not valid JSON, replace it with a fresh object containing error+orig.
    let existing: Option<(String,)> =
        sqlx::query_as("SELECT task_payload FROM dispatcher_logs WHERE id = ?")
            .bind(&task_id)
            .fetch_optional(&db.0)
            .await
            .map_err(|e| format!("read existing payload: {e}"))?;
    let Some((existing_payload,)) = existing else {
        return Err(format!("task {task_id} not found"));
    };

    let merged = match serde_json::from_str::<serde_json::Value>(&existing_payload) {
        Ok(serde_json::Value::Object(mut map)) => {
            map.insert(
                "error".to_string(),
                serde_json::Value::String(reason.clone()),
            );
            serde_json::Value::Object(map).to_string()
        }
        _ => serde_json::json!({
            "error": reason,
            "original_payload": existing_payload
        })
        .to_string(),
    };

    sqlx::query(
        "UPDATE dispatcher_logs
         SET status = 'failed', task_payload = ?, completed_at = CURRENT_TIMESTAMP
         WHERE id = ?",
    )
    .bind(&merged)
    .bind(&task_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("fail task: {e}"))?;

    let task = fetch_task_by_id(db, &task_id).await?;
    log::info!("fail_task id={} reason={}", task.id, reason);
    let _ = app.emit("dispatcher-task-changed", &task);
    Ok(task)
}

// ─── v1.0.22 Phase 11C — Extended dispatch with hub-and-spoke metadata ─────

#[derive(Debug, Default)]
pub struct DispatchExtras {
    pub parent_task_id: Option<String>,
    pub hop_kind: Option<String>,
    pub routed_by_model: Option<String>,
    pub refined_prompt: Option<String>,
}

/// Допустимые значения `hop_kind` (write-side валидация вместо SQL CHECK —
/// даёт более понятные сообщения об ошибке). Извлечено из `dispatch_task_inner_ex`
/// (Day 5) для unit-тестирования; поведение идентично прежнему инлайну.
pub(crate) fn validate_hop_kind(hk: &str) -> Result<(), String> {
    const ALLOWED: &[&str] = &["raw_request", "refined", "subtask", "retry", "clarification"];
    if ALLOWED.contains(&hk) {
        Ok(())
    } else {
        Err(format!("invalid hop_kind '{hk}'"))
    }
}

/// Расширенная версия `dispatch_task_inner` которая заполняет колонки
/// Phase 11C (parent_task_id, hop_kind, routed_by_model, refined_prompt).
/// Старый `dispatch_task_inner` остаётся как backward-compat обёртка.
pub async fn dispatch_task_inner_ex(
    from_entity: String,
    to_entity: String,
    payload: serde_json::Value,
    extras: DispatchExtras,
    db: &WritePool,
    app: &AppHandle,
) -> Result<DispatcherTask, String> {
    validate_entity(&from_entity, "from_entity")?;
    validate_entity(&to_entity, "to_entity")?;

    let payload_str = serde_json::to_string(&payload)
        .map_err(|e| format!("payload serialize: {e}"))?;
    if payload_str.len() > MAX_PAYLOAD_LEN {
        return Err(format!(
            "payload too large ({} bytes > {MAX_PAYLOAD_LEN})",
            payload_str.len()
        ));
    }

    // Валидация hop_kind на write-side (вместо SQL CHECK — даёт лучшие сообщения).
    if let Some(hk) = &extras.hop_kind {
        validate_hop_kind(hk)?;
    }

    let id = format!("task-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO dispatcher_logs
            (id, from_entity, to_entity, task_payload, status,
             parent_task_id, hop_kind, routed_by_model, refined_prompt)
         VALUES (?, ?, ?, ?, 'in_progress', ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&from_entity)
    .bind(&to_entity)
    .bind(&payload_str)
    .bind(&extras.parent_task_id)
    .bind(&extras.hop_kind)
    .bind(&extras.routed_by_model)
    .bind(&extras.refined_prompt)
    .execute(&db.0)
    .await
    .map_err(|e| format!("insert dispatcher task: {e}"))?;

    let task = fetch_task_by_id(db, &id).await?;
    log::info!(
        "dispatch_task_ex id={} from={} to={} hop={:?} parent={:?}",
        task.id, task.from_entity, task.to_entity, task.hop_kind, task.parent_task_id
    );
    let _ = app.emit("dispatcher-task-changed", &task);
    Ok(task)
}

/// Сматывает цепочку hop'ов снизу-вверх: от current_task → parent → ... → root.
/// Cap=20 depth (защита от циклов / повреждённых данных).
pub async fn chain_for_task(
    task_id: &str,
    db: &WritePool,
) -> Result<Vec<DispatcherTask>, String> {
    let mut chain: Vec<DispatcherTask> = Vec::new();
    let mut current_id: Option<String> = Some(task_id.to_string());
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for _depth in 0..20 {
        let Some(id) = current_id.take() else { break };
        if !seen.insert(id.clone()) {
            log::warn!("chain_for_task: cycle detected at {id}, breaking");
            break;
        }
        match fetch_task_by_id(db, &id).await {
            Ok(t) => {
                current_id = t.parent_task_id.clone();
                chain.push(t);
            }
            Err(_) => break,
        }
    }
    Ok(chain)
}

#[tauri::command]
pub async fn get_task_chain(
    task_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<DispatcherTask>, String> {
    chain_for_task(&task_id, &db).await
}

/// Записывает AI-решение Диспетчера в журнал dispatcher_decisions.
pub async fn record_decision(
    source_task_id: &str,
    result_task_id: Option<&str>,
    decision_kind: &str,
    reasoning: Option<&str>,
    model_used: &str,
    routing_complexity: Option<&str>,
    elapsed_ms: Option<i64>,
    db: &WritePool,
) -> Result<String, String> {
    let id = format!("dec-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO dispatcher_decisions
            (id, source_task_id, result_task_id, decision_kind, reasoning,
             model_used, routing_complexity, elapsed_ms)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(source_task_id)
    .bind(result_task_id)
    .bind(decision_kind)
    .bind(reasoning)
    .bind(model_used)
    .bind(routing_complexity)
    .bind(elapsed_ms)
    .execute(&db.0)
    .await
    .map_err(|e| format!("record_decision: {e}"))?;
    Ok(id)
}

#[tauri::command]
pub async fn list_decisions_for_task(
    task_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<DispatcherDecision>, String> {
    sqlx::query_as::<_, DispatcherDecision>(
        "SELECT id, source_task_id, result_task_id, decision_kind, reasoning,
                model_used, routing_complexity, elapsed_ms, created_at
         FROM dispatcher_decisions
         WHERE source_task_id = ? OR result_task_id = ?
         ORDER BY created_at ASC",
    )
    .bind(&task_id)
    .bind(&task_id)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list decisions: {e}"))
}

/// Увеличивает attempts_count для retry-сценария.
pub async fn bump_attempts(task_id: &str, db: &WritePool) -> Result<i64, String> {
    sqlx::query(
        "UPDATE dispatcher_logs SET attempts_count = attempts_count + 1 WHERE id = ?",
    )
    .bind(task_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("bump_attempts: {e}"))?;
    let row: (i64,) = sqlx::query_as("SELECT attempts_count FROM dispatcher_logs WHERE id = ?")
        .bind(task_id)
        .fetch_one(&db.0)
        .await
        .map_err(|e| format!("read attempts: {e}"))?;
    Ok(row.0)
}

pub async fn fetch_task_by_id_public(
    db: &WritePool,
    id: &str,
) -> Result<DispatcherTask, String> {
    fetch_task_by_id(db, id).await
}

async fn fetch_task_by_id(db: &WritePool, id: &str) -> Result<DispatcherTask, String> {
    sqlx::query_as::<_, DispatcherTask>(
        &format!("SELECT {TASK_SELECT_COLS}
         FROM dispatcher_logs WHERE id = ?"),
    )
    .bind(id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch task by id: {e}"))
}

// ---------------------------------------------------------------------------
// Tests (Day 5 — forward-path critical-path)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    #[test]
    fn validate_hop_kind_accepts_allowed() {
        for hk in ["raw_request", "refined", "subtask", "retry", "clarification"] {
            assert!(validate_hop_kind(hk).is_ok(), "{hk} должен быть разрешён");
        }
    }

    #[test]
    fn validate_hop_kind_rejects_unknown() {
        for hk in ["forward_to_post", "", "REFINED"] {
            assert!(validate_hop_kind(hk).is_err(), "{hk} должен быть отклонён");
        }
    }

    #[tokio::test]
    async fn record_decision_writes_row() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        // FK off: юнит-тестируем сам INSERT, не ссылочную целостность
        // (родительских строк dispatcher_logs в тесте нет).
        sqlx::query("PRAGMA foreign_keys=OFF").execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TABLE dispatcher_decisions ( \
                id TEXT PRIMARY KEY, \
                source_task_id TEXT NOT NULL REFERENCES dispatcher_logs(id), \
                result_task_id TEXT REFERENCES dispatcher_logs(id), \
                decision_kind TEXT NOT NULL CHECK (decision_kind IN ('forward','decompose','escalate','reject','clarify','retry')), \
                reasoning TEXT, \
                model_used TEXT NOT NULL, \
                routing_complexity TEXT CHECK (routing_complexity IS NULL OR routing_complexity IN ('simple','complex')), \
                elapsed_ms INTEGER, \
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP \
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        let db = WritePool(pool);

        let id = record_decision(
            "src-1",
            Some("res-1"),
            "forward",
            Some("test reason"),
            "qwen3:14b",
            Some("simple"),
            Some(42),
            &db,
        )
        .await
        .unwrap();
        assert!(id.starts_with("dec-"));

        let row: (String, String, Option<String>) = sqlx::query_as(
            "SELECT decision_kind, source_task_id, result_task_id \
             FROM dispatcher_decisions WHERE id = ?",
        )
        .bind(&id)
        .fetch_one(&db.0)
        .await
        .unwrap();
        assert_eq!(row.0, "forward");
        assert_eq!(row.1, "src-1");
        assert_eq!(row.2.as_deref(), Some("res-1"));
    }
}
