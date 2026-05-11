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
        "SELECT id, from_entity, to_entity, task_payload, status, execution_time_ms, created_at
         FROM dispatcher_logs
         WHERE status IN ('in_progress', 'failed')
         ORDER BY created_at DESC
         LIMIT ?",
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
        "SELECT id, from_entity, to_entity, task_payload, status, execution_time_ms, created_at
         FROM dispatcher_logs
         ORDER BY created_at DESC
         LIMIT ?",
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
         SET status = 'completed', execution_time_ms = ?
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
         SET status = 'failed', task_payload = ?
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

async fn fetch_task_by_id(db: &WritePool, id: &str) -> Result<DispatcherTask, String> {
    sqlx::query_as::<_, DispatcherTask>(
        "SELECT id, from_entity, to_entity, task_payload, status, execution_time_ms, created_at
         FROM dispatcher_logs WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch task by id: {e}"))
}
