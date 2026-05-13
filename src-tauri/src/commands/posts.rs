//! Posts CRUD (Step 3 — Module 2).
//!
//! Posts live inside Departments. Each post has a slug (URL-safe id), a human
//! title, a Hubbard "central product" (ЦКП), and an optional main statistic
//! metric. UI flow: open department card → click "+ Add post" → fill modal →
//! INSERT via `create_post`.

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::State;

use crate::db::WritePool;

#[derive(Debug, Deserialize)]
pub struct PostInput {
    pub department_id: String,
    pub slug: String,
    pub title: String,
    pub central_product: String,
    pub main_statistic_metric: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct Post {
    pub id: String,
    pub department_id: String,
    pub slug: String,
    pub title: String,
    pub central_product: String,
    pub main_statistic_metric: Option<String>,
    pub status: String,
    pub created_at: String,
}

static SLUG_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9](?:[a-z0-9-]{0,38}[a-z0-9])?$").unwrap());

/// Шаг 9: pub для re-use в `commands/tool_calls.rs` при `create_post` / `update_post`.
pub fn validate_slug(slug: &str) -> Result<(), String> {
    if !SLUG_RE.is_match(slug) {
        return Err(format!(
            "slug '{slug}' invalid (allowed: a-z 0-9 -, 2-40 chars, no leading/trailing dash)"
        ));
    }
    Ok(())
}

/// Шаг 9: pub для re-use в `commands/tool_calls.rs`.
pub fn validate_text(field: &str, value: &str, min: usize, max: usize) -> Result<(), String> {
    let len = value.chars().count();
    if len < min || len > max {
        return Err(format!(
            "{field} length must be {min}..={max}, got {len}"
        ));
    }
    Ok(())
}

/// Шаг 9: маппинг dept_number (0..7) → department_id (`dept-N-...`).
/// Возвращает None если отделения с таким номером нет (не должно случаться —
/// 8 отделений захардкожены в миграции 01_init.sql).
pub async fn dept_id_from_number(
    db: &WritePool,
    dept_number: i64,
) -> Result<Option<String>, String> {
    sqlx::query_as::<_, (String,)>("SELECT id FROM departments WHERE dept_number = ?")
        .bind(dept_number)
        .fetch_optional(&db.0)
        .await
        .map(|opt| opt.map(|(id,)| id))
        .map_err(|e| format!("dept lookup: {e}"))
}

#[tauri::command]
pub async fn create_post(
    input: PostInput,
    db: State<'_, WritePool>,
) -> Result<Post, String> {
    validate_slug(&input.slug)?;
    validate_text("title", &input.title, 2, 200)?;
    validate_text("central_product", &input.central_product, 5, 500)?;
    if let Some(metric) = &input.main_statistic_metric {
        validate_text("main_statistic_metric", metric, 0, 100).ok();
    }

    // Sanity check that the department exists — clearer error than
    // a foreign-key violation surfaced by SQLite.
    let dept_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM departments WHERE id = ?")
            .bind(&input.department_id)
            .fetch_optional(&db.0)
            .await
            .map_err(|e| format!("department check: {e}"))?;
    if dept_exists.is_none() {
        return Err(format!(
            "department '{}' does not exist",
            input.department_id
        ));
    }

    let id = format!("post-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO posts (id, department_id, slug, title, central_product, main_statistic_metric)
         VALUES (?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&input.department_id)
    .bind(&input.slug)
    .bind(&input.title)
    .bind(&input.central_product)
    .bind(&input.main_statistic_metric)
    .execute(&db.0)
    .await
    .map_err(|e| match e {
        sqlx::Error::Database(db_err) if db_err.code().as_deref() == Some("2067") => {
            format!("slug '{}' already taken", input.slug)
        }
        other => format!("insert post: {other}"),
    })?;

    fetch_post_by_id(&db, &id).await
}

#[tauri::command]
pub async fn list_posts_by_dept(
    department_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<Post>, String> {
    sqlx::query_as::<_, Post>(
        "SELECT id, department_id, slug, title, central_product, main_statistic_metric, status,
                created_at
         FROM posts
         WHERE department_id = ? AND status != 'archived'
         ORDER BY created_at ASC",
    )
    .bind(&department_id)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list posts: {e}"))
}

async fn fetch_post_by_id(db: &WritePool, id: &str) -> Result<Post, String> {
    sqlx::query_as::<_, Post>(
        "SELECT id, department_id, slug, title, central_product, main_statistic_metric, status,
                created_at
         FROM posts WHERE id = ?",
    )
    .bind(id)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch post: {e}"))
}
