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

// ---------------------------------------------------------------------------
// v1.0.19 — Per-post Knowledge (system prompt + own Vault folder)
// ---------------------------------------------------------------------------

/// Hard cap для системного промпта поста: 100 KB — типичный CLAUDE.md ~ 5-20 KB.
const POST_SYSTEM_PROMPT_MAX_BYTES: usize = 100_000;

#[derive(Debug, Serialize)]
pub struct PostKnowledge {
    pub slug: String,
    pub title: String,
    pub system_prompt_md: Option<String>,
    pub vault_subdir: Option<String>,
    pub vault_abs_path: Option<String>,
    pub claude_agent_name: Option<String>,
    pub updated_at: Option<String>,
}

/// Возвращает системный промпт поста и метаданные Vault. Используется UI
/// «PostsEditor» при открытии формы редактирования.
#[tauri::command]
pub async fn get_post_knowledge(
    slug: String,
    db: State<'_, WritePool>,
    vault: State<'_, crate::vault::VaultState>,
) -> Result<PostKnowledge, String> {
    let row: Option<(String, String, Option<String>, Option<String>, Option<String>, Option<String>)> =
        sqlx::query_as(
            "SELECT slug, title, system_prompt_md, vault_subdir, claude_agent_name, updated_at
             FROM posts WHERE slug = ?",
        )
        .bind(&slug)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("get_post_knowledge: {e}"))?;

    let (slug, title, system_prompt_md, vault_subdir, claude_agent_name, updated_at) =
        row.ok_or_else(|| format!("post '{slug}' not found"))?;

    // Абсолютный путь к папке поста (если sub_dir известен — иначе считаем по slug).
    let pvr = crate::vault::post_vault_root(&vault.root, &slug).ok();
    let vault_abs_path = pvr.map(|p| p.display().to_string());

    Ok(PostKnowledge {
        slug,
        title,
        system_prompt_md,
        vault_subdir,
        vault_abs_path,
        claude_agent_name,
        updated_at,
    })
}

#[derive(Debug, Deserialize)]
pub struct UpdatePostKnowledgeInput {
    /// slug поста (ключ поиска)
    pub slug: String,
    /// Новый текст системного промпта. Null / пустая строка = очистить.
    pub system_prompt_md: Option<String>,
}

/// Сохраняет system_prompt_md поста + обеспечивает существование папки
/// `<vault>/posts/<slug>/{02-Patterns,04-Wins}` + записывает копию
/// промпта в `<vault>/posts/<slug>/CLAUDE.md` для будущего spawn-а агента.
#[tauri::command]
pub async fn update_post_knowledge(
    input: UpdatePostKnowledgeInput,
    db: State<'_, WritePool>,
    vault: State<'_, crate::vault::VaultState>,
) -> Result<PostKnowledge, String> {
    let slug = input.slug.trim().to_string();
    if slug.is_empty() {
        return Err("slug пустой".into());
    }

    let prompt = input
        .system_prompt_md
        .map(|s| s.trim_end().to_string())
        .filter(|s| !s.is_empty());

    if let Some(p) = &prompt {
        if p.len() > POST_SYSTEM_PROMPT_MAX_BYTES {
            return Err(format!(
                "system_prompt_md слишком большой: {} байт (макс {POST_SYSTEM_PROMPT_MAX_BYTES})",
                p.len()
            ));
        }
    }

    // Санитизация slug + резервирование claude_agent_name + vault_subdir.
    let safe_slug = crate::vault::sanitize_post_slug(&slug)
        .map_err(|e| format!("slug invalid: {e}"))?;
    let vault_subdir = format!("posts/{safe_slug}");
    let claude_agent_name = format!("mspro-{safe_slug}");

    // Обеспечиваем папку поста — даже если промпт NULL, чтоб UI «Открыть в проводнике» работало.
    crate::vault::ensure_post_vault_dirs(&vault.root, &safe_slug)?;

    // Записываем CLAUDE.md внутри папки поста (если промпт задан).
    if let Some(p) = &prompt {
        let pvr = crate::vault::post_vault_root(&vault.root, &safe_slug)?;
        let claude_md = pvr.join("CLAUDE.md");
        std::fs::write(&claude_md, p)
            .map_err(|e| format!("write CLAUDE.md: {e}"))?;
        log::info!("post knowledge: wrote {} ({} bytes)", claude_md.display(), p.len());
    }

    // UPDATE в SQLite — обновляем поле + ставим updated_at = now.
    let res = sqlx::query(
        "UPDATE posts SET system_prompt_md = ?,
                          vault_subdir = ?,
                          claude_agent_name = ?,
                          updated_at = datetime('now')
         WHERE slug = ?",
    )
    .bind(&prompt)
    .bind(&vault_subdir)
    .bind(&claude_agent_name)
    .bind(&slug)
    .execute(&db.0)
    .await
    .map_err(|e| format!("update post knowledge: {e}"))?;

    if res.rows_affected() == 0 {
        return Err(format!("post '{slug}' not found"));
    }

    // Перечитываем
    get_post_knowledge(slug, db, vault).await
}

#[derive(Debug, Deserialize)]
pub struct ImportPostVaultInput {
    pub slug: String,
    pub src_path: String,
}

#[derive(Debug, Serialize)]
pub struct ImportPostVaultResult {
    pub copied: usize,
    pub vault_abs_path: String,
}

/// Открывает указанный путь в Windows Explorer. Используется кнопкой
/// «📂 Открыть в проводнике» в UI знаний поста. Безопасно: проверяем что
/// путь существует и находится внутри `<vault>` (нельзя открыть произвольную
/// директорию системы через эту команду).
#[tauri::command]
pub async fn open_post_vault_in_explorer(
    slug: String,
    vault: State<'_, crate::vault::VaultState>,
) -> Result<(), String> {
    let safe_slug = crate::vault::sanitize_post_slug(&slug)
        .map_err(|e| format!("slug invalid: {e}"))?;
    let pvr = crate::vault::ensure_post_vault_dirs(&vault.root, &safe_slug)?;
    // Проверка что путь внутри Vault root (защита от race-conditions).
    let canon_root = vault
        .root
        .canonicalize()
        .map_err(|e| format!("canon vault root: {e}"))?;
    let canon_pvr = pvr.canonicalize().map_err(|e| format!("canon pvr: {e}"))?;
    if !canon_pvr.starts_with(&canon_root) {
        return Err("path escapes Vault root".into());
    }
    // Запускаем проводник без CREATE_NO_WINDOW — пользователь хочет видеть окно.
    std::process::Command::new("explorer")
        .arg(&canon_pvr)
        .spawn()
        .map_err(|e| format!("spawn explorer: {e}"))?;
    Ok(())
}

/// Копирует все `.md` из указанной Владельцем папки в `<vault>/posts/<slug>/`.
/// Симлинки и не-md файлы игнорируются. Не более 500 файлов за раз.
#[tauri::command]
pub async fn import_post_vault(
    input: ImportPostVaultInput,
    vault: State<'_, crate::vault::VaultState>,
) -> Result<ImportPostVaultResult, String> {
    let src = std::path::PathBuf::from(&input.src_path);
    if !src.exists() {
        return Err(format!("папка не найдена: {}", input.src_path));
    }
    let safe_slug = crate::vault::sanitize_post_slug(&input.slug)
        .map_err(|e| format!("slug invalid: {e}"))?;

    // Запускаем в blocking-пуле — fs может быть медленным на больших папках.
    let root = vault.root.clone();
    let slug = safe_slug.clone();
    let src_clone = src.clone();
    let copied = tokio::task::spawn_blocking(move || {
        crate::vault::import_folder_to_post(&root, &slug, &src_clone)
    })
    .await
    .map_err(|e| format!("join: {e}"))??;

    let pvr = crate::vault::post_vault_root(&vault.root, &safe_slug)?;
    Ok(ImportPostVaultResult {
        copied,
        vault_abs_path: pvr.display().to_string(),
    })
}
