//! Единый резолвер исполнителя — определяет «кто и как» будет выполнять задачу.
//!
//! Phase B: resolve_executor ищет сначала в org_agents, потом fallback в posts.
//! OrgAgent имеет приоритет при коллизии slug'ов.

use serde::Serialize;

use crate::db::WritePool;
use crate::settings::AppSettings;
use crate::vault::sanitize_post_slug;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum ExecutorKind {
    Post,
    OrgAgent,
}

#[derive(Debug, Clone)]
pub struct ExecutorSpec {
    pub slug: String,
    pub kind: ExecutorKind,
    pub entity_id: String,
    pub system_prompt: String,
    pub model: String,
    pub brain_mode: String,
    pub agent_md_name: String,
    pub agent_folder_path: Option<String>,
}

/// Резолвит исполнителя по slug: сначала org_agents (primary), потом posts (legacy fallback).
pub async fn resolve_executor(
    db: &WritePool,
    slug: &str,
    settings: &AppSettings,
) -> Result<ExecutorSpec, String> {
    // 1. org_agents WHERE slug = ? AND status = 'active' (PRIMARY)
    let agent_row: Option<(String, String, Option<String>, Option<String>, String, Option<String>)> = sqlx::query_as(
        "SELECT id, slug, role_prompt_md, brain_model, brain_mode, folder_path \
         FROM org_agents WHERE slug = ? AND status = 'active'",
    )
    .bind(slug)
    .fetch_optional(&db.0)
    .await
    .map_err(|e| format!("org_agents lookup: {e}"))?;

    if let Some((agent_id, agent_slug, role_prompt_opt, brain_model_opt, brain_mode, folder_path)) = agent_row {
        let system_prompt = role_prompt_opt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| format!("у агента '{agent_slug}' нет роли (CLAUDE.md)"))?;

        let disk_slug = crate::org_tree::to_disk_slug(&agent_slug);

        let model = brain_model_opt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .unwrap_or_else(|| settings.claude_cli_model.clone());

        return Ok(ExecutorSpec {
            slug: agent_slug,
            kind: ExecutorKind::OrgAgent,
            entity_id: agent_id,
            system_prompt: system_prompt.to_string(),
            model,
            brain_mode,
            agent_md_name: format!("mspro-org-{}", disk_slug),
            agent_folder_path: folder_path,
        });
    }

    // 2. posts WHERE slug = ? AND status = 'active' (LEGACY FALLBACK)
    // posts ретайрятся в витке 1: после retire этот путь всегда возвращает None. Полное удаление posts-runtime + posts.rs CRUD + фронт — виток 2.
    let post_row: Option<(String, String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT id, slug, system_prompt_md, preferred_model FROM posts WHERE slug = ? AND status = 'active'")
            .bind(slug)
            .fetch_optional(&db.0)
            .await
            .map_err(|e| format!("posts lookup: {e}"))?;

    if let Some((post_id, post_slug, system_prompt_opt, preferred_model_opt)) = post_row {
        let system_prompt = system_prompt_opt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                format!(
                    "post '{post_slug}' не имеет system_prompt_md — задай его в Posts Editor (🧠)"
                )
            })?;

        let safe_slug =
            sanitize_post_slug(&post_slug).map_err(|e| format!("slug invalid: {e}"))?;

        let model = preferred_model_opt
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty() && !s.to_lowercase().starts_with("qwen"))
            .map(|s| s.to_string())
            .unwrap_or_else(|| settings.claude_cli_model.clone());

        return Ok(ExecutorSpec {
            slug: post_slug,
            kind: ExecutorKind::Post,
            entity_id: post_id,
            system_prompt: system_prompt.to_string(),
            model,
            brain_mode: "claude_cli".to_string(),
            agent_md_name: format!("mspro-{}", safe_slug),
            agent_folder_path: None,
        });
    }

    // Подсчёт для информативного сообщения
    let active_posts: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM posts WHERE status = 'active'")
        .fetch_one(&db.0)
        .await
        .unwrap_or((0,));
    let active_agents: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM org_agents WHERE status = 'active'")
        .fetch_one(&db.0)
        .await
        .unwrap_or((0,));
    Err(format!(
        "исполнитель не найден: {slug}. Доступных исполнителей: {} (постов: {}, агентов: {})",
        active_posts.0 + active_agents.0,
        active_posts.0,
        active_agents.0
    ))
}

/// Резолвит org_agent по id (не slug) — для команды `run_org_agent_now`.
pub async fn resolve_org_agent_by_id(
    db: &WritePool,
    agent_id: &str,
    settings: &AppSettings,
) -> Result<ExecutorSpec, String> {
    let row: Option<(String, String, Option<String>, Option<String>, String, String, Option<String>)> = sqlx::query_as(
        "SELECT id, slug, role_prompt_md, brain_model, brain_mode, status, folder_path \
         FROM org_agents WHERE id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&db.0)
    .await
    .map_err(|e| format!("org_agent lookup: {e}"))?;

    let (db_id, agent_slug, role_prompt_opt, brain_model_opt, brain_mode, status, folder_path) =
        row.ok_or_else(|| format!("агент не найден: {agent_id}"))?;

    if status != "active" {
        return Err(format!(
            "агент '{agent_slug}' не активен (status='{status}')"
        ));
    }

    let system_prompt = role_prompt_opt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "у агента нет роли (CLAUDE.md)".to_string())?;

    let disk_slug = crate::org_tree::to_disk_slug(&agent_slug);

    let model = brain_model_opt
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| settings.claude_cli_model.clone());

    Ok(ExecutorSpec {
        slug: agent_slug,
        kind: ExecutorKind::OrgAgent,
        entity_id: db_id,
        system_prompt: system_prompt.to_string(),
        model,
        brain_mode,
        agent_md_name: format!("mspro-org-{}", disk_slug),
        agent_folder_path: folder_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn setup_db() -> WritePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE posts (
                id TEXT PRIMARY KEY,
                department_id TEXT NOT NULL DEFAULT 'd1',
                slug TEXT NOT NULL UNIQUE,
                title TEXT NOT NULL,
                central_product TEXT NOT NULL DEFAULT '',
                system_prompt_md TEXT,
                preferred_model TEXT,
                status TEXT NOT NULL DEFAULT 'active',
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP
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
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        WritePool(pool)
    }

    #[tokio::test]
    async fn resolve_post_found() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO posts (id, title, slug, system_prompt_md, preferred_model) \
             VALUES ('p1', 'Frontend', 'frontend', 'You are a frontend developer', 'opus')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let spec = resolve_executor(&db, "frontend", &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(spec.kind, ExecutorKind::Post);
        assert_eq!(spec.slug, "frontend");
        assert_eq!(spec.entity_id, "p1");
        assert_eq!(spec.system_prompt, "You are a frontend developer");
        assert_eq!(spec.agent_md_name, "mspro-frontend");
        assert_eq!(spec.brain_mode, "claude_cli");
    }

    #[tokio::test]
    async fn resolve_org_agent_active() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO org_agents (id, department_id, name, slug, status, \
             role_prompt_md, brain_mode, brain_model) \
             VALUES ('a1', 'd1', 'Programmer', 'programmer', 'active', \
             'You are a Rust developer', 'claude_cli', 'opus')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let spec = resolve_executor(&db, "programmer", &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(spec.kind, ExecutorKind::OrgAgent);
        assert_eq!(spec.slug, "programmer");
        assert_eq!(spec.entity_id, "a1");
        assert_eq!(spec.system_prompt, "You are a Rust developer");
        assert_eq!(spec.agent_md_name, "mspro-org-programmer");
        assert_eq!(spec.brain_mode, "claude_cli");
    }

    #[tokio::test]
    async fn resolve_not_found() {
        let db = setup_db().await;
        let result = resolve_executor(&db, "nonexistent", &AppSettings::default()).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains("исполнитель не найден"));
        assert!(err.contains("Доступных исполнителей"));
    }

    #[tokio::test]
    async fn resolve_post_archived_skipped() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO posts (id, title, slug, system_prompt_md, status) \
             VALUES ('p1', 'Archived', 'archived-post', 'Old role', 'archived')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let result = resolve_executor(&db, "archived-post", &AppSettings::default()).await;
        assert!(result.is_err(), "archived post should not resolve");
    }

    #[tokio::test]
    async fn resolve_org_agent_inactive_skipped() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO org_agents (id, department_id, name, slug, status, \
             role_prompt_md, brain_mode) \
             VALUES ('a1', 'd1', 'Agent', 'myagent', 'paused', 'Some role', 'claude_cli')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let result = resolve_executor(&db, "myagent", &AppSettings::default()).await;
        assert!(
            result.is_err(),
            "paused agent should not resolve as OrgAgent"
        );
    }

    #[tokio::test]
    async fn resolve_org_agent_preferred_over_post() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO posts (id, title, slug, system_prompt_md) \
             VALUES ('p1', 'Test', 'shared-slug', 'Post role')",
        )
        .execute(&db.0)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO org_agents (id, department_id, name, slug, status, \
             role_prompt_md, brain_mode) \
             VALUES ('a1', 'd1', 'Agent', 'shared-slug', 'active', 'Agent role', 'claude_cli')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let spec = resolve_executor(&db, "shared-slug", &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(
            spec.kind,
            ExecutorKind::OrgAgent,
            "org_agent should take priority over post (Phase B)"
        );
    }

    #[tokio::test]
    async fn resolve_post_fallback_when_no_org_agent() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO posts (id, title, slug, system_prompt_md, preferred_model) \
             VALUES ('p1', 'Frontend', 'frontend', 'You are frontend dev', 'opus')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let spec = resolve_executor(&db, "frontend", &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(spec.kind, ExecutorKind::Post, "post resolves as legacy fallback");
        assert_eq!(spec.slug, "frontend");
    }

    #[tokio::test]
    async fn resolve_by_id_found() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO org_agents (id, department_id, name, slug, status, \
             role_prompt_md, brain_mode, brain_model) \
             VALUES ('agent-abc123', 'd1', 'DevOps', 'devops', 'active', \
             'DevOps role', 'claude_cli', 'sonnet')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let spec = resolve_org_agent_by_id(&db, "agent-abc123", &AppSettings::default())
            .await
            .unwrap();
        assert_eq!(spec.kind, ExecutorKind::OrgAgent);
        assert_eq!(spec.entity_id, "agent-abc123");
        assert_eq!(spec.slug, "devops");
        assert_eq!(spec.brain_mode, "claude_cli");
        assert_eq!(spec.model, "sonnet");
    }

    #[tokio::test]
    async fn resolve_by_id_not_found() {
        let db = setup_db().await;
        let result =
            resolve_org_agent_by_id(&db, "nonexistent", &AppSettings::default()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("агент не найден"));
    }

    #[tokio::test]
    async fn resolve_by_id_inactive_rejected() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO org_agents (id, department_id, name, slug, status, \
             role_prompt_md, brain_mode) \
             VALUES ('a1', 'd1', 'Off Agent', 'offagent', 'off', 'Role', 'claude_cli')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let result =
            resolve_org_agent_by_id(&db, "a1", &AppSettings::default()).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("не активен"));
    }

    #[tokio::test]
    async fn resolve_post_model_fallback_to_settings() {
        let db = setup_db().await;
        sqlx::query(
            "INSERT INTO posts (id, title, slug, system_prompt_md) \
             VALUES ('p1', 'Test', 'test', 'Test role')",
        )
        .execute(&db.0)
        .await
        .unwrap();

        let settings = AppSettings::default();
        let spec = resolve_executor(&db, "test", &settings).await.unwrap();
        assert_eq!(spec.model, settings.claude_cli_model);
    }
}
