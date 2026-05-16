//! Artifacts — Centralized Outbox для результатов задач (v1.0.22 Phase 11C).
//!
//! Связь:
//!   * Физика: `<app_data>/Outbox/<task_id>/result.docx` (см. `outbox.rs`)
//!   * Реестр: SQLite `task_artifacts(id, task_id, rel_path, mime, size, ...)`
//!   * Status: approved_at / rejected_at управляются Владельцем через UI
//!   * Когда **все** артефакты задачи approved → complete_task
//!   * При reject → bump_attempts и (опционально) retry через Диспетчера

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::{AppHandle, Emitter, Manager, State};

use crate::commands::dispatcher;
use crate::db::WritePool;
use crate::outbox;
use crate::vault::VaultState;

#[derive(Debug, Serialize, FromRow, Clone)]
pub struct Artifact {
    pub id: String,
    pub task_id: String,
    pub rel_path: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub created_by: String,
    pub created_at: String,
    pub approved_at: Option<String>,
    pub rejected_at: Option<String>,
    pub reject_reason: Option<String>,
}

/// Регистрирует физически существующий файл в реестре `task_artifacts`.
/// rel_path должен быть относительно `Outbox/<task_id>/`.
pub async fn register_artifact(
    task_id: &str,
    rel_path: &str,
    mime_type: Option<&str>,
    created_by: &str,
    db: &WritePool,
    vault_state: &VaultState,
    app: &AppHandle,
) -> Result<Artifact, String> {
    let abs = outbox::safe_artifact_path(&vault_state.root, task_id, rel_path)?;
    let size_bytes = std::fs::metadata(&abs)
        .map(|m| m.len() as i64)
        .map_err(|e| format!("file metadata: {e}"))?;

    if size_bytes as u64 > outbox::ARTIFACT_MAX_BYTES {
        return Err(format!(
            "artifact size {} > cap {}",
            size_bytes,
            outbox::ARTIFACT_MAX_BYTES
        ));
    }

    let safe_rel = outbox::sanitize_rel_path(rel_path)?;
    let id = format!("art-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO task_artifacts
            (id, task_id, rel_path, mime_type, size_bytes, created_by)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(task_id, rel_path) DO UPDATE SET
            size_bytes = excluded.size_bytes,
            mime_type = excluded.mime_type,
            created_by = excluded.created_by",
    )
    .bind(&id)
    .bind(task_id)
    .bind(&safe_rel)
    .bind(mime_type)
    .bind(size_bytes)
    .bind(created_by)
    .execute(&db.0)
    .await
    .map_err(|e| format!("insert artifact: {e}"))?;

    // Обновляем outbox_path в dispatcher_logs если ещё пусто.
    sqlx::query(
        "UPDATE dispatcher_logs SET outbox_path = ?
         WHERE id = ? AND (outbox_path IS NULL OR outbox_path = '')",
    )
    .bind(format!("Outbox/{task_id}/"))
    .bind(task_id)
    .execute(&db.0)
    .await
    .ok();

    let art = fetch_artifact(&id, db).await?;
    let _ = app.emit("artifact-changed", &art);
    Ok(art)
}

async fn fetch_artifact(id: &str, db: &WritePool) -> Result<Artifact, String> {
    sqlx::query_as::<_, Artifact>(
        "SELECT id, task_id, rel_path, mime_type, size_bytes, created_by,
                created_at, approved_at, rejected_at, reject_reason
         FROM task_artifacts WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch artifact: {e}"))
}

#[tauri::command]
pub async fn list_task_artifacts(
    task_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<Artifact>, String> {
    sqlx::query_as::<_, Artifact>(
        "SELECT id, task_id, rel_path, mime_type, size_bytes, created_by,
                created_at, approved_at, rejected_at, reject_reason
         FROM task_artifacts WHERE task_id = ? ORDER BY created_at ASC",
    )
    .bind(&task_id)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list artifacts: {e}"))
}

#[tauri::command]
pub async fn open_artifact_in_default_app(
    artifact_id: String,
    db: State<'_, WritePool>,
    vault: State<'_, VaultState>,
) -> Result<(), String> {
    let art = fetch_artifact(&artifact_id, &db).await?;
    let abs = outbox::safe_artifact_path(&vault.root, &art.task_id, &art.rel_path)?;
    if !abs.exists() {
        return Err(format!("file not found: {}", abs.display()));
    }
    std::process::Command::new("explorer")
        .arg(&abs)
        .spawn()
        .map_err(|e| format!("spawn explorer: {e}"))?;
    Ok(())
}

#[tauri::command]
pub async fn approve_artifact(
    artifact_id: String,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<Artifact, String> {
    let rows = sqlx::query(
        "UPDATE task_artifacts SET approved_at = CURRENT_TIMESTAMP, rejected_at = NULL, reject_reason = NULL
         WHERE id = ? AND approved_at IS NULL",
    )
    .bind(&artifact_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("approve artifact: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err("artifact уже утверждён или не найден".into());
    }
    let art = fetch_artifact(&artifact_id, &db).await?;
    let _ = app.emit("artifact-changed", &art);

    // Если у task все артефакты approved — completes the task
    let pending: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM task_artifacts
         WHERE task_id = ? AND approved_at IS NULL AND rejected_at IS NULL",
    )
    .bind(&art.task_id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("count pending: {e}"))?;
    if pending.0 == 0 {
        let _ = dispatcher::complete_task_inner(art.task_id.clone(), None, &db, &app).await;
    }
    Ok(art)
}

#[derive(Debug, Deserialize)]
pub struct RejectArtifactInput {
    pub artifact_id: String,
    pub reject_reason: String,
}

#[tauri::command]
pub async fn reject_artifact(
    input: RejectArtifactInput,
    db: State<'_, WritePool>,
    app: AppHandle,
) -> Result<Artifact, String> {
    if input.reject_reason.trim().is_empty() {
        return Err("reject_reason обязательный".into());
    }
    sqlx::query(
        "UPDATE task_artifacts
         SET rejected_at = CURRENT_TIMESTAMP, reject_reason = ?, approved_at = NULL
         WHERE id = ?",
    )
    .bind(&input.reject_reason)
    .bind(&input.artifact_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("reject artifact: {e}"))?;
    let art = fetch_artifact(&input.artifact_id, &db).await?;
    let _ = app.emit("artifact-changed", &art);

    // Bump attempts на родительской задаче (для retry-сценария).
    let _ = dispatcher::bump_attempts(&art.task_id, &db).await;
    Ok(art)
}

/// Регистрирует артефакт извне (UI / WS RPC / тесты). Файл должен УЖЕ
/// физически лежать в `<app_data>/Outbox/<task_id>/<rel_path>`.
#[derive(Debug, Deserialize)]
pub struct RegisterArtifactInput {
    pub task_id: String,
    pub rel_path: String,
    pub mime_type: Option<String>,
    pub created_by: String,
}

#[tauri::command]
pub async fn register_external_artifact(
    input: RegisterArtifactInput,
    db: State<'_, WritePool>,
    vault: State<'_, VaultState>,
    app: AppHandle,
) -> Result<Artifact, String> {
    register_artifact(
        &input.task_id,
        &input.rel_path,
        input.mime_type.as_deref(),
        &input.created_by,
        &db,
        vault.inner(),
        &app,
    )
    .await
}

/// Для smoke-теста V6: создаёт fake-артефакт (пустой файл с заданным содержимым).
/// Полезно когда реальных агентов-исполнителей ещё нет (Phase 11B).
#[derive(Debug, Deserialize)]
pub struct CreateFakeArtifactInput {
    pub task_id: String,
    pub rel_path: String,
    pub content: String,
    pub mime_type: Option<String>,
}

#[tauri::command]
pub async fn create_fake_artifact(
    input: CreateFakeArtifactInput,
    db: State<'_, WritePool>,
    vault: State<'_, VaultState>,
    app: AppHandle,
) -> Result<Artifact, String> {
    let abs = outbox::safe_artifact_path(&vault.root, &input.task_id, &input.rel_path)?;
    std::fs::write(&abs, input.content.as_bytes())
        .map_err(|e| format!("write fake artifact: {e}"))?;
    register_artifact(
        &input.task_id,
        &input.rel_path,
        input.mime_type.as_deref(),
        "fake-agent",
        &db,
        vault.inner(),
        &app,
    )
    .await
}
