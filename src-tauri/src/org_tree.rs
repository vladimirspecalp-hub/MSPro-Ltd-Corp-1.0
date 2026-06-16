//! Заход 3 — Материализация оргструктуры на диск.
//!
//! Корень: `C:\CODE\Agents\`. БД = master, диск = derived.
//! Каждый CRUD в org_chart / agent_card вызывает `try_sync_*` для обновления
//! диска. Ошибки диска → `log::warn`, НЕ откат SQL (BEST-EFFORT).
//!
//! Защита ручных правок: SHA-256 последнего записанного контента хранится в
//! таблице `org_disk_sync`. Если хэш на диске расходится с записанным — файл
//! был отредактирован вручную и НЕ перезатирается (кроме force_rebuild).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use tauri::State;
use tokio::sync::Mutex as TokioMutex;

use crate::db::WritePool;

// =========================================================================
// State
// =========================================================================

pub const ORG_TREE_ROOT: &str = r"C:\CODE\Agents";

pub struct OrgTreeState {
    pub root: PathBuf,
    locks: std::sync::Mutex<HashMap<String, Arc<TokioMutex<()>>>>,
}

impl OrgTreeState {
    pub fn new(root: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&root) {
            log::warn!("org_tree: cannot create root {}: {e}", root.display());
        }
        Self {
            root,
            locks: std::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn entity_lock(&self, id: &str) -> Arc<TokioMutex<()>> {
        self.locks
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert_with(|| Arc::new(TokioMutex::new(())))
            .clone()
    }
}

// =========================================================================
// Cyrillic → Latin transliteration + disk-safe slug
// =========================================================================

pub fn to_disk_slug(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let mut out = String::with_capacity(lowered.len() * 2);

    for c in lowered.chars() {
        match c {
            'а' => out.push('a'),
            'б' => out.push('b'),
            'в' => out.push('v'),
            'г' => out.push('g'),
            'д' => out.push('d'),
            'е' => out.push('e'),
            'ё' => out.push_str("yo"),
            'ж' => out.push_str("zh"),
            'з' => out.push('z'),
            'и' => out.push('i'),
            'й' => out.push('y'),
            'к' => out.push('k'),
            'л' => out.push('l'),
            'м' => out.push('m'),
            'н' => out.push('n'),
            'о' => out.push('o'),
            'п' => out.push('p'),
            'р' => out.push('r'),
            'с' => out.push('s'),
            'т' => out.push('t'),
            'у' => out.push('u'),
            'ф' => out.push('f'),
            'х' => out.push_str("kh"),
            'ц' => out.push_str("ts"),
            'ч' => out.push_str("ch"),
            'ш' => out.push_str("sh"),
            'щ' => out.push_str("shch"),
            'ъ' | 'ь' => {}
            'ы' => out.push('y'),
            'э' => out.push('e'),
            'ю' => out.push_str("yu"),
            'я' => out.push_str("ya"),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => out.push(c),
            _ => out.push('-'),
        }
    }

    // Collapse consecutive dashes
    let mut result = String::with_capacity(out.len());
    let mut prev_dash = false;
    for c in out.chars() {
        if c == '-' {
            if !prev_dash {
                result.push('-');
            }
            prev_dash = true;
        } else {
            result.push(c);
            prev_dash = false;
        }
    }
    let result = result.trim_matches('-');
    let result: String = result.chars().take(64).collect();
    if result.is_empty() {
        "entity".to_string()
    } else {
        result
    }
}

// =========================================================================
// Slug dedup (UNIQUE within table)
// =========================================================================

pub async fn dedup_slug_in_table(
    pool: &SqlitePool,
    table: &str,
    base: &str,
    exclude_id: Option<&str>,
) -> Result<String, String> {
    if !matches!(table, "org_divisions" | "org_departments") {
        return Err("invalid table for slug dedup".into());
    }
    for i in 0..100 {
        let candidate = if i == 0 {
            base.to_string()
        } else {
            format!("{base}-{i}")
        };
        let exists: Option<(i64,)> = if let Some(eid) = exclude_id {
            sqlx::query_as(&format!(
                "SELECT 1 FROM {table} WHERE slug = ? AND id != ? LIMIT 1"
            ))
            .bind(&candidate)
            .bind(eid)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("dedup slug: {e}"))?
        } else {
            sqlx::query_as(&format!(
                "SELECT 1 FROM {table} WHERE slug = ? LIMIT 1"
            ))
            .bind(&candidate)
            .fetch_optional(pool)
            .await
            .map_err(|e| format!("dedup slug: {e}"))?
        };
        if exists.is_none() {
            return Ok(candidate);
        }
    }
    Err(format!("cannot deduplicate slug '{base}' in {table}"))
}

// =========================================================================
// Source guard: sanitize + canonicalize + starts_with + symlink reject
// =========================================================================

fn sanitize_component(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .collect()
}

pub fn safe_org_path(root: &Path, components: &[&str]) -> Result<PathBuf, String> {
    if components.is_empty() {
        return Err("empty components".into());
    }

    let mut path = root.to_path_buf();
    for comp in components {
        let sanitized = sanitize_component(comp);
        if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
            return Err(format!("invalid path component: '{comp}' → '{sanitized}'"));
        }
        path.push(&sanitized);
    }

    let root_canon =
        std::fs::canonicalize(root).map_err(|e| format!("canonicalize root: {e}"))?;

    let mut check = root.to_path_buf();
    for (i, comp) in components.iter().enumerate() {
        let sanitized = sanitize_component(comp);
        check.push(&sanitized);
        if check.exists() {
            if check
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
            {
                return Err(format!("symlink detected: {}", check.display()));
            }
            let canon = std::fs::canonicalize(&check)
                .map_err(|e| format!("canonicalize {}: {e}", check.display()))?;
            if !canon.starts_with(&root_canon) {
                return Err(format!(
                    "path escape: {} resolves outside root",
                    check.display()
                ));
            }
        } else if i == components.len() - 1 {
            // Leaf doesn't exist yet — canonicalize parent and verify leaf name
            let parent = check.parent().ok_or("no parent for leaf")?;
            if parent.exists() {
                let parent_canon = std::fs::canonicalize(parent)
                    .map_err(|e| format!("canonicalize parent: {e}"))?;
                if !parent_canon.starts_with(&root_canon) {
                    return Err(format!(
                        "path escape: parent {} resolves outside root",
                        parent.display()
                    ));
                }
            }
        }
    }

    Ok(path)
}

// =========================================================================
// SHA-256
// =========================================================================

fn sha256_hex(data: &[u8]) -> String {
    format!("{:x}", Sha256::digest(data))
}

// =========================================================================
// org_disk_sync DB operations
// =========================================================================

async fn get_stored_hash(pool: &SqlitePool, etype: &str, eid: &str, file: &str) -> Option<String> {
    sqlx::query_as::<_, (String,)>(
        "SELECT content_hash FROM org_disk_sync \
         WHERE entity_type = ? AND entity_id = ? AND file_rel = ?",
    )
    .bind(etype)
    .bind(eid)
    .bind(file)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten()
    .map(|r| r.0)
}

async fn set_stored_hash(pool: &SqlitePool, etype: &str, eid: &str, file: &str, hash: &str) {
    let _ = sqlx::query(
        "INSERT INTO org_disk_sync (entity_type, entity_id, file_rel, content_hash, written_at) \
         VALUES (?, ?, ?, ?, datetime('now')) \
         ON CONFLICT(entity_type, entity_id, file_rel) DO UPDATE SET \
            content_hash = excluded.content_hash, written_at = excluded.written_at",
    )
    .bind(etype)
    .bind(eid)
    .bind(file)
    .bind(hash)
    .execute(pool)
    .await;
}

pub async fn clean_sync_records(pool: &SqlitePool, etype: &str, eid: &str) {
    let _ = sqlx::query("DELETE FROM org_disk_sync WHERE entity_type = ? AND entity_id = ?")
        .bind(etype)
        .bind(eid)
        .execute(pool)
        .await;
}

// =========================================================================
// Protected file write (respects manual edits)
// =========================================================================

async fn write_protected(
    pool: &SqlitePool,
    file_path: &Path,
    etype: &str,
    eid: &str,
    file_name: &str,
    content: &[u8],
    force: bool,
) -> bool {
    let new_hash = sha256_hex(content);

    if !force && file_path.exists() {
        match std::fs::read(file_path) {
            Ok(disk_content) => {
                let disk_hash = sha256_hex(&disk_content);
                if let Some(stored_hash) = get_stored_hash(pool, etype, eid, file_name).await {
                    if disk_hash != stored_hash {
                        log::warn!(
                            "org_tree: {etype}/{eid}/{file_name} manually edited, skipping"
                        );
                        return false;
                    }
                    if disk_hash == new_hash {
                        return false; // unchanged
                    }
                } else {
                    // File exists but no stored hash — adopt it, don't overwrite
                    log::warn!(
                        "org_tree: {etype}/{eid}/{file_name} exists without stored hash, adopting"
                    );
                    set_stored_hash(pool, etype, eid, file_name, &disk_hash).await;
                    return false;
                }
            }
            Err(e) => {
                log::warn!("org_tree: read {}: {e}", file_path.display());
                return false;
            }
        }
    }

    match std::fs::write(file_path, content) {
        Ok(()) => {
            set_stored_hash(pool, etype, eid, file_name, &new_hash).await;
            true
        }
        Err(e) => {
            log::warn!("org_tree: write {}: {e}", file_path.display());
            false
        }
    }
}

async fn remove_if_null(
    pool: &SqlitePool,
    file_path: &Path,
    etype: &str,
    eid: &str,
    file_name: &str,
    force: bool,
) {
    if !file_path.exists() {
        return;
    }
    if !force {
        if let Some(stored_hash) = get_stored_hash(pool, etype, eid, file_name).await {
            if let Ok(disk) = std::fs::read(file_path) {
                if sha256_hex(&disk) != stored_hash {
                    log::warn!(
                        "org_tree: {etype}/{eid}/{file_name} manually edited, not removing"
                    );
                    return;
                }
            }
        }
    }
    let _ = std::fs::remove_file(file_path);
    let _ = sqlx::query(
        "DELETE FROM org_disk_sync \
         WHERE entity_type = ? AND entity_id = ? AND file_rel = ?",
    )
    .bind(etype)
    .bind(eid)
    .bind(file_name)
    .execute(pool)
    .await;
}

// =========================================================================
// Markdown / content formatters
// =========================================================================

fn format_division_md(name: &str, desc: Option<&str>) -> String {
    let mut s = format!("# {name}\n");
    if let Some(d) = desc {
        if !d.is_empty() {
            s.push_str(&format!("\n{d}\n"));
        }
    }
    s
}

fn format_department_md(name: &str, desc: Option<&str>) -> String {
    let mut s = format!("# {name}\n");
    if let Some(d) = desc {
        if !d.is_empty() {
            s.push_str(&format!("\n{d}\n"));
        }
    }
    s
}

fn is_trivial_json(s: &str) -> bool {
    let t = s.trim();
    t == "[]" || t == "{}" || t.is_empty()
}

fn is_blank(s: &Option<String>) -> bool {
    s.as_ref().map(|v| v.trim().is_empty()).unwrap_or(true)
}

// =========================================================================
// Sync: division
// =========================================================================

#[derive(sqlx::FromRow)]
struct DivSyncRow {
    id: String,
    name: String,
    slug: Option<String>,
    description: Option<String>,
}

async fn sync_division(pool: &SqlitePool, root: &Path, div_id: &str, force: bool) {
    let row: Option<DivSyncRow> = sqlx::query_as(
        "SELECT id, name, slug, description FROM org_divisions WHERE id = ?",
    )
    .bind(div_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some(div) = row else { return };
    let slug = match div.slug {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return,
    };

    let dir = match safe_org_path(root, &[slug]) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("org_tree: bad path for div {div_id}: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("org_tree: mkdir {}: {e}", dir.display());
        return;
    }

    let md = format_division_md(&div.name, div.description.as_deref());
    write_protected(
        pool,
        &dir.join("division.md"),
        "division",
        div_id,
        "division.md",
        md.as_bytes(),
        force,
    )
    .await;
}

// =========================================================================
// Sync: department
// =========================================================================

#[derive(sqlx::FromRow)]
struct DeptSyncRow {
    id: String,
    name: String,
    slug: Option<String>,
    description: Option<String>,
    div_slug: Option<String>,
}

async fn sync_department(pool: &SqlitePool, root: &Path, dept_id: &str, force: bool) {
    let row: Option<DeptSyncRow> = sqlx::query_as(
        "SELECT d.id, d.name, d.slug, d.description, div.slug AS div_slug \
         FROM org_departments d \
         JOIN org_divisions div ON d.division_id = div.id \
         WHERE d.id = ?",
    )
    .bind(dept_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some(dept) = row else { return };
    let div_slug = match dept.div_slug {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return,
    };
    let slug = match dept.slug {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return,
    };

    let dir = match safe_org_path(root, &[div_slug, slug]) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("org_tree: bad path for dept {dept_id}: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("org_tree: mkdir {}: {e}", dir.display());
        return;
    }

    let md = format_department_md(&dept.name, dept.description.as_deref());
    write_protected(
        pool,
        &dir.join("department.md"),
        "department",
        dept_id,
        "department.md",
        md.as_bytes(),
        force,
    )
    .await;
}

// =========================================================================
// Sync: agent (CLAUDE.md + .mcp.json + memory/ + checklist + ckp)
// =========================================================================

#[derive(sqlx::FromRow)]
struct AgentSyncRow {
    id: String,
    slug: String,
    #[allow(dead_code)]
    name: String,
    role_prompt_md: Option<String>,
    mcp_servers_json: String,
    memory_md: Option<String>,
    checklist_json: String,
    ckp_text: Option<String>,
    dept_slug: Option<String>,
    div_slug: Option<String>,
}

async fn sync_agent(pool: &SqlitePool, root: &Path, agent_id: &str, force: bool) {
    let row: Option<AgentSyncRow> = sqlx::query_as(
        "SELECT a.id, a.slug, a.name, \
                a.role_prompt_md, a.mcp_servers_json, a.memory_md, \
                a.checklist_json, a.ckp_text, \
                d.slug AS dept_slug, div.slug AS div_slug \
         FROM org_agents a \
         JOIN org_departments d ON a.department_id = d.id \
         JOIN org_divisions div ON d.division_id = div.id \
         WHERE a.id = ?",
    )
    .bind(agent_id)
    .fetch_optional(pool)
    .await
    .ok()
    .flatten();

    let Some(agent) = row else { return };
    let div_slug = match agent.div_slug {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return,
    };
    let dept_slug = match agent.dept_slug {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return,
    };
    let agent_disk_slug = to_disk_slug(&agent.slug);

    let dir = match safe_org_path(root, &[div_slug, dept_slug, &agent_disk_slug]) {
        Ok(d) => d,
        Err(e) => {
            log::warn!("org_tree: bad path for agent {agent_id}: {e}");
            return;
        }
    };

    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("org_tree: mkdir {}: {e}", dir.display());
        return;
    }

    // CLAUDE.md ← role_prompt_md
    if !is_blank(&agent.role_prompt_md) {
        write_protected(
            pool,
            &dir.join("CLAUDE.md"),
            "agent",
            agent_id,
            "CLAUDE.md",
            agent.role_prompt_md.as_ref().unwrap().as_bytes(),
            force,
        )
        .await;
    } else {
        remove_if_null(pool, &dir.join("CLAUDE.md"), "agent", agent_id, "CLAUDE.md", force).await;
    }

    // .mcp.json ← mcp_servers_json
    if !is_trivial_json(&agent.mcp_servers_json) {
        write_protected(
            pool,
            &dir.join(".mcp.json"),
            "agent",
            agent_id,
            ".mcp.json",
            agent.mcp_servers_json.as_bytes(),
            force,
        )
        .await;
    } else {
        remove_if_null(
            pool,
            &dir.join(".mcp.json"),
            "agent",
            agent_id,
            ".mcp.json",
            force,
        )
        .await;
    }

    // memory/context.md ← memory_md
    if !is_blank(&agent.memory_md) {
        let mem_dir = dir.join("memory");
        let _ = std::fs::create_dir_all(&mem_dir);
        write_protected(
            pool,
            &mem_dir.join("context.md"),
            "agent",
            agent_id,
            "memory/context.md",
            agent.memory_md.as_ref().unwrap().as_bytes(),
            force,
        )
        .await;
    } else {
        remove_if_null(
            pool,
            &dir.join("memory").join("context.md"),
            "agent",
            agent_id,
            "memory/context.md",
            force,
        )
        .await;
    }

    // checklist.json ← checklist_json
    if !is_trivial_json(&agent.checklist_json) {
        write_protected(
            pool,
            &dir.join("checklist.json"),
            "agent",
            agent_id,
            "checklist.json",
            agent.checklist_json.as_bytes(),
            force,
        )
        .await;
    } else {
        remove_if_null(
            pool,
            &dir.join("checklist.json"),
            "agent",
            agent_id,
            "checklist.json",
            force,
        )
        .await;
    }

    // ckp.md ← ckp_text
    if !is_blank(&agent.ckp_text) {
        write_protected(
            pool,
            &dir.join("ckp.md"),
            "agent",
            agent_id,
            "ckp.md",
            agent.ckp_text.as_ref().unwrap().as_bytes(),
            force,
        )
        .await;
    } else {
        remove_if_null(pool, &dir.join("ckp.md"), "agent", agent_id, "ckp.md", force).await;
    }

    // Update folder_path in org_agents
    let path_str = dir.to_string_lossy().to_string();
    let _ = sqlx::query("UPDATE org_agents SET folder_path = ? WHERE id = ?")
        .bind(&path_str)
        .bind(agent_id)
        .execute(pool)
        .await;
}

// =========================================================================
// Soft delete → .trash/{timestamp}_{name}
// =========================================================================

pub fn trash_folder(root: &Path, folder: &Path) {
    if !folder.exists() {
        return;
    }
    let trash = root.join(".trash");
    if let Err(e) = std::fs::create_dir_all(&trash) {
        log::warn!("org_tree: create .trash: {e}");
        return;
    }
    let ts = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let name = folder
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let dest = trash.join(format!("{ts}_{name}"));
    if let Err(e) = std::fs::rename(folder, &dest) {
        log::warn!("org_tree: trash {}: {e}", folder.display());
    }
}

// =========================================================================
// Public try_sync_* (with entity lock, called from CRUD commands)
// =========================================================================

pub async fn try_sync_division(pool: &SqlitePool, state: &OrgTreeState, div_id: &str) {
    let lock = state.entity_lock(div_id);
    let _guard = lock.lock().await;
    sync_division(pool, &state.root, div_id, false).await;
}

pub async fn try_sync_department(pool: &SqlitePool, state: &OrgTreeState, dept_id: &str) {
    let lock = state.entity_lock(dept_id);
    let _guard = lock.lock().await;
    sync_department(pool, &state.root, dept_id, false).await;
}

pub async fn try_sync_agent(pool: &SqlitePool, state: &OrgTreeState, agent_id: &str) {
    let lock = state.entity_lock(agent_id);
    let _guard = lock.lock().await;
    sync_agent(pool, &state.root, agent_id, false).await;
}

/// Sync all agents under a division (after division rename/move).
pub async fn sync_children_of_division(pool: &SqlitePool, state: &OrgTreeState, div_id: &str) {
    let depts: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM org_departments WHERE division_id = ?",
    )
    .bind(div_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (did,) in &depts {
        try_sync_department(pool, state, did).await;
    }
    let agents: Vec<(String,)> = sqlx::query_as(
        "SELECT a.id FROM org_agents a \
         JOIN org_departments d ON a.department_id = d.id \
         WHERE d.division_id = ?",
    )
    .bind(div_id)
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for (aid,) in &agents {
        try_sync_agent(pool, state, aid).await;
    }
}

/// Sync all agents under a department (after department rename/move).
pub async fn sync_children_of_department(pool: &SqlitePool, state: &OrgTreeState, dept_id: &str) {
    let agents: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM org_agents WHERE department_id = ?")
            .bind(dept_id)
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (aid,) in &agents {
        try_sync_agent(pool, state, aid).await;
    }
}

// =========================================================================
// Rebuild (full tree from DB)
// =========================================================================

pub async fn rebuild_all(
    pool: &SqlitePool,
    root: &Path,
    force: bool,
) -> Result<String, String> {
    let mut synced = 0u32;

    let divs: Vec<(String,)> = sqlx::query_as("SELECT id FROM org_divisions")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("rebuild divs: {e}"))?;
    for (id,) in &divs {
        sync_division(pool, root, id, force).await;
        synced += 1;
    }

    let depts: Vec<(String,)> = sqlx::query_as("SELECT id FROM org_departments")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("rebuild depts: {e}"))?;
    for (id,) in &depts {
        sync_department(pool, root, id, force).await;
        synced += 1;
    }

    let agents: Vec<(String,)> = sqlx::query_as("SELECT id FROM org_agents")
        .fetch_all(pool)
        .await
        .map_err(|e| format!("rebuild agents: {e}"))?;
    for (id,) in &agents {
        sync_agent(pool, root, id, force).await;
        synced += 1;
    }

    Ok(format!("rebuilt {synced} entities"))
}

// =========================================================================
// Data-fix: transliterate existing name→slug for divisions & departments
// =========================================================================

pub async fn datafix_slugs(pool: &SqlitePool) {
    // Divisions
    let divs: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, slug FROM org_divisions")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (id, name, slug) in &divs {
        if slug.is_some() {
            continue;
        }
        let base = to_disk_slug(name);
        let final_slug = dedup_slug_in_table(pool, "org_divisions", &base, Some(id))
            .await
            .unwrap_or(base);
        let _ = sqlx::query("UPDATE org_divisions SET slug = ? WHERE id = ?")
            .bind(&final_slug)
            .bind(id)
            .execute(pool)
            .await;
        log::info!("datafix: division '{name}' → slug '{final_slug}'");
    }

    // Departments
    let depts: Vec<(String, String, Option<String>)> =
        sqlx::query_as("SELECT id, name, slug FROM org_departments")
            .fetch_all(pool)
            .await
            .unwrap_or_default();
    for (id, name, slug) in &depts {
        if slug.is_some() {
            continue;
        }
        let base = to_disk_slug(name);
        let final_slug = dedup_slug_in_table(pool, "org_departments", &base, Some(id))
            .await
            .unwrap_or(base);
        let _ = sqlx::query("UPDATE org_departments SET slug = ? WHERE id = ?")
            .bind(&final_slug)
            .bind(id)
            .execute(pool)
            .await;
        log::info!("datafix: department '{name}' → slug '{final_slug}'");
    }
}

// =========================================================================
// Tauri commands
// =========================================================================

#[tauri::command]
pub async fn rebuild_org_tree(
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<String, String> {
    rebuild_all(&db.0, &tree.root, false).await
}

#[tauri::command]
pub async fn force_rebuild_org_tree(
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<String, String> {
    rebuild_all(&db.0, &tree.root, true).await
}

// =========================================================================
// Tests
// =========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;
    use tempfile::tempdir;

    async fn setup_db() -> SqlitePool {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE org_divisions ( \
                id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, \
                slug TEXT, sort_order INTEGER NOT NULL DEFAULT 0, \
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP \
            ); \
            CREATE TABLE org_departments ( \
                id TEXT PRIMARY KEY, division_id TEXT NOT NULL, name TEXT NOT NULL, \
                description TEXT, slug TEXT, sort_order INTEGER NOT NULL DEFAULT 0, \
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP \
            ); \
            CREATE TABLE org_agents ( \
                id TEXT PRIMARY KEY, department_id TEXT NOT NULL, name TEXT NOT NULL, \
                slug TEXT NOT NULL, role_label TEXT NOT NULL DEFAULT 'member', \
                status TEXT NOT NULL DEFAULT 'active', folder_path TEXT, \
                sort_order INTEGER NOT NULL DEFAULT 0, \
                created_at DATETIME DEFAULT CURRENT_TIMESTAMP, \
                updated_at TEXT DEFAULT NULL, \
                role_prompt_md TEXT DEFAULT NULL, \
                brain_mode TEXT NOT NULL DEFAULT 'disabled', \
                brain_model TEXT DEFAULT NULL, brain_endpoint TEXT DEFAULT NULL, \
                mcp_servers_json TEXT NOT NULL DEFAULT '[]', \
                ckp_text TEXT DEFAULT NULL, checklist_json TEXT NOT NULL DEFAULT '[]', \
                memory_md TEXT DEFAULT NULL \
            ); \
            CREATE TABLE org_disk_sync ( \
                entity_type TEXT NOT NULL, entity_id TEXT NOT NULL, \
                file_rel TEXT NOT NULL, content_hash TEXT NOT NULL, \
                written_at TEXT NOT NULL DEFAULT (datetime('now')), \
                PRIMARY KEY (entity_type, entity_id, file_rel) \
            );",
        )
        .execute(&pool)
        .await
        .unwrap();
        pool
    }

    // --- Slug transliteration ---

    #[test]
    fn slug_cyrillic_translit() {
        assert_eq!(to_disk_slug("Юрист"), "yurist");
        assert_eq!(to_disk_slug("Отдел Продаж"), "otdel-prodazh");
        assert_eq!(to_disk_slug("Щёлково"), "shchyolkovo");
        assert_eq!(to_disk_slug("Программист"), "programmist");
    }

    #[test]
    fn slug_ascii_passthrough() {
        assert_eq!(to_disk_slug("Office Manager"), "office-manager");
        assert_eq!(to_disk_slug("dev-ops"), "dev-ops");
        assert_eq!(to_disk_slug("test_agent"), "test_agent");
    }

    #[test]
    fn slug_rejects_traversal_and_special() {
        let s = to_disk_slug("../escape");
        assert!(!s.contains(".."), "must not contain ..: got '{s}'");
        assert!(!s.contains('/'), "must not contain /: got '{s}'");

        let s2 = to_disk_slug("hello world/../../etc");
        assert!(!s2.contains(".."));
        assert!(!s2.contains('/'));
    }

    #[test]
    fn slug_empty_fallback() {
        assert_eq!(to_disk_slug("!!!"), "entity");
        assert_eq!(to_disk_slug("   "), "entity");
        assert_eq!(to_disk_slug(""), "entity");
    }

    #[test]
    fn slug_collapse_dashes() {
        assert_eq!(to_disk_slug("a---b"), "a-b");
        assert_eq!(to_disk_slug("- leading -"), "leading");
    }

    // --- Sanitize component ---

    #[test]
    fn sanitize_strips_dots_slashes_spaces() {
        assert_eq!(sanitize_component("abc-def_123"), "abc-def_123");
        assert_eq!(sanitize_component("../escape"), "escape");
        assert_eq!(sanitize_component("hello world"), "helloworld");
        assert_eq!(sanitize_component("кириллица"), "");
    }

    // --- Source guard ---

    #[test]
    fn source_guard_rejects_empty() {
        let root = tempdir().unwrap();
        assert!(safe_org_path(root.path(), &[""]).is_err());
        assert!(safe_org_path(root.path(), &[".."]).is_err());
        assert!(safe_org_path(root.path(), &["ok", ".."]).is_err());
    }

    #[test]
    fn source_guard_accepts_valid() {
        let root = tempdir().unwrap();
        let r = safe_org_path(root.path(), &["division-1", "dept-a", "agent-x"]);
        assert!(r.is_ok());
        let p = r.unwrap();
        assert!(p.starts_with(root.path()));
    }

    // --- Idempotent sync ---

    #[tokio::test]
    async fn idempotent_sync() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Продажи', 'prodazhi'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Полевые', 'polevye'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md) \
                VALUES ('a1', 'p1', 'Алекс', 'aleks', '# Agent Alex');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sync_division(&pool, root.path(), "d1", false).await;
        sync_department(&pool, root.path(), "p1", false).await;
        sync_agent(&pool, root.path(), "a1", false).await;

        let claude_md = root.path().join("prodazhi").join("polevye").join("aleks").join("CLAUDE.md");
        assert!(claude_md.exists(), "CLAUDE.md should be created");
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert_eq!(content, "# Agent Alex");

        // Second sync — should be no-op (returns false from write_protected)
        sync_agent(&pool, root.path(), "a1", false).await;
        let content2 = std::fs::read_to_string(&claude_md).unwrap();
        assert_eq!(content2, "# Agent Alex");
    }

    // --- Manual edit preserved ---

    #[tokio::test]
    async fn manual_edit_preserved() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Dev', 'dev'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Core', 'core'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md) \
                VALUES ('a1', 'p1', 'Bot', 'bot', '# Original');",
        )
        .execute(&pool)
        .await
        .unwrap();

        // First sync
        sync_agent(&pool, root.path(), "a1", false).await;
        let claude_md = root.path().join("dev").join("core").join("bot").join("CLAUDE.md");
        assert_eq!(std::fs::read_to_string(&claude_md).unwrap(), "# Original");

        // Manual edit on disk
        std::fs::write(&claude_md, "# Manually Edited").unwrap();

        // Update DB content
        sqlx::query("UPDATE org_agents SET role_prompt_md = '# Updated from DB' WHERE id = 'a1'")
            .execute(&pool)
            .await
            .unwrap();

        // Second sync — should NOT overwrite manual edit
        sync_agent(&pool, root.path(), "a1", false).await;
        let content = std::fs::read_to_string(&claude_md).unwrap();
        assert_eq!(content, "# Manually Edited", "manual edit must be preserved");
    }

    // --- Force rebuild overrides manual edit ---

    #[tokio::test]
    async fn force_rebuild_overrides() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Div', 'div'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Dept', 'dept'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md) \
                VALUES ('a1', 'p1', 'Ag', 'ag', '# V1');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sync_agent(&pool, root.path(), "a1", false).await;
        let claude_md = root.path().join("div").join("dept").join("ag").join("CLAUDE.md");
        std::fs::write(&claude_md, "# Manual").unwrap();

        sqlx::query("UPDATE org_agents SET role_prompt_md = '# V2' WHERE id = 'a1'")
            .execute(&pool)
            .await
            .unwrap();

        // Force rebuild overwrites
        sync_agent(&pool, root.path(), "a1", true).await;
        assert_eq!(std::fs::read_to_string(&claude_md).unwrap(), "# V2");
    }

    // --- Full rebuild from DB ---

    #[tokio::test]
    async fn rebuild_from_db() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Alpha', 'alpha'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Beta', 'beta'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md, ckp_text) \
                VALUES ('a1', 'p1', 'Gamma', 'gamma', '# Gamma Agent', '# CKP notes');",
        )
        .execute(&pool)
        .await
        .unwrap();

        let result = rebuild_all(&pool, root.path(), false).await.unwrap();
        assert!(result.contains("rebuilt 3"));

        assert!(root.path().join("alpha").join("division.md").exists());
        assert!(root.path().join("alpha").join("beta").join("department.md").exists());
        assert!(root.path().join("alpha").join("beta").join("gamma").join("CLAUDE.md").exists());
        assert!(root.path().join("alpha").join("beta").join("gamma").join("ckp.md").exists());
    }

    // --- Datafix slugs ---

    #[tokio::test]
    async fn datafix_generates_slugs() {
        let pool = setup_db().await;

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name) VALUES ('d1', 'Отдел Продаж'); \
             INSERT INTO org_divisions (id, name) VALUES ('d2', 'Отдел Продаж'); \
             INSERT INTO org_departments (id, division_id, name) VALUES ('p1', 'd1', 'Юристы');",
        )
        .execute(&pool)
        .await
        .unwrap();

        datafix_slugs(&pool).await;

        let d1: (Option<String>,) =
            sqlx::query_as("SELECT slug FROM org_divisions WHERE id = 'd1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let d2: (Option<String>,) =
            sqlx::query_as("SELECT slug FROM org_divisions WHERE id = 'd2'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let p1: (Option<String>,) =
            sqlx::query_as("SELECT slug FROM org_departments WHERE id = 'p1'")
                .fetch_one(&pool)
                .await
                .unwrap();

        assert_eq!(d1.0.as_deref(), Some("otdel-prodazh"));
        assert!(d2.0.is_some());
        assert_ne!(d2.0.as_deref(), d1.0.as_deref(), "dedup must produce different slugs");
        assert_eq!(p1.0.as_deref(), Some("yuristy"));
    }

    // --- NULL fields don't produce files ---

    #[tokio::test]
    async fn null_fields_skip_files() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'D', 'd'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'P', 'p'); \
             INSERT INTO org_agents (id, department_id, name, slug) \
                VALUES ('a1', 'p1', 'Bare', 'bare');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sync_agent(&pool, root.path(), "a1", false).await;

        let agent_dir = root.path().join("d").join("p").join("bare");
        assert!(agent_dir.exists(), "agent dir should be created");
        assert!(!agent_dir.join("CLAUDE.md").exists(), "no CLAUDE.md for NULL prompt");
        assert!(!agent_dir.join(".mcp.json").exists(), "no .mcp.json for default []");
        assert!(!agent_dir.join("ckp.md").exists(), "no ckp.md for NULL");
    }

    // --- agent_card_save creates folder with CLAUDE.md (integration test for sync_agent) ---

    #[tokio::test]
    async fn sync_agent_creates_claude_md_and_mcp() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Sales', 'sales'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Field', 'field'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md, mcp_servers_json, checklist_json) \
                VALUES ('a1', 'p1', 'Bot', 'bot', '# My Agent', '[{\"name\":\"ctx7\"}]', '[\"step1\"]');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sync_agent(&pool, root.path(), "a1", false).await;

        let agent_dir = root.path().join("sales").join("field").join("bot");
        assert!(agent_dir.exists(), "agent dir must be created");
        assert!(agent_dir.join("CLAUDE.md").exists(), "CLAUDE.md must be created");
        assert_eq!(
            std::fs::read_to_string(agent_dir.join("CLAUDE.md")).unwrap(),
            "# My Agent"
        );
        assert!(agent_dir.join(".mcp.json").exists(), ".mcp.json must be created");
        assert!(agent_dir.join("checklist.json").exists(), "checklist.json must be created");

        let fp: (Option<String>,) =
            sqlx::query_as("SELECT folder_path FROM org_agents WHERE id = 'a1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(fp.0.is_some(), "folder_path must be set in DB after sync");
    }

    // --- delete_agent trashes folder ---

    #[tokio::test]
    async fn trash_agent_folder() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        sqlx::raw_sql(
            "INSERT INTO org_divisions (id, name, slug) VALUES ('d1', 'Dev', 'dev'); \
             INSERT INTO org_departments (id, division_id, name, slug) VALUES ('p1', 'd1', 'Core', 'core'); \
             INSERT INTO org_agents (id, department_id, name, slug, role_prompt_md) \
                VALUES ('a1', 'p1', 'Worker', 'worker', '# Worker');",
        )
        .execute(&pool)
        .await
        .unwrap();

        sync_agent(&pool, root.path(), "a1", false).await;
        let agent_dir = root.path().join("dev").join("core").join("worker");
        assert!(agent_dir.exists());
        assert!(agent_dir.join("CLAUDE.md").exists());

        // Simulate delete_agent: trash folder + clean sync
        trash_folder(root.path(), &agent_dir);
        clean_sync_records(&pool, "agent", "a1").await;

        assert!(!agent_dir.exists(), "agent dir must be gone after trash");
        let trash_dir = root.path().join(".trash");
        assert!(trash_dir.exists(), ".trash dir must exist");
        let entries: Vec<_> = std::fs::read_dir(&trash_dir).unwrap().collect();
        assert_eq!(entries.len(), 1, "exactly one trashed folder");
        let trashed = entries[0].as_ref().unwrap().path();
        assert!(
            trashed.join("CLAUDE.md").exists(),
            "trashed folder must contain CLAUDE.md"
        );

        let sync_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM org_disk_sync WHERE entity_type = 'agent' AND entity_id = 'a1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sync_count.0, 0, "sync records must be cleaned");
    }

    // --- write_protected adopts existing file without stored hash ---

    #[tokio::test]
    async fn write_protected_adopts_existing_no_hash() {
        let pool = setup_db().await;
        let root = tempdir().unwrap();

        let file_path = root.path().join("existing.md");
        std::fs::write(&file_path, "# Manual content").unwrap();

        let written = write_protected(
            &pool,
            &file_path,
            "agent",
            "a1",
            "existing.md",
            b"# New content from DB",
            false,
        )
        .await;
        assert!(!written, "must NOT overwrite existing file without stored hash");
        assert_eq!(
            std::fs::read_to_string(&file_path).unwrap(),
            "# Manual content",
            "original content must be preserved"
        );

        let hash = get_stored_hash(&pool, "agent", "a1", "existing.md").await;
        assert!(hash.is_some(), "hash of existing file must be stored (adopted)");
    }
}
