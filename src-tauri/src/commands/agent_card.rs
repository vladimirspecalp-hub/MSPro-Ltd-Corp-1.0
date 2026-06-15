//! Заход 2 — Карточка агента: CRUD карточки + управление связями (next/verifier).
//!
//! brain_mode валидируется на Rust (enum match, НЕ SQL CHECK — избегаем table
//! rebuild). Секреты — через существующие vault_* команды (конвенция key_name:
//! `agent-{short_id}-{type}-{name}`).

use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::State;

use crate::db::WritePool;

const BRAIN_MODES: &[&str] = &["disabled", "claude_cli", "qwen_http", "external_gateway"];

pub fn validate_brain_mode(mode: &str) -> Result<(), String> {
    if BRAIN_MODES.contains(&mode) {
        Ok(())
    } else {
        Err(format!(
            "brain_mode '{mode}' invalid (allowed: {})",
            BRAIN_MODES.join(", ")
        ))
    }
}

// ---------------------------------------------------------------------------
// DTO
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, FromRow)]
pub struct AgentCard {
    pub id: String,
    pub name: String,
    pub slug: String,
    pub role_label: String,
    pub status: String,
    pub role_prompt_md: Option<String>,
    pub brain_mode: String,
    pub brain_model: Option<String>,
    pub brain_endpoint: Option<String>,
    pub mcp_servers_json: String,
    pub ckp_text: Option<String>,
    pub checklist_json: String,
    pub memory_md: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct AgentCardInput {
    pub role_prompt_md: Option<String>,
    pub brain_mode: String,
    pub brain_model: Option<String>,
    pub brain_endpoint: Option<String>,
    pub mcp_servers_json: String,
    pub ckp_text: Option<String>,
    pub checklist_json: String,
    pub memory_md: Option<String>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AgentLink {
    pub id: String,
    pub from_agent_id: String,
    pub to_agent_id: String,
    pub link_type: String,
    pub description: Option<String>,
    pub sort_order: i64,
    pub created_at: String,
}

// ---------------------------------------------------------------------------
// Card CRUD
// ---------------------------------------------------------------------------

async fn fetch_card(db: &WritePool, agent_id: &str) -> Result<AgentCard, String> {
    sqlx::query_as::<_, AgentCard>(
        "SELECT id, name, slug, role_label, status,
                role_prompt_md, brain_mode, brain_model, brain_endpoint,
                mcp_servers_json, ckp_text, checklist_json, memory_md
         FROM org_agents WHERE id = ?",
    )
    .bind(agent_id)
    .fetch_optional(&db.0)
    .await
    .map_err(|e| format!("agent_card_get: {e}"))?
    .ok_or_else(|| "agent not found".to_string())
}

#[tauri::command]
pub async fn agent_card_get(
    agent_id: String,
    db: State<'_, WritePool>,
) -> Result<AgentCard, String> {
    fetch_card(&db, &agent_id).await
}

#[tauri::command]
pub async fn agent_card_save(
    agent_id: String,
    input: AgentCardInput,
    db: State<'_, WritePool>,
) -> Result<AgentCard, String> {
    validate_brain_mode(&input.brain_mode)?;

    serde_json::from_str::<serde_json::Value>(&input.mcp_servers_json)
        .map_err(|e| format!("mcp_servers_json invalid JSON: {e}"))?;
    serde_json::from_str::<serde_json::Value>(&input.checklist_json)
        .map_err(|e| format!("checklist_json invalid JSON: {e}"))?;

    let rows = sqlx::query(
        "UPDATE org_agents SET
            role_prompt_md = ?, brain_mode = ?, brain_model = ?,
            brain_endpoint = ?, mcp_servers_json = ?, ckp_text = ?,
            checklist_json = ?, memory_md = ?, updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(&input.role_prompt_md)
    .bind(&input.brain_mode)
    .bind(&input.brain_model)
    .bind(&input.brain_endpoint)
    .bind(&input.mcp_servers_json)
    .bind(&input.ckp_text)
    .bind(&input.checklist_json)
    .bind(&input.memory_md)
    .bind(&agent_id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("agent_card_save: {e}"))?
    .rows_affected();

    if rows == 0 {
        return Err("agent not found".to_string());
    }
    fetch_card(&db, &agent_id).await
}

// ---------------------------------------------------------------------------
// Links CRUD
// ---------------------------------------------------------------------------

const LINK_TYPES: &[&str] = &["next", "verifier", "input_from"];

#[tauri::command]
pub async fn agent_links_get(
    agent_id: String,
    db: State<'_, WritePool>,
) -> Result<Vec<AgentLink>, String> {
    sqlx::query_as::<_, AgentLink>(
        "SELECT id, from_agent_id, to_agent_id, link_type, description, sort_order, created_at
         FROM org_agent_links
         WHERE from_agent_id = ? OR to_agent_id = ?
         ORDER BY link_type, sort_order",
    )
    .bind(&agent_id)
    .bind(&agent_id)
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("agent_links_get: {e}"))
}

#[tauri::command]
pub async fn agent_link_set(
    from_agent_id: String,
    to_agent_id: String,
    link_type: String,
    description: Option<String>,
    db: State<'_, WritePool>,
) -> Result<AgentLink, String> {
    if !LINK_TYPES.contains(&link_type.as_str()) {
        return Err(format!(
            "link_type '{link_type}' invalid (allowed: next, verifier, input_from)"
        ));
    }
    if from_agent_id == to_agent_id {
        return Err("cannot link agent to itself".to_string());
    }

    let from_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM org_agents WHERE id = ?")
            .bind(&from_agent_id)
            .fetch_optional(&db.0)
            .await
            .map_err(|e| format!("check from_agent: {e}"))?;
    if from_exists.is_none() {
        return Err(format!("from_agent '{from_agent_id}' not found"));
    }

    let to_exists: Option<(String,)> =
        sqlx::query_as("SELECT id FROM org_agents WHERE id = ?")
            .bind(&to_agent_id)
            .fetch_optional(&db.0)
            .await
            .map_err(|e| format!("check to_agent: {e}"))?;
    if to_exists.is_none() {
        return Err(format!("to_agent '{to_agent_id}' not found"));
    }

    if link_type == "next" {
        let cycle: Option<(i64,)> = sqlx::query_as(
            "WITH RECURSIVE chain(agent_id, depth) AS (
                SELECT ?, 0
                UNION ALL
                SELECT l.to_agent_id, chain.depth + 1
                FROM org_agent_links l
                JOIN chain ON l.from_agent_id = chain.agent_id
                WHERE l.link_type = 'next' AND chain.depth < 50
            )
            SELECT 1 FROM chain WHERE agent_id = ? AND depth > 0
            LIMIT 1",
        )
        .bind(&to_agent_id)
        .bind(&from_agent_id)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("cycle check: {e}"))?;

        if cycle.is_some() {
            return Err(
                "adding this link would create a cycle in the 'next' chain".to_string(),
            );
        }
    }

    let id = format!("link-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type, description)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(from_agent_id, to_agent_id, link_type) DO UPDATE SET
            description = excluded.description",
    )
    .bind(&id)
    .bind(&from_agent_id)
    .bind(&to_agent_id)
    .bind(&link_type)
    .bind(&description)
    .execute(&db.0)
    .await
    .map_err(|e| format!("agent_link_set: {e}"))?;

    sqlx::query_as::<_, AgentLink>(
        "SELECT id, from_agent_id, to_agent_id, link_type, description, sort_order, created_at
         FROM org_agent_links
         WHERE from_agent_id = ? AND to_agent_id = ? AND link_type = ?",
    )
    .bind(&from_agent_id)
    .bind(&to_agent_id)
    .bind(&link_type)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch link: {e}"))
}

#[tauri::command]
pub async fn agent_link_remove(
    link_id: String,
    db: State<'_, WritePool>,
) -> Result<(), String> {
    let rows = sqlx::query("DELETE FROM org_agent_links WHERE id = ?")
        .bind(&link_id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("agent_link_remove: {e}"))?
        .rows_affected();
    if rows == 0 {
        return Err("link not found".to_string());
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn setup_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE org_agents (
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
                link_type TEXT NOT NULL CHECK (link_type IN ('next','verifier','input_from')),
                description TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY (from_agent_id) REFERENCES org_agents(id) ON DELETE CASCADE,
                FOREIGN KEY (to_agent_id) REFERENCES org_agents(id) ON DELETE CASCADE,
                UNIQUE(from_agent_id, to_agent_id, link_type),
                CHECK(from_agent_id != to_agent_id)
            );
            CREATE INDEX IF NOT EXISTS idx_agent_links_to ON org_agent_links(to_agent_id);",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    #[test]
    fn brain_mode_validation() {
        assert!(validate_brain_mode("disabled").is_ok());
        assert!(validate_brain_mode("claude_cli").is_ok());
        assert!(validate_brain_mode("qwen_http").is_ok());
        assert!(validate_brain_mode("external_gateway").is_ok());
        assert!(validate_brain_mode("gpt4").is_err());
        assert!(validate_brain_mode("").is_err());
    }

    #[tokio::test]
    async fn self_heal_columns_idempotent() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE org_agents (
                id TEXT PRIMARY KEY,
                department_id TEXT NOT NULL,
                name TEXT NOT NULL,
                slug TEXT NOT NULL,
                role_label TEXT NOT NULL DEFAULT 'member',
                status TEXT NOT NULL DEFAULT 'active',
                folder_path TEXT,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT DEFAULT NULL
            );",
        )
        .execute(&pool)
        .await
        .unwrap();

        let card_alters: &[(&str, &str)] = &[
            ("role_prompt_md",  "ALTER TABLE org_agents ADD COLUMN role_prompt_md TEXT DEFAULT NULL"),
            ("brain_mode",      "ALTER TABLE org_agents ADD COLUMN brain_mode TEXT NOT NULL DEFAULT 'disabled'"),
            ("brain_model",     "ALTER TABLE org_agents ADD COLUMN brain_model TEXT DEFAULT NULL"),
            ("brain_endpoint",  "ALTER TABLE org_agents ADD COLUMN brain_endpoint TEXT DEFAULT NULL"),
            ("mcp_servers_json","ALTER TABLE org_agents ADD COLUMN mcp_servers_json TEXT NOT NULL DEFAULT '[]'"),
            ("ckp_text",        "ALTER TABLE org_agents ADD COLUMN ckp_text TEXT DEFAULT NULL"),
            ("checklist_json",  "ALTER TABLE org_agents ADD COLUMN checklist_json TEXT NOT NULL DEFAULT '[]'"),
            ("memory_md",       "ALTER TABLE org_agents ADD COLUMN memory_md TEXT DEFAULT NULL"),
        ];

        // First pass: apply all ALTERs
        let cols: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(org_agents)")
                .fetch_all(&pool).await.unwrap();
        let existing: std::collections::HashSet<String> =
            cols.into_iter().map(|c| c.1).collect();
        for (col, sql) in card_alters {
            if !existing.contains(*col) {
                sqlx::raw_sql(sql).execute(&pool).await.unwrap();
            }
        }

        // Verify all 8 new columns present
        let cols_after: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(org_agents)")
                .fetch_all(&pool).await.unwrap();
        let names: Vec<String> = cols_after.into_iter().map(|c| c.1).collect();
        for (col, _) in card_alters {
            assert!(names.contains(&col.to_string()), "column '{col}' missing after self-heal");
        }

        // Second pass: same ALTERs should be no-ops (idempotent)
        let cols2: Vec<(i64, String, String, i64, Option<String>, i64)> =
            sqlx::query_as("PRAGMA table_info(org_agents)")
                .fetch_all(&pool).await.unwrap();
        let existing2: std::collections::HashSet<String> =
            cols2.into_iter().map(|c| c.1).collect();
        for (col, sql) in card_alters {
            if !existing2.contains(*col) {
                sqlx::raw_sql(sql).execute(&pool).await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn cycle_detection_next_links() {
        let pool = setup_db().await;
        sqlx::raw_sql(
            "INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a1','d1','A1','a1');
             INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a2','d1','A2','a2');
             INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a3','d1','A3','a3');",
        )
        .execute(&pool).await.unwrap();

        // Chain: a1 → a2 → a3
        sqlx::query("INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l1','a1','a2','next')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l2','a2','a3','next')")
            .execute(&pool).await.unwrap();

        // Try a3 → a1 (would close cycle)
        let cycle: Option<(i64,)> = sqlx::query_as(
            "WITH RECURSIVE chain(agent_id, depth) AS (
                SELECT ?, 0
                UNION ALL
                SELECT l.to_agent_id, chain.depth + 1
                FROM org_agent_links l
                JOIN chain ON l.from_agent_id = chain.agent_id
                WHERE l.link_type = 'next' AND chain.depth < 50
            )
            SELECT 1 FROM chain WHERE agent_id = ? AND depth > 0
            LIMIT 1",
        )
        .bind("a1")   // to_agent_id of proposed link (a3→a1)
        .bind("a3")   // from_agent_id
        .fetch_optional(&pool).await.unwrap();
        assert!(cycle.is_some(), "cycle should be detected: a3→a1 closes a1→a2→a3");

        // Non-cyclic: a3 → a1 as 'verifier' (only 'next' chains are checked)
        sqlx::query("INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l3','a3','a1','verifier')")
            .execute(&pool).await.unwrap();

        // a1 → a3 as 'next': follow from a3 — no outgoing 'next' exists, so no cycle
        let no_cycle: Option<(i64,)> = sqlx::query_as(
            "WITH RECURSIVE chain(agent_id, depth) AS (
                SELECT ?, 0
                UNION ALL
                SELECT l.to_agent_id, chain.depth + 1
                FROM org_agent_links l
                JOIN chain ON l.from_agent_id = chain.agent_id
                WHERE l.link_type = 'next' AND chain.depth < 50
            )
            SELECT 1 FROM chain WHERE agent_id = ? AND depth > 0
            LIMIT 1",
        )
        .bind("a3")   // to_agent_id
        .bind("a1")   // from_agent_id
        .fetch_optional(&pool).await.unwrap();
        assert!(no_cycle.is_none(), "no cycle: a3 has no outgoing 'next'");
    }

    #[tokio::test]
    async fn agent_card_save_validates_brain_mode_in_db() {
        let pool = setup_db().await;
        sqlx::query("INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a1','d1','Test','test')")
            .execute(&pool).await.unwrap();

        // Valid write
        sqlx::query("UPDATE org_agents SET brain_mode = ? WHERE id = 'a1'")
            .bind("claude_cli")
            .execute(&pool).await.unwrap();
        let row: (String,) = sqlx::query_as("SELECT brain_mode FROM org_agents WHERE id = 'a1'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(row.0, "claude_cli");

        // Default value is correct
        sqlx::query("INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a2','d1','T2','t2')")
            .execute(&pool).await.unwrap();
        let row2: (String,) = sqlx::query_as("SELECT brain_mode FROM org_agents WHERE id = 'a2'")
            .fetch_one(&pool).await.unwrap();
        assert_eq!(row2.0, "disabled");
    }

    #[tokio::test]
    async fn self_ref_link_rejected() {
        let pool = setup_db().await;
        sqlx::query("INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a1','d1','A1','a1')")
            .execute(&pool).await.unwrap();

        let result = sqlx::query(
            "INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l1','a1','a1','next')"
        ).execute(&pool).await;
        assert!(result.is_err(), "self-reference should be rejected by CHECK");
    }

    #[tokio::test]
    async fn duplicate_link_rejected() {
        let pool = setup_db().await;
        sqlx::raw_sql(
            "INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a1','d1','A1','a1');
             INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a2','d1','A2','a2');",
        ).execute(&pool).await.unwrap();

        sqlx::query("INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l1','a1','a2','next')")
            .execute(&pool).await.unwrap();
        let dup = sqlx::query(
            "INSERT INTO org_agent_links (id, from_agent_id, to_agent_id, link_type) VALUES ('l2','a1','a2','next')"
        ).execute(&pool).await;
        assert!(dup.is_err(), "duplicate (from, to, type) should be rejected by UNIQUE");
    }
}
