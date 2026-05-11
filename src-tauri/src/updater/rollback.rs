//! Rollback to a previous version using a stored backup.
//!
//! Windows blocks writes to a running .exe, so the swap is delegated to a
//! tiny external helper (`mspro-rollback-helper.exe`) that:
//!   1. Waits for our PID to exit (10 s timeout).
//!   2. Retries the file copy every 500 ms (×20 = 10 s) — Windows can hold
//!      a mandatory lock on a closing exe slightly longer than expected.
//!   3. Spawns the now-restored exe.
//!
//! This module's responsibility is to:
//!   1. Validate the requested backup (PE magic + size).
//!   2. Locate the helper exe (bundled in resources/).
//!   3. Spawn the helper with our PID + paths.
//!   4. Call `app.exit(0)` to release the file lock.

use std::path::PathBuf;
use std::process::Command;
use tauri::{AppHandle, Manager};

use super::backup::{self, BackupEntry};

#[tauri::command]
pub async fn rollback_to(app: AppHandle, version: String) -> Result<(), String> {
    let backups = backup::list_backups().map_err(|e| format!("list backups: {e}"))?;
    let target_backup: BackupEntry = backups
        .into_iter()
        .find(|b| b.version == version)
        .ok_or_else(|| format!("backup for v{version} not found"))?;

    log::info!(
        "rollback_to v{version}: validating {}",
        target_backup.path.display()
    );
    backup::validate_pe_exe(&target_backup.path)?;

    let current_exe =
        std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    let helper_path = locate_helper(&app)?;
    let pid = std::process::id();

    log::info!(
        "rollback_to v{version}: spawning helper {} (pid={pid})",
        helper_path.display()
    );

    Command::new(&helper_path)
        .arg("--target")
        .arg(&current_exe)
        .arg("--source")
        .arg(&target_backup.path)
        .arg("--pid")
        .arg(pid.to_string())
        .spawn()
        .map_err(|e| format!("spawn helper: {e}"))?;

    log::info!("rollback_to v{version}: exiting to release exe lock");
    app.exit(0);
    // Unreachable in practice — `app.exit` terminates the process.
    Ok(())
}

fn locate_helper(app: &AppHandle) -> Result<PathBuf, String> {
    // 1. Production: helper is bundled as a resource.
    if let Ok(resource_dir) = app.path().resource_dir() {
        let candidate = resource_dir
            .join("resources")
            .join("mspro-rollback-helper.exe");
        if candidate.exists() {
            return Ok(candidate);
        }
        // Tauri v2 sometimes flattens resources into the resource_dir root.
        let flat = resource_dir.join("mspro-rollback-helper.exe");
        if flat.exists() {
            return Ok(flat);
        }
    }
    // 2. Dev mode: helper sits next to its build output, copied via build.rs.
    let dev_path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()))
        .map(|d| d.join("mspro-rollback-helper.exe"));
    if let Some(dev) = dev_path {
        if dev.exists() {
            return Ok(dev);
        }
    }
    // 3. Source tree (cargo run --bin) — fallback during development.
    let source_tree = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("resources")
        .join("mspro-rollback-helper.exe");
    if source_tree.exists() {
        return Ok(source_tree);
    }
    Err("mspro-rollback-helper.exe not found in resource_dir or dev paths".to_string())
}
