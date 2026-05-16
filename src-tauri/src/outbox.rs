//! Centralized Outbox для артефактов задач (Фаза 11C).
//!
//! Файловая структура:
//!
//!   <app_data_dir>/Outbox/
//!     ├── <task-id>/
//!     │    ├── _metadata.json         — snapshot dispatcher_logs row
//!     │    ├── result.docx             — артефакт от поста/агента
//!     │    └── ...
//!     └── <task-id-2>/
//!
//! Артефакты регистрируются в SQLite таблице `task_artifacts` (см.
//! migration 06_dispatcher_hub.sql). Физические файлы — здесь.
//!
//! Защита от path-traversal:
//!   1. task_id санитизируется через `sanitize_task_id`
//!   2. rel_path не должен содержать `..` / абсолютных префиксов /
//!      символов вне `[A-Za-z0-9._-/]`
//!   3. После `path.join(rel)` проверяем `canonicalize().starts_with(outbox_root)`

use std::path::{Path, PathBuf};

/// Имя поддиректории внутри `<app_data>/`.
pub const OUTBOX_DIR: &str = "Outbox";

/// Cap на размер одного артефакта — 100 MB. Больше — требует отдельной логики.
pub const ARTIFACT_MAX_BYTES: u64 = 100 * 1024 * 1024;

/// Возвращает `<vault_root_parent>/Outbox/`. На практике `vault_root` это
/// `<app_data>/Vault/` (см. lib.rs::setup) — родитель = `<app_data>/`.
pub fn outbox_root(vault_root: &Path) -> PathBuf {
    vault_root
        .parent()
        .map(|p| p.join(OUTBOX_DIR))
        .unwrap_or_else(|| PathBuf::from(OUTBOX_DIR))
}

/// Идемпотентно создаёт корень Outbox. Вызывается из setup().
pub fn ensure_outbox_root(vault_root: &Path) -> std::io::Result<PathBuf> {
    let root = outbox_root(vault_root);
    std::fs::create_dir_all(&root)?;
    Ok(root)
}

/// Санитизирует task_id для использования как имя директории.
/// Разрешено: ASCII alphanumeric, `-`, `_`. Длина 1..=80.
/// Это совпадает с форматом UUID + префиксов (`task-<uuid>`).
pub fn sanitize_task_id(task_id: &str) -> Result<String, String> {
    let s = task_id.trim();
    if s.is_empty() || s.len() > 80 {
        return Err(format!(
            "task_id длина {} вне 1..=80",
            s.len()
        ));
    }
    if s == "." || s == ".." {
        return Err("task_id '.' / '..' запрещён".into());
    }
    if !s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(format!("task_id '{s}' содержит запрещённые символы"));
    }
    Ok(s.to_string())
}

/// Санитизирует относительный путь файла внутри `<task-id>/`.
/// Разрешено: ASCII alphanumeric, `.`, `-`, `_`, `/`. Без ведущих `/`,
/// без `..` сегментов, длина ≤ 200.
pub fn sanitize_rel_path(rel: &str) -> Result<String, String> {
    let s = rel.trim().trim_start_matches('/').trim_start_matches('\\');
    if s.is_empty() || s.len() > 200 {
        return Err(format!("rel_path длина {} вне 1..=200", s.len()));
    }
    // Нормализуем разделители на /
    let normalized: String = s.chars().map(|c| if c == '\\' { '/' } else { c }).collect();
    for seg in normalized.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return Err(format!("rel_path содержит запрещённый сегмент '{seg}'"));
        }
        if !seg.chars().all(|c| {
            c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'
        }) {
            return Err(format!("rel_path сегмент '{seg}' содержит запрещённые символы"));
        }
    }
    Ok(normalized)
}

/// Возвращает `<outbox_root>/<task-id>/`, создаёт идемпотентно.
pub fn task_outbox_dir(vault_root: &Path, task_id: &str) -> Result<PathBuf, String> {
    let safe = sanitize_task_id(task_id)?;
    let root = outbox_root(vault_root);
    std::fs::create_dir_all(&root).map_err(|e| format!("create outbox_root: {e}"))?;
    let dir = root.join(safe);
    std::fs::create_dir_all(&dir).map_err(|e| format!("create task_outbox_dir: {e}"))?;
    Ok(dir)
}

/// Полный безопасный путь к артефакту: `<outbox>/<task-id>/<rel>`.
/// Возвращает Err если канонизированный путь вышел за пределы task_outbox_dir.
pub fn safe_artifact_path(
    vault_root: &Path,
    task_id: &str,
    rel_path: &str,
) -> Result<PathBuf, String> {
    let task_dir = task_outbox_dir(vault_root, task_id)?;
    let safe_rel = sanitize_rel_path(rel_path)?;
    let candidate = task_dir.join(&safe_rel);

    // Канонизируем parent (файл может не существовать ещё).
    if let Some(parent) = candidate.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("ensure artifact parent: {e}"))?;
        let canon_parent = parent
            .canonicalize()
            .map_err(|e| format!("canonicalize parent: {e}"))?;
        let canon_task = task_dir
            .canonicalize()
            .map_err(|e| format!("canonicalize task_dir: {e}"))?;
        if !canon_parent.starts_with(&canon_task) {
            return Err("artifact path escapes task_outbox_dir".into());
        }
    }
    Ok(candidate)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_task_id_accepts_valid() {
        assert_eq!(sanitize_task_id("task-abc123").unwrap(), "task-abc123");
        assert_eq!(
            sanitize_task_id("550e8400-e29b-41d4-a716-446655440000").unwrap(),
            "550e8400-e29b-41d4-a716-446655440000"
        );
        assert_eq!(sanitize_task_id("simple_id").unwrap(), "simple_id");
    }

    #[test]
    fn sanitize_task_id_rejects_traversal() {
        assert!(sanitize_task_id("..").is_err());
        assert!(sanitize_task_id(".").is_err());
        assert!(sanitize_task_id("../../etc").is_err());
        assert!(sanitize_task_id("with/slash").is_err());
        assert!(sanitize_task_id("with\\backslash").is_err());
        assert!(sanitize_task_id("").is_err());
        assert!(sanitize_task_id("кириллица").is_err());
    }

    #[test]
    fn sanitize_rel_path_accepts_nested() {
        assert_eq!(sanitize_rel_path("result.docx").unwrap(), "result.docx");
        assert_eq!(
            sanitize_rel_path("logs/execution.log").unwrap(),
            "logs/execution.log"
        );
        // Backslashes нормализуются в forward slash
        assert_eq!(
            sanitize_rel_path("logs\\sub\\file.txt").unwrap(),
            "logs/sub/file.txt"
        );
        // Ведущий слеш удаляется
        assert_eq!(sanitize_rel_path("/result.docx").unwrap(), "result.docx");
    }

    #[test]
    fn sanitize_rel_path_rejects_traversal() {
        assert!(sanitize_rel_path("../escape.txt").is_err());
        assert!(sanitize_rel_path("logs/../escape.txt").is_err());
        assert!(sanitize_rel_path("a/./b.txt").is_err());
        assert!(sanitize_rel_path("file with space.txt").is_err());
        assert!(sanitize_rel_path("").is_err());
    }

    #[test]
    fn task_outbox_dir_isolates() {
        let tmp = std::env::temp_dir().join(format!("outbox-test-{}", uuid::Uuid::new_v4()));
        let fake_vault = tmp.join("Vault");
        std::fs::create_dir_all(&fake_vault).unwrap();

        let dir_a = task_outbox_dir(&fake_vault, "task-a").unwrap();
        let dir_b = task_outbox_dir(&fake_vault, "task-b").unwrap();
        assert!(dir_a.exists());
        assert!(dir_b.exists());
        assert_ne!(dir_a, dir_b);

        // Оба должны быть внутри <tmp>/Outbox/, не внутри Vault/
        let outbox_root_path = outbox_root(&fake_vault);
        assert!(dir_a.starts_with(&outbox_root_path));
        assert!(!dir_a.starts_with(&fake_vault));

        std::fs::remove_dir_all(&tmp).ok();
    }

    #[test]
    fn safe_artifact_path_blocks_escape() {
        let tmp = std::env::temp_dir().join(format!("outbox-esc-{}", uuid::Uuid::new_v4()));
        let fake_vault = tmp.join("Vault");
        std::fs::create_dir_all(&fake_vault).unwrap();

        // OK case
        let ok = safe_artifact_path(&fake_vault, "task-x", "result.docx").unwrap();
        assert!(ok.parent().unwrap().exists());

        // Escape attempts
        assert!(safe_artifact_path(&fake_vault, "task-x", "../../etc/passwd").is_err());
        assert!(safe_artifact_path(&fake_vault, "..", "result.docx").is_err());

        std::fs::remove_dir_all(&tmp).ok();
    }
}
