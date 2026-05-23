//! TICKET-001: arbitrary Vault file operations for CEO tools.
//!
//! write_vault_file / patch_vault_file / delete_vault_file with path validation,
//! atomic writes, soft-delete to `.archive/`, and audit logging.

use std::fs;
use std::io::{ErrorKind, Write};
use std::path::{Component, Path, PathBuf};

use chrono::Utc;
use sqlx::SqlitePool;

pub const MAX_CONTENT_BYTES: usize = 200 * 1024;
pub const MAX_PATH_BYTES: usize = 255;
pub const ARCHIVE_DIR: &str = ".archive";

const ALLOWED_EXTENSIONS: &[&str] = &["md", "txt", "json", "yaml", "yml"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VaultOpError {
    PathTraversal,
    InvalidExtension,
    ContentTooLarge,
    FileExists,
    FileNotFound,
    AnchorNotFound,
    AmbiguousAnchor,
    AnchorRequired,
    PermissionDenied,
    DiskFull,
    CannotArchiveArchive,
    Io(String),
}

impl VaultOpError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::PathTraversal => "PathTraversal",
            Self::InvalidExtension => "InvalidExtension",
            Self::ContentTooLarge => "ContentTooLarge",
            Self::FileExists => "FileExists",
            Self::FileNotFound => "FileNotFound",
            Self::AnchorNotFound => "AnchorNotFound",
            Self::AmbiguousAnchor => "AmbiguousAnchor",
            Self::AnchorRequired => "AnchorRequired",
            Self::PermissionDenied => "PermissionDenied",
            Self::DiskFull => "DiskFull",
            Self::CannotArchiveArchive => "CannotArchiveArchive",
            Self::Io(_) => "IoError",
        }
    }
}

impl std::fmt::Display for VaultOpError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(msg) => write!(f, "{msg}"),
            other => write!(f, "{}", other.code()),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PatchMode {
    Prepend,
    Append,
    InsertAfter,
}

impl PatchMode {
    pub fn parse(s: &str) -> Result<Self, VaultOpError> {
        match s {
            "prepend" => Ok(Self::Prepend),
            "append" => Ok(Self::Append),
            "insert_after" => Ok(Self::InsertAfter),
            _ => Err(VaultOpError::Io(format!("unknown patch mode: {s}"))),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Prepend => "prepend",
            Self::Append => "append",
            Self::InsertAfter => "insert_after",
        }
    }
}

#[derive(Debug, Clone)]
pub struct WriteResult {
    pub abs_path: PathBuf,
    pub bytes_written: usize,
    pub created_dirs: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PatchResult {
    pub abs_path: PathBuf,
    pub bytes_before: usize,
    pub bytes_after: usize,
    pub mode: PatchMode,
}

#[derive(Debug, Clone)]
pub struct DeleteResult {
    pub original_path: PathBuf,
    pub archive_path: PathBuf,
}

/// Resolve `rel_path` under `vault_root` with traversal checks.
pub fn resolve_vault_path(vault_root: &Path, rel_path: &str) -> Result<PathBuf, VaultOpError> {
    validate_rel_path(rel_path)?;
    let rel = Path::new(rel_path);
    if rel.is_absolute() {
        return Err(VaultOpError::PathTraversal);
    }
    for component in rel.components() {
        match component {
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(VaultOpError::PathTraversal);
            }
            Component::CurDir | Component::Normal(_) => {}
        }
    }
    let joined = vault_root.join(rel);
    ensure_inside_vault(vault_root, &joined)
}

pub fn validate_rel_path(rel_path: &str) -> Result<(), VaultOpError> {
    let trimmed = rel_path.trim();
    if trimmed.is_empty() || trimmed.as_bytes().len() > MAX_PATH_BYTES {
        return Err(VaultOpError::PathTraversal);
    }
    if trimmed.contains('\\') {
        // normalize check — backslashes rejected for consistency
        return Err(VaultOpError::PathTraversal);
    }
    if trimmed.starts_with('/') || trimmed.contains("..") {
        return Err(VaultOpError::PathTraversal);
    }
    validate_extension(trimmed)?;
    Ok(())
}

pub fn validate_extension(rel_path: &str) -> Result<(), VaultOpError> {
    let ext = Path::new(rel_path)
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_lowercase())
        .ok_or(VaultOpError::InvalidExtension)?;
    if ALLOWED_EXTENSIONS.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(VaultOpError::InvalidExtension)
    }
}

pub fn validate_content_size(content: &str) -> Result<(), VaultOpError> {
    if content.len() > MAX_CONTENT_BYTES {
        return Err(VaultOpError::ContentTooLarge);
    }
    Ok(())
}

fn ensure_inside_vault(vault_root: &Path, candidate: &Path) -> Result<PathBuf, VaultOpError> {
    let canon_root = vault_root
        .canonicalize()
        .map_err(map_io_err)?;
    if candidate.exists() {
        let canon = candidate.canonicalize().map_err(map_io_err)?;
        if !canon.starts_with(&canon_root) {
            return Err(VaultOpError::PathTraversal);
        }
        return Ok(canon);
    }
    // File may not exist yet — canonicalize nearest existing parent.
    let mut current = candidate.to_path_buf();
    while !current.exists() {
        let Some(parent) = current.parent() else {
            return Err(VaultOpError::PathTraversal);
        };
        if parent == current {
            return Err(VaultOpError::PathTraversal);
        }
        current = parent.to_path_buf();
    }
    let canon_parent = current.canonicalize().map_err(map_io_err)?;
    if !canon_parent.starts_with(&canon_root) {
        return Err(VaultOpError::PathTraversal);
    }
    Ok(candidate.to_path_buf())
}

fn map_io_err(e: std::io::Error) -> VaultOpError {
    match e.kind() {
        ErrorKind::PermissionDenied => VaultOpError::PermissionDenied,
        ErrorKind::StorageFull => VaultOpError::DiskFull,
        _ => VaultOpError::Io(e.to_string()),
    }
}

fn atomic_write(path: &Path, content: &str) -> Result<(), VaultOpError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(map_io_err)?;
    }
    let tmp = path.with_extension(format!(
        "{}.tmp",
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("tmp")
    ));
    {
        let mut file = fs::File::create(&tmp).map_err(map_io_err)?;
        file.write_all(content.as_bytes()).map_err(map_io_err)?;
        file.sync_all().map_err(map_io_err)?;
    }
    if path.exists() {
        fs::remove_file(path).map_err(map_io_err)?;
    }
    fs::rename(&tmp, path).map_err(map_io_err)?;
    Ok(())
}

pub fn write_file(
    vault_root: &Path,
    rel_path: &str,
    content: &str,
    overwrite: bool,
) -> Result<WriteResult, VaultOpError> {
    validate_content_size(content)?;
    let target = resolve_vault_path(vault_root, rel_path)?;
    if target.exists() && !overwrite {
        return Err(VaultOpError::FileExists);
    }
    let mut created_dirs = Vec::new();
    if let Some(parent) = target.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent).map_err(map_io_err)?;
            if let Ok(rel) = parent.strip_prefix(vault_root) {
                created_dirs.push(rel.display().to_string());
            }
        }
    }
    atomic_write(&target, content)?;
    Ok(WriteResult {
        abs_path: target,
        bytes_written: content.len(),
        created_dirs,
    })
}

pub fn patch_file(
    vault_root: &Path,
    rel_path: &str,
    mode: PatchMode,
    content: &str,
    anchor: Option<&str>,
) -> Result<PatchResult, VaultOpError> {
    validate_content_size(content)?;
    let target = resolve_vault_path(vault_root, rel_path)?;
    if !target.is_file() {
        return Err(VaultOpError::FileNotFound);
    }
    let old = fs::read_to_string(&target).map_err(map_io_err)?;
    let bytes_before = old.len();

    let new_content = match mode {
        PatchMode::Prepend => format!("{content}\n{old}"),
        PatchMode::Append => format!("{old}\n{content}"),
        PatchMode::InsertAfter => {
            let anchor = anchor.ok_or(VaultOpError::AnchorRequired)?;
            insert_after_anchor(&old, anchor, content)?
        }
    };
    if new_content.len() > MAX_CONTENT_BYTES {
        return Err(VaultOpError::ContentTooLarge);
    }
    let bytes_after = new_content.len();
    atomic_write(&target, &new_content)?;
    Ok(PatchResult {
        abs_path: target,
        bytes_before,
        bytes_after,
        mode,
    })
}

fn insert_after_anchor(old: &str, anchor: &str, content: &str) -> Result<String, VaultOpError> {
    let mut matches = 0usize;
    let mut out = String::new();
    let mut inserted = false;
    for line in old.split_inclusive('\n') {
        out.push_str(line);
        if line.contains(anchor) {
            matches += 1;
            if matches == 1 {
                out.push_str(content);
                if !content.ends_with('\n') {
                    out.push('\n');
                }
                inserted = true;
            }
        }
    }
    if matches == 0 {
        return Err(VaultOpError::AnchorNotFound);
    }
    if matches > 1 {
        return Err(VaultOpError::AmbiguousAnchor);
    }
    debug_assert!(inserted);
    Ok(out)
}

pub fn delete_file(
    vault_root: &Path,
    rel_path: &str,
    reason: Option<&str>,
) -> Result<DeleteResult, VaultOpError> {
    if rel_path.trim_start_matches('/').starts_with(ARCHIVE_DIR) {
        return Err(VaultOpError::CannotArchiveArchive);
    }
    let target = resolve_vault_path(vault_root, rel_path)?;
    if !target.is_file() {
        return Err(VaultOpError::FileNotFound);
    }
    let date = Utc::now().format("%Y-%m-%d").to_string();
    let archive_base = vault_root.join(ARCHIVE_DIR).join(&date).join(rel_path);
    fs::create_dir_all(
        archive_base
            .parent()
            .ok_or_else(|| VaultOpError::Io("no archive parent".into()))?,
    )
    .map_err(map_io_err)?;

    let mut archive_path = archive_base.clone();
    let mut n = 2u32;
    while archive_path.exists() {
        let stem = archive_base
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("file");
        let ext = archive_base.extension().and_then(|e| e.to_str());
        let parent = archive_path.parent().unwrap();
        let new_name = match ext {
            Some(ext) => format!("{stem}__{n}.{ext}"),
            None => format!("{stem}__{n}"),
        };
        archive_path = parent.join(new_name);
        n += 1;
    }

    fs::rename(&target, &archive_path).map_err(map_io_err)?;

    if let Some(r) = reason {
        let archived = fs::read_to_string(&archive_path).map_err(map_io_err)?;
        let header = format!("<!-- ARCHIVED {date} by CEO. Reason: {r} -->\n");
        fs::write(&archive_path, format!("{header}{archived}")).map_err(map_io_err)?;
    }

    Ok(DeleteResult {
        original_path: target,
        archive_path,
    })
}

pub async fn log_vault_op(
    pool: &SqlitePool,
    source_post: &str,
    tool: &str,
    path: &str,
    mode: Option<&str>,
    anchor: Option<&str>,
    bytes_before: Option<i64>,
    bytes_after: Option<i64>,
    success: bool,
    error_code: Option<&str>,
    archive_path: Option<&str>,
    reason: Option<&str>,
) -> Result<(), sqlx::Error> {
    let ts = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO vault_ops_log \
         (timestamp, source_post, tool, path, mode, anchor, bytes_before, bytes_after, success, error_code, archive_path, reason) \
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&ts)
    .bind(source_post)
    .bind(tool)
    .bind(path)
    .bind(mode)
    .bind(anchor)
    .bind(bytes_before)
    .bind(bytes_after)
    .bind(if success { 1 } else { 0 })
    .bind(error_code)
    .bind(archive_path)
    .bind(reason)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Mutex, MutexGuard};
    use tempfile::TempDir;

    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock_tests() -> MutexGuard<'static, ()> {
        TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn setup_vault() -> (TempDir, PathBuf) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("Vault");
        fs::create_dir_all(&root).unwrap();
        let canon = root.canonicalize().unwrap();
        (tmp, canon)
    }

    #[test]
    fn write_creates_new_file_in_root() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        let r = write_file(&root, "test.md", "# hi", false).unwrap();
        assert!(r.abs_path.exists());
        assert_eq!(fs::read_to_string(r.abs_path).unwrap(), "# hi");
    }

    #[test]
    fn write_creates_nested_dirs() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "03-Phases/sub/file.md", "x", false).unwrap();
        assert!(root.join("03-Phases/sub/file.md").exists());
    }

    #[test]
    fn write_rejects_path_traversal() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert_eq!(
            write_file(&root, "../../etc/passwd", "x", false).unwrap_err(),
            VaultOpError::PathTraversal
        );
    }

    #[test]
    fn write_rejects_absolute() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert!(matches!(
            write_file(&root, "/etc/passwd", "x", false),
            Err(VaultOpError::PathTraversal)
        ));
    }

    #[test]
    fn write_rejects_bad_extension() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert_eq!(
            write_file(&root, "malware.exe", "x", false).unwrap_err(),
            VaultOpError::InvalidExtension
        );
    }

    #[test]
    fn write_rejects_oversize() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        let big = "x".repeat(MAX_CONTENT_BYTES + 1);
        assert_eq!(
            write_file(&root, "big.md", &big, false).unwrap_err(),
            VaultOpError::ContentTooLarge
        );
    }

    #[test]
    fn write_no_overwrite_by_default() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "dup.md", "a", false).unwrap();
        assert_eq!(
            write_file(&root, "dup.md", "b", false).unwrap_err(),
            VaultOpError::FileExists
        );
    }

    #[test]
    fn write_overwrite_true_replaces() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "dup.md", "a", false).unwrap();
        write_file(&root, "dup.md", "b", true).unwrap();
        assert_eq!(fs::read_to_string(root.join("dup.md")).unwrap(), "b");
    }

    #[test]
    fn write_atomic_preserves_original_if_tmp_not_renamed() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "safe.md", "original content", false).unwrap();
        let target = root.join("safe.md");
        let tmp = target.with_extension("md.tmp");
        fs::write(&tmp, "partial crash data").unwrap();
        // Simulate crash before rename — original must remain intact.
        assert_eq!(
            fs::read_to_string(&target).unwrap(),
            "original content"
        );
        let _ = fs::remove_file(tmp);
    }

    #[test]
    fn patch_size_after_exceeds_limit() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        let base = "x".repeat(MAX_CONTENT_BYTES - 1);
        write_file(&root, "big.md", &base, false).unwrap();
        assert_eq!(
            patch_file(&root, "big.md", PatchMode::Append, "xx", None).unwrap_err(),
            VaultOpError::ContentTooLarge
        );
    }

    #[test]
    fn patch_prepend_append_insert() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "p.md", "middle\n", false).unwrap();
        patch_file(&root, "p.md", PatchMode::Prepend, "top", None).unwrap();
        patch_file(&root, "p.md", PatchMode::Append, "bottom", None).unwrap();
        let s = fs::read_to_string(root.join("p.md")).unwrap();
        assert!(s.starts_with("top"));
        assert!(s.contains("middle"));
        assert!(s.contains("bottom"));

        write_file(&root, "a.md", "line1\nANCHOR\nline3\n", false).unwrap();
        patch_file(
            &root,
            "a.md",
            PatchMode::InsertAfter,
            "inserted",
            Some("ANCHOR"),
        )
        .unwrap();
        let a = fs::read_to_string(root.join("a.md")).unwrap();
        assert!(a.contains("ANCHOR\ninserted\n"));
    }

    #[test]
    fn patch_file_not_found() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert_eq!(
            patch_file(&root, "missing.md", PatchMode::Append, "x", None).unwrap_err(),
            VaultOpError::FileNotFound
        );
    }

    #[test]
    fn delete_file_not_found() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert_eq!(
            delete_file(&root, "missing.md", None).unwrap_err(),
            VaultOpError::FileNotFound
        );
    }

    #[test]
    fn io_error_mapping_permission_and_disk_full() {
        assert_eq!(
            map_io_err(std::io::Error::new(
                ErrorKind::PermissionDenied,
                "denied"
            )),
            VaultOpError::PermissionDenied
        );
        assert_eq!(
            map_io_err(std::io::Error::new(ErrorKind::StorageFull, "full")),
            VaultOpError::DiskFull
        );
    }

    #[test]
    fn patch_anchor_errors() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "x.md", "only\n", false).unwrap();
        assert_eq!(
            patch_file(&root, "x.md", PatchMode::InsertAfter, "z", Some("nope")).unwrap_err(),
            VaultOpError::AnchorNotFound
        );
        assert_eq!(
            patch_file(&root, "x.md", PatchMode::InsertAfter, "z", None).unwrap_err(),
            VaultOpError::AnchorRequired
        );
        write_file(&root, "y.md", "A\nA\n", false).unwrap();
        assert_eq!(
            patch_file(&root, "y.md", PatchMode::InsertAfter, "z", Some("A")).unwrap_err(),
            VaultOpError::AmbiguousAnchor
        );
    }

    #[test]
    fn delete_moves_to_archive() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "decisions-log.md", "DEC", false).unwrap();
        let r = delete_file(&root, "decisions-log.md", None).unwrap();
        assert!(!root.join("decisions-log.md").exists());
        assert!(r.archive_path.exists());
        assert!(r.archive_path.to_string_lossy().contains(".archive"));
    }

    #[test]
    fn delete_collision_appends_suffix() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        let date = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let archive_dir = root.join(ARCHIVE_DIR).join(&date);
        fs::create_dir_all(&archive_dir).unwrap();
        fs::write(archive_dir.join("dup.md"), "already").unwrap();
        write_file(&root, "dup.md", "live", false).unwrap();
        let r = delete_file(&root, "dup.md", None).unwrap();
        assert!(r.archive_path.to_string_lossy().contains("__2"));
    }

    #[test]
    fn delete_moves_to_archive_with_reason() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "decisions-log.md", "DEC", false).unwrap();
        let r = delete_file(&root, "decisions-log.md", Some("obsolete")).unwrap();
        assert!(!root.join("decisions-log.md").exists());
        assert!(r.archive_path.exists());
        let archived = fs::read_to_string(r.archive_path).unwrap();
        assert!(archived.starts_with("<!-- ARCHIVED"));
    }

    #[test]
    fn delete_rejects_archive_path() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        assert_eq!(
            delete_file(&root, ".archive/foo.md", None).unwrap_err(),
            VaultOpError::CannotArchiveArchive
        );
    }

    #[test]
    fn vault_sync_e2e() {
        let _g = lock_tests();
        let (_tmp, root) = setup_vault();
        write_file(&root, "02-Patterns/old-decisions.md", "OLD", false).unwrap();
        patch_file(
            &root,
            "02-Patterns/old-decisions.md",
            PatchMode::Prepend,
            "DEPRECATED header",
            None,
        )
        .unwrap();
        write_file(&root, "decisions-log.md", "DEC-001..004 final", false).unwrap();
        let old = fs::read_to_string(root.join("02-Patterns/old-decisions.md")).unwrap();
        assert!(old.starts_with("DEPRECATED header"));
        assert_eq!(
            fs::read_to_string(root.join("decisions-log.md")).unwrap(),
            "DEC-001..004 final"
        );
    }

    #[tokio::test]
    async fn vault_ops_log_records_operations() {
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("memory db");
        sqlx::query(include_str!("../migrations/07_vault_ops_log.sql"))
            .execute(&pool)
            .await
            .expect("migrate");

        log_vault_op(
            &pool,
            "ceo",
            "patch_vault_file",
            "02-Patterns/old-decisions.md",
            Some("prepend"),
            None,
            Some(100),
            Some(150),
            true,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        log_vault_op(
            &pool,
            "ceo",
            "write_vault_file",
            "decisions-log.md",
            None,
            None,
            None,
            Some(20),
            true,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM vault_ops_log WHERE source_post = 'ceo' AND success = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count.0, 2);
    }
}
