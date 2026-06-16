//! Этап 1 (Заход 1) — Конструктор оргструктуры (вкладка «Оргсхема»).
//!
//! Динамическое дерево 3 уровней: Отделение (`org_divisions`) → Отдел
//! (`org_departments`) → Агент (`org_agents`). Гендир — особый верхний узел,
//! рисуется фронтом отдельно (не в этих таблицах).
//!
//! ГРАНИЦА Захода 1: ТОЛЬКО операции с БД. Диск НЕ трогаем (материализация
//! папок агентов + копирование — Заход 3). `delete` = удаление строк БД
//! (явный каскад, не полагаемся на `PRAGMA foreign_keys`). Папки на диске
//! приложение в скелете НЕ создаёт и НЕ стирает на этом этапе.
//!
//! ОТДЕЛЬНЫЕ таблицы `org_*` НЕ пересекаются с `departments`/`posts`
//! (от них зависят мозг Гендира и Диспетчер) — изоляция.

use serde::Serialize;
use sqlx::FromRow;
use tauri::State;

use crate::db::WritePool;
use crate::org_tree::{self, OrgTreeState};

// ---------------------------------------------------------------------------
// DTO дерева (nested) — то, что отдаём фронту одним вызовом list_org_tree.
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct OrgTree {
    pub divisions: Vec<DivisionNode>,
}

#[derive(Debug, Serialize)]
pub struct DivisionNode {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i64,
    pub departments: Vec<DepartmentNode>,
}

#[derive(Debug, Serialize)]
pub struct DepartmentNode {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub sort_order: i64,
    pub agents: Vec<AgentNode>,
}

#[derive(Debug, Serialize, FromRow)]
pub struct AgentNode {
    pub id: String,
    pub department_id: String,
    pub name: String,
    pub slug: String,
    pub role_label: String,
    pub status: String,
    pub folder_path: Option<String>,
    pub sort_order: i64,
}

// Плоские строки для сборки дерева.
#[derive(FromRow)]
struct DivisionRow {
    id: String,
    name: String,
    description: Option<String>,
    sort_order: i64,
}

#[derive(FromRow)]
struct DepartmentRow {
    id: String,
    division_id: String,
    name: String,
    description: Option<String>,
    sort_order: i64,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Простой slug из имени (для будущего имени папки в Заходе 3). Сохраняет
/// буквы/цифры (в т.ч. кириллицу через is_alphanumeric), остальное → '-'.
/// На Заходе 1 это только метаданные — уникальность папок решается в Заходе 3.
fn make_slug(name: &str) -> String {
    let lowered = name.trim().to_lowercase();
    let mut s: String = lowered
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    while s.contains("--") {
        s = s.replace("--", "-");
    }
    let s = s.trim_matches('-').to_string();
    let s: String = s.chars().take(64).collect();
    if s.is_empty() {
        "agent".to_string()
    } else {
        s
    }
}

fn validate_name(name: &str) -> Result<String, String> {
    let t = name.trim();
    if t.is_empty() {
        return Err("название не может быть пустым".into());
    }
    if t.chars().count() > 200 {
        return Err("название слишком длинное (макс 200)".into());
    }
    Ok(t.to_string())
}

fn norm_desc(description: Option<String>) -> Option<String> {
    description
        .map(|d| d.trim().to_string())
        .filter(|d| !d.is_empty())
}

fn validate_role(role_label: &str) -> Result<String, String> {
    match role_label {
        "head" | "member" => Ok(role_label.to_string()),
        _ => Err("role_label должен быть 'head' или 'member'".into()),
    }
}

// ---------------------------------------------------------------------------
// Read — дерево целиком
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn list_org_tree(db: State<'_, WritePool>) -> Result<OrgTree, String> {
    let divs: Vec<DivisionRow> = sqlx::query_as(
        "SELECT id, name, description, sort_order FROM org_divisions ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list divisions: {e}"))?;

    let deps: Vec<DepartmentRow> = sqlx::query_as(
        "SELECT id, division_id, name, description, sort_order FROM org_departments ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list departments: {e}"))?;

    let agents: Vec<AgentNode> = sqlx::query_as(
        "SELECT id, department_id, name, slug, role_label, status, folder_path, sort_order \
         FROM org_agents ORDER BY sort_order ASC, name ASC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list agents: {e}"))?;

    // Сборка nested-дерева в памяти (данных немного — фильтрация ок).
    let divisions = divs
        .into_iter()
        .map(|d| {
            let departments = deps
                .iter()
                .filter(|dep| dep.division_id == d.id)
                .map(|dep| DepartmentNode {
                    id: dep.id.clone(),
                    name: dep.name.clone(),
                    description: dep.description.clone(),
                    sort_order: dep.sort_order,
                    agents: agents
                        .iter()
                        .filter(|a| a.department_id == dep.id)
                        .map(|a| AgentNode {
                            id: a.id.clone(),
                            department_id: a.department_id.clone(),
                            name: a.name.clone(),
                            slug: a.slug.clone(),
                            role_label: a.role_label.clone(),
                            status: a.status.clone(),
                            folder_path: a.folder_path.clone(),
                            sort_order: a.sort_order,
                        })
                        .collect(),
                })
                .collect();
            DivisionNode {
                id: d.id,
                name: d.name,
                description: d.description,
                sort_order: d.sort_order,
                departments,
            }
        })
        .collect();

    Ok(OrgTree { divisions })
}

// ---------------------------------------------------------------------------
// Divisions (Отделения)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn create_division(
    name: String,
    description: Option<String>,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<String, String> {
    let name = validate_name(&name)?;
    let slug = org_tree::to_disk_slug(&name);
    let slug = org_tree::dedup_slug_in_table(&db.0, "org_divisions", &slug, None).await?;
    let id = format!("div-{}", uuid::Uuid::new_v4());
    sqlx::query("INSERT INTO org_divisions (id, name, description, slug) VALUES (?, ?, ?, ?)")
        .bind(&id)
        .bind(&name)
        .bind(norm_desc(description))
        .bind(&slug)
        .execute(&db.0)
        .await
        .map_err(|e| format!("create division: {e}"))?;
    org_tree::try_sync_division(&db.0, &tree, &id).await;
    Ok(id)
}

#[tauri::command]
pub async fn rename_division(
    id: String,
    name: String,
    description: Option<String>,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let name = validate_name(&name)?;
    let old_slug: Option<(Option<String>,)> =
        sqlx::query_as("SELECT slug FROM org_divisions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&db.0)
            .await
            .ok()
            .flatten();
    let new_slug = org_tree::to_disk_slug(&name);
    let new_slug =
        org_tree::dedup_slug_in_table(&db.0, "org_divisions", &new_slug, Some(&id)).await?;
    let rows = sqlx::query(
        "UPDATE org_divisions SET name = ?, description = ?, slug = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(norm_desc(description))
    .bind(&new_slug)
    .bind(&id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("rename division: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err("отделение не найдено".into());
    }
    if let Some((Some(old),)) = old_slug {
        if old != new_slug {
            if let (Ok(old_path), Ok(new_path)) = (
                org_tree::safe_org_path(&tree.root, &[&old]),
                org_tree::safe_org_path(&tree.root, &[&new_slug]),
            ) {
                if old_path.exists() && !new_path.exists() {
                    let _ = std::fs::rename(&old_path, &new_path);
                }
            }
        }
    }
    org_tree::try_sync_division(&db.0, &tree, &id).await;
    org_tree::sync_children_of_division(&db.0, &tree, &id).await;
    Ok(())
}

/// Удаление отделения = явный каскад в БД + soft-delete папки на диске.
#[tauri::command]
pub async fn delete_division(
    id: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let div_slug: Option<(Option<String>,)> =
        sqlx::query_as("SELECT slug FROM org_divisions WHERE id = ?")
            .bind(&id)
            .fetch_optional(&db.0)
            .await
            .ok()
            .flatten();
    let child_agents: Vec<(String,)> = sqlx::query_as(
        "SELECT id FROM org_agents WHERE department_id IN \
         (SELECT id FROM org_departments WHERE division_id = ?)",
    )
    .bind(&id)
    .fetch_all(&db.0)
    .await
    .unwrap_or_default();
    let child_depts: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM org_departments WHERE division_id = ?")
            .bind(&id)
            .fetch_all(&db.0)
            .await
            .unwrap_or_default();

    sqlx::query(
        "DELETE FROM org_agents WHERE department_id IN \
         (SELECT id FROM org_departments WHERE division_id = ?)",
    )
    .bind(&id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("delete division agents: {e}"))?;
    sqlx::query("DELETE FROM org_departments WHERE division_id = ?")
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete division departments: {e}"))?;
    sqlx::query("DELETE FROM org_divisions WHERE id = ?")
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete division: {e}"))?;

    if let Some((Some(slug),)) = div_slug {
        if let Ok(path) = org_tree::safe_org_path(&tree.root, &[&slug]) {
            org_tree::trash_folder(&tree.root, &path);
        }
    }
    for (aid,) in &child_agents {
        org_tree::clean_sync_records(&db.0, "agent", aid).await;
    }
    for (did,) in &child_depts {
        org_tree::clean_sync_records(&db.0, "department", did).await;
    }
    org_tree::clean_sync_records(&db.0, "division", &id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Departments (Отделы)
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn create_department(
    division_id: String,
    name: String,
    description: Option<String>,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<String, String> {
    let name = validate_name(&name)?;
    let parent: Option<(String,)> = sqlx::query_as("SELECT id FROM org_divisions WHERE id = ?")
        .bind(&division_id)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("check division: {e}"))?;
    if parent.is_none() {
        return Err("отделение-родитель не найдено".into());
    }
    let slug = org_tree::to_disk_slug(&name);
    let slug = org_tree::dedup_slug_in_table(&db.0, "org_departments", &slug, None).await?;
    let id = format!("dpt-{}", uuid::Uuid::new_v4());
    sqlx::query(
        "INSERT INTO org_departments (id, division_id, name, description, slug) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&division_id)
    .bind(&name)
    .bind(norm_desc(description))
    .bind(&slug)
    .execute(&db.0)
    .await
    .map_err(|e| format!("create department: {e}"))?;
    org_tree::try_sync_department(&db.0, &tree, &id).await;
    Ok(id)
}

#[tauri::command]
pub async fn rename_department(
    id: String,
    name: String,
    description: Option<String>,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let name = validate_name(&name)?;
    let old_info: Option<(Option<String>, String)> =
        sqlx::query_as("SELECT d.slug, div.slug FROM org_departments d JOIN org_divisions div ON d.division_id = div.id WHERE d.id = ?")
            .bind(&id)
            .fetch_optional(&db.0)
            .await
            .ok()
            .flatten();
    let new_slug = org_tree::to_disk_slug(&name);
    let new_slug =
        org_tree::dedup_slug_in_table(&db.0, "org_departments", &new_slug, Some(&id)).await?;
    let rows = sqlx::query(
        "UPDATE org_departments SET name = ?, description = ?, slug = ? WHERE id = ?",
    )
    .bind(&name)
    .bind(norm_desc(description))
    .bind(&new_slug)
    .bind(&id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("rename department: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err("отдел не найден".into());
    }
    if let Some((Some(old_slug), div_slug)) = old_info {
        if old_slug != new_slug {
            if let (Ok(old_path), Ok(new_path)) = (
                org_tree::safe_org_path(&tree.root, &[&div_slug, &old_slug]),
                org_tree::safe_org_path(&tree.root, &[&div_slug, &new_slug]),
            ) {
                if old_path.exists() && !new_path.exists() {
                    let _ = std::fs::rename(&old_path, &new_path);
                }
            }
        }
    }
    org_tree::try_sync_department(&db.0, &tree, &id).await;
    org_tree::sync_children_of_department(&db.0, &tree, &id).await;
    Ok(())
}

/// Переместить отдел в другое отделение (смена «прописки» в БД + диск).
#[tauri::command]
pub async fn move_department(
    id: String,
    new_division_id: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let parent: Option<(String,)> = sqlx::query_as("SELECT id FROM org_divisions WHERE id = ?")
        .bind(&new_division_id)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("check division: {e}"))?;
    if parent.is_none() {
        return Err("целевое отделение не найдено".into());
    }
    let old_info: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT d.slug, div.slug FROM org_departments d \
         JOIN org_divisions div ON d.division_id = div.id WHERE d.id = ?",
    )
    .bind(&id)
    .fetch_optional(&db.0)
    .await
    .ok()
    .flatten();
    let rows = sqlx::query("UPDATE org_departments SET division_id = ? WHERE id = ?")
        .bind(&new_division_id)
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("move department: {e}"))?
        .rows_affected();
    if rows == 0 {
        return Err("отдел не найден".into());
    }
    if let Some((Some(dept_slug), old_div_slug)) = old_info {
        let new_div_slug: Option<(Option<String>,)> =
            sqlx::query_as("SELECT slug FROM org_divisions WHERE id = ?")
                .bind(&new_division_id)
                .fetch_optional(&db.0)
                .await
                .ok()
                .flatten();
        if let Some((Some(ndv),)) = new_div_slug {
            if let (Ok(old_path), Ok(new_path)) = (
                org_tree::safe_org_path(&tree.root, &[&old_div_slug, &dept_slug]),
                org_tree::safe_org_path(&tree.root, &[&ndv, &dept_slug]),
            ) {
                let new_parent = new_path.parent().unwrap_or(&tree.root);
                let _ = std::fs::create_dir_all(new_parent);
                if old_path.exists() && !new_path.exists() {
                    let _ = std::fs::rename(&old_path, &new_path);
                }
            }
        }
    }
    org_tree::sync_children_of_department(&db.0, &tree, &id).await;
    Ok(())
}

#[tauri::command]
pub async fn delete_department(
    id: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let dept_info: Option<(Option<String>, String)> = sqlx::query_as(
        "SELECT d.slug, div.slug FROM org_departments d \
         JOIN org_divisions div ON d.division_id = div.id WHERE d.id = ?",
    )
    .bind(&id)
    .fetch_optional(&db.0)
    .await
    .ok()
    .flatten();
    let child_agents: Vec<(String,)> =
        sqlx::query_as("SELECT id FROM org_agents WHERE department_id = ?")
            .bind(&id)
            .fetch_all(&db.0)
            .await
            .unwrap_or_default();

    sqlx::query("DELETE FROM org_agents WHERE department_id = ?")
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete department agents: {e}"))?;
    sqlx::query("DELETE FROM org_departments WHERE id = ?")
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete department: {e}"))?;

    if let Some((Some(dept_slug), div_slug)) = dept_info {
        if let Ok(path) = org_tree::safe_org_path(&tree.root, &[&div_slug, &dept_slug]) {
            org_tree::trash_folder(&tree.root, &path);
        }
    }
    for (aid,) in &child_agents {
        org_tree::clean_sync_records(&db.0, "agent", aid).await;
    }
    org_tree::clean_sync_records(&db.0, "department", &id).await;
    Ok(())
}

// ---------------------------------------------------------------------------
// Agents (Агенты) — Заход 1: только метаданные в БД, без папки на диске.
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn create_agent(
    department_id: String,
    name: String,
    role_label: Option<String>,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<String, String> {
    let name = validate_name(&name)?;
    let role = validate_role(role_label.as_deref().unwrap_or("member"))?;
    let parent: Option<(String,)> = sqlx::query_as("SELECT id FROM org_departments WHERE id = ?")
        .bind(&department_id)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("check department: {e}"))?;
    if parent.is_none() {
        return Err("отдел-родитель не найден".into());
    }
    let id = format!("agt-{}", uuid::Uuid::new_v4());
    let slug = make_slug(&name);
    sqlx::query(
        "INSERT INTO org_agents (id, department_id, name, slug, role_label) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&department_id)
    .bind(&name)
    .bind(&slug)
    .bind(&role)
    .execute(&db.0)
    .await
    .map_err(|e| format!("create agent: {e}"))?;
    org_tree::try_sync_agent(&db.0, &tree, &id).await;
    Ok(id)
}

#[tauri::command]
pub async fn rename_agent(
    id: String,
    name: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let name = validate_name(&name)?;
    let old_info: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT a.slug, d.slug, div.slug FROM org_agents a \
         JOIN org_departments d ON a.department_id = d.id \
         JOIN org_divisions div ON d.division_id = div.id \
         WHERE a.id = ?",
    )
    .bind(&id)
    .fetch_optional(&db.0)
    .await
    .ok()
    .flatten();
    let slug = make_slug(&name);
    let rows = sqlx::query(
        "UPDATE org_agents SET name = ?, slug = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&name)
    .bind(&slug)
    .bind(&id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("rename agent: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err("агент не найден".into());
    }
    if let Some((old_slug, Some(dept_slug), Some(div_slug))) = old_info {
        let new_disk_slug = org_tree::to_disk_slug(&slug);
        let old_disk_slug = org_tree::to_disk_slug(&old_slug);
        if old_disk_slug != new_disk_slug {
            if let (Ok(old_path), Ok(new_path)) = (
                org_tree::safe_org_path(&tree.root, &[&div_slug, &dept_slug, &old_disk_slug]),
                org_tree::safe_org_path(&tree.root, &[&div_slug, &dept_slug, &new_disk_slug]),
            ) {
                if old_path.exists() && !new_path.exists() {
                    let _ = std::fs::rename(&old_path, &new_path);
                }
            }
        }
    }
    org_tree::try_sync_agent(&db.0, &tree, &id).await;
    Ok(())
}

/// Переместить агента в другой отдел = смена «прописки» в БД + перенос папки на диске.
#[tauri::command]
pub async fn move_agent(
    id: String,
    new_department_id: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let parent: Option<(String,)> = sqlx::query_as("SELECT id FROM org_departments WHERE id = ?")
        .bind(&new_department_id)
        .fetch_optional(&db.0)
        .await
        .map_err(|e| format!("check department: {e}"))?;
    if parent.is_none() {
        return Err("целевой отдел не найден".into());
    }
    let old_info: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT a.slug, d.slug, div.slug FROM org_agents a \
         JOIN org_departments d ON a.department_id = d.id \
         JOIN org_divisions div ON d.division_id = div.id \
         WHERE a.id = ?",
    )
    .bind(&id)
    .fetch_optional(&db.0)
    .await
    .ok()
    .flatten();
    let rows = sqlx::query(
        "UPDATE org_agents SET department_id = ?, updated_at = datetime('now') WHERE id = ?",
    )
    .bind(&new_department_id)
    .bind(&id)
    .execute(&db.0)
    .await
    .map_err(|e| format!("move agent: {e}"))?
    .rows_affected();
    if rows == 0 {
        return Err("агент не найден".into());
    }
    if let Some((agent_slug, Some(old_dept_slug), Some(old_div_slug))) = old_info {
        let new_loc: Option<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT d.slug, div.slug FROM org_departments d \
             JOIN org_divisions div ON d.division_id = div.id \
             WHERE d.id = ?",
        )
        .bind(&new_department_id)
        .fetch_optional(&db.0)
        .await
        .ok()
        .flatten();
        if let Some((Some(new_dept_slug), Some(new_div_slug))) = new_loc {
            let disk_slug = org_tree::to_disk_slug(&agent_slug);
            if let (Ok(old_path), Ok(new_path)) = (
                org_tree::safe_org_path(
                    &tree.root,
                    &[&old_div_slug, &old_dept_slug, &disk_slug],
                ),
                org_tree::safe_org_path(
                    &tree.root,
                    &[&new_div_slug, &new_dept_slug, &disk_slug],
                ),
            ) {
                if let Some(new_parent) = new_path.parent() {
                    let _ = std::fs::create_dir_all(new_parent);
                }
                if old_path.exists() && !new_path.exists() {
                    let _ = std::fs::rename(&old_path, &new_path);
                }
            }
        }
    }
    org_tree::try_sync_agent(&db.0, &tree, &id).await;
    Ok(())
}

/// Удаление агента = строка БД + связи + soft-delete папки на диске.
#[tauri::command]
pub async fn delete_agent(
    id: String,
    db: State<'_, WritePool>,
    tree: State<'_, OrgTreeState>,
) -> Result<(), String> {
    let agent_loc: Option<(String, Option<String>, Option<String>)> = sqlx::query_as(
        "SELECT a.slug, d.slug, div.slug FROM org_agents a \
         JOIN org_departments d ON a.department_id = d.id \
         JOIN org_divisions div ON d.division_id = div.id \
         WHERE a.id = ?",
    )
    .bind(&id)
    .fetch_optional(&db.0)
    .await
    .ok()
    .flatten();

    sqlx::query("DELETE FROM org_agent_links WHERE from_agent_id = ? OR to_agent_id = ?")
        .bind(&id)
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete agent links: {e}"))?;
    let rows = sqlx::query("DELETE FROM org_agents WHERE id = ?")
        .bind(&id)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete agent: {e}"))?
        .rows_affected();
    if rows == 0 {
        return Err("агент не найден".into());
    }

    if let Some((agent_slug, Some(dept_slug), Some(div_slug))) = agent_loc {
        let disk_slug = org_tree::to_disk_slug(&agent_slug);
        if let Ok(path) =
            org_tree::safe_org_path(&tree.root, &[&div_slug, &dept_slug, &disk_slug])
        {
            org_tree::trash_folder(&tree.root, &path);
        }
    }
    org_tree::clean_sync_records(&db.0, "agent", &id).await;
    Ok(())
}

/// Сменить метку роли (глава/обычный) и статус (active/paused/off).
#[tauri::command]
pub async fn set_agent_role_status(
    id: String,
    role_label: Option<String>,
    status: Option<String>,
    db: State<'_, WritePool>,
) -> Result<(), String> {
    if let Some(role) = role_label {
        let role = validate_role(&role)?;
        sqlx::query("UPDATE org_agents SET role_label = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(&role)
            .bind(&id)
            .execute(&db.0)
            .await
            .map_err(|e| format!("set role: {e}"))?;
    }
    if let Some(st) = status {
        if !matches!(st.as_str(), "active" | "paused" | "off") {
            return Err("status должен быть active/paused/off".into());
        }
        sqlx::query("UPDATE org_agents SET status = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(&st)
            .bind(&id)
            .execute(&db.0)
            .await
            .map_err(|e| format!("set status: {e}"))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slug_basic_and_cyrillic() {
        assert_eq!(make_slug("Office Manager"), "office-manager");
        assert_eq!(make_slug("  Юрист!! "), "юрист");
        assert_eq!(make_slug("a---b"), "a-b");
        assert_eq!(make_slug("!!!"), "agent");
    }

    #[test]
    fn role_validation() {
        assert!(validate_role("head").is_ok());
        assert!(validate_role("member").is_ok());
        assert!(validate_role("boss").is_err());
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("  ").is_err());
        assert_eq!(validate_name("  Продажи ").unwrap(), "Продажи");
    }

    #[tokio::test]
    async fn tree_crud_roundtrip_db_only() {
        use sqlx::SqlitePool;
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(
            "CREATE TABLE org_divisions (id TEXT PRIMARY KEY, name TEXT NOT NULL, description TEXT, sort_order INTEGER NOT NULL DEFAULT 0, created_at DATETIME DEFAULT CURRENT_TIMESTAMP); \
             CREATE TABLE org_departments (id TEXT PRIMARY KEY, division_id TEXT NOT NULL, name TEXT NOT NULL, description TEXT, sort_order INTEGER NOT NULL DEFAULT 0, created_at DATETIME DEFAULT CURRENT_TIMESTAMP); \
             CREATE TABLE org_agents (id TEXT PRIMARY KEY, department_id TEXT NOT NULL, name TEXT NOT NULL, slug TEXT NOT NULL, role_label TEXT NOT NULL DEFAULT 'member', status TEXT NOT NULL DEFAULT 'active', folder_path TEXT, sort_order INTEGER NOT NULL DEFAULT 0, created_at DATETIME DEFAULT CURRENT_TIMESTAMP, updated_at TEXT);",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Вставляем: 1 отделение → 1 отдел → 1 агент (вручную, без State).
        sqlx::query("INSERT INTO org_divisions (id, name) VALUES ('d1', 'Продажи')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO org_departments (id, division_id, name) VALUES ('p1', 'd1', 'Полевые')")
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO org_agents (id, department_id, name, slug) VALUES ('a1', 'p1', 'Алекс', 'aleks')")
            .execute(&pool).await.unwrap();

        // Каскад delete отделения убирает отдел и агента (логика delete_division).
        sqlx::query("DELETE FROM org_agents WHERE department_id IN (SELECT id FROM org_departments WHERE division_id = 'd1')")
            .execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM org_departments WHERE division_id = 'd1'").execute(&pool).await.unwrap();
        sqlx::query("DELETE FROM org_divisions WHERE id = 'd1'").execute(&pool).await.unwrap();

        let agents_left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM org_agents").fetch_one(&pool).await.unwrap();
        let deps_left: (i64,) = sqlx::query_as("SELECT COUNT(*) FROM org_departments").fetch_one(&pool).await.unwrap();
        assert_eq!(agents_left.0, 0);
        assert_eq!(deps_left.0, 0);
    }
}
