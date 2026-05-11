//! Backup of the currently-running exe before applying an update.
//!
//! Layout:
//!   %LOCALAPPDATA%\ru.msproltd.corp\backups\
//!     v1.0.0-2026-05-10T12-34-56Z.exe
//!     v1.0.1-2026-05-10T18-22-11Z.exe
//!     ...
//!
//! Retention: keep the 3 most recent backups, delete older ones to bound disk
//! usage at ~225 MB (3 × ~75 MB exe).

use serde::Serialize;
use std::path::{Path, PathBuf};

const BACKUP_RETENTION: usize = 3;

#[derive(Debug, Clone, Serialize)]
pub struct BackupEntry {
    /// Filename, e.g. "v1.0.0-2026-05-10T12-34-56Z.exe".
    pub filename: String,
    /// Parsed version string, e.g. "1.0.0".
    pub version: String,
    /// ISO-8601 timestamp parsed from filename.
    pub created_at: String,
    /// Bytes on disk.
    pub size_bytes: u64,
    /// Absolute path (for rollback).
    pub path: PathBuf,
}

/// Resolves the backup directory under %LOCALAPPDATA%, creating it if missing.
pub fn backup_dir() -> std::io::Result<PathBuf> {
    let local = std::env::var("LOCALAPPDATA")
        .map_err(|e| std::io::Error::other(format!("LOCALAPPDATA unset: {e}")))?;
    let dir = PathBuf::from(local).join("ru.msproltd.corp").join("backups");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Copies the currently-running exe into the backup dir with a timestamped
/// filename. Returns the absolute path of the created backup.
pub fn create_backup(current_version: &str) -> std::io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = backup_dir()?;
    let ts = chrono::Utc::now().format("%Y-%m-%dT%H-%M-%SZ");
    let filename = format!("v{current_version}-{ts}.exe");
    let dst = dir.join(&filename);
    std::fs::copy(&exe, &dst)?;
    log::info!("backup created: {}", dst.display());
    cleanup_old_backups(BACKUP_RETENTION).ok();
    Ok(dst)
}

/// Lists existing backups, newest first.
pub fn list_backups() -> std::io::Result<Vec<BackupEntry>> {
    let dir = backup_dir()?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("exe") {
            continue;
        }
        let filename = match path.file_name().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };
        let (version, ts) = match parse_backup_name(&filename) {
            Some(v) => v,
            None => continue,
        };
        let size_bytes = entry.metadata()?.len();
        entries.push(BackupEntry {
            filename,
            version,
            created_at: ts,
            size_bytes,
            path,
        });
    }
    // Newest first (lexicographic on ISO-8601 timestamp ≡ chronological).
    entries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(entries)
}

/// Keeps the `keep` newest backups; deletes the rest.
pub fn cleanup_old_backups(keep: usize) -> std::io::Result<()> {
    let entries = list_backups()?;
    for old in entries.into_iter().skip(keep) {
        if let Err(e) = std::fs::remove_file(&old.path) {
            log::warn!("failed to cleanup old backup {}: {e}", old.path.display());
        } else {
            log::info!("cleaned up old backup: {}", old.filename);
        }
    }
    Ok(())
}

/// Validates that a candidate file is a real Windows PE exe (sanity check
/// before swapping over the running app — protects against corrupted backup
/// files that could brick the installation).
pub fn validate_pe_exe(path: &Path) -> Result<(), String> {
    let metadata =
        std::fs::metadata(path).map_err(|e| format!("backup not found: {e}"))?;
    let size = metadata.len();
    if size < 1_000_000 {
        return Err(format!(
            "backup too small ({size} bytes); expected ≥ 1 MB"
        ));
    }
    let mut buf = [0u8; 2];
    use std::io::Read;
    let mut file = std::fs::File::open(path).map_err(|e| format!("open backup: {e}"))?;
    file.read_exact(&mut buf)
        .map_err(|e| format!("read magic: {e}"))?;
    // PE magic = "MZ" (0x4D 0x5A) — DOS header signature.
    if buf != [0x4D, 0x5A] {
        return Err(format!(
            "backup is not a Windows PE exe (magic={:02X}{:02X}, expected 4D5A)",
            buf[0], buf[1]
        ));
    }
    Ok(())
}

/// Parses "v1.0.0-2026-05-10T12-34-56Z.exe" → ("1.0.0", "2026-05-10T12-34-56Z").
fn parse_backup_name(filename: &str) -> Option<(String, String)> {
    let without_ext = filename.strip_suffix(".exe")?;
    let without_v = without_ext.strip_prefix('v')?;
    // Find the first '-' followed by a 4-digit year. Simpler: split on first
    // occurrence of "-20" (date prefix). Sufficient for our naming scheme.
    let dash = without_v.find("-20")?;
    let version = without_v[..dash].to_string();
    let ts = without_v[dash + 1..].to_string();
    Some((version, ts))
}

#[tauri::command]
pub async fn list_backups_cmd() -> Result<Vec<BackupEntry>, String> {
    list_backups().map_err(|e| format!("list backups: {e}"))
}
