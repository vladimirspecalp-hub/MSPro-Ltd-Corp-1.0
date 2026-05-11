//! Update checking and installation flow.
//!
//! Workflow:
//!   1. UI calls `check_for_update` → returns Option<UpdateInfo>.
//!   2. UI shows "Update to v{x}" button + changelog.
//!   3. UI calls `install_update_with_backup`:
//!       a. Create backup of current exe (see backup.rs).
//!       b. Tauri-plugin-updater downloads + installs new exe (atomic
//!          swap-on-restart handled by the plugin).
//!       c. App restarts on the new version.
//!   4. If something is broken in the new version — UI calls
//!      `rollback_to(version)` (see rollback.rs).

use serde::Serialize;
use tauri::AppHandle;
use tauri_plugin_updater::UpdaterExt;

use super::backup;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current_version: String,
    pub date: Option<String>,
    pub body: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateProgressEvent {
    pub downloaded: u64,
    pub total: u64,
    pub percent: u32,
}

#[tauri::command]
pub async fn check_for_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    let current = app.package_info().version.to_string();
    let updater = app
        .updater()
        .map_err(|e| format!("updater init: {e}"))?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
            current_version: current,
            date: update.date.map(|d| d.to_string()),
            body: update.body.clone(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(format!("check: {e}")),
    }
}

/// Downloads + installs the available update, creating a backup of the
/// currently-running exe first. Emits `update-progress` events to the UI
/// while downloading. After install, calls `app.restart()`.
#[tauri::command]
pub async fn install_update_with_backup(app: AppHandle) -> Result<(), String> {
    let current_version = app.package_info().version.to_string();
    log::info!("install_update_with_backup: backing up v{current_version}");

    // Backup BEFORE we ask Tauri to download/install. If the backup fails,
    // we abort the update entirely — better to keep the working version.
    backup::create_backup(&current_version)
        .map_err(|e| format!("backup current exe: {e}"))?;

    let updater = app
        .updater()
        .map_err(|e| format!("updater init: {e}"))?;
    let update = updater
        .check()
        .await
        .map_err(|e| format!("check: {e}"))?
        .ok_or_else(|| "no update available".to_string())?;

    log::info!(
        "downloading + installing update v{} (size unknown until first chunk)",
        update.version
    );

    let app_for_progress = app.clone();
    update
        .download_and_install(
            move |chunk_length, content_length| {
                let downloaded = chunk_length as u64;
                let total = content_length.unwrap_or(0);
                let percent = if total > 0 {
                    ((downloaded as f64 / total as f64) * 100.0) as u32
                } else {
                    0
                };
                let _ = tauri::Emitter::emit(
                    &app_for_progress,
                    "update-progress",
                    UpdateProgressEvent {
                        downloaded,
                        total,
                        percent,
                    },
                );
            },
            || {
                log::info!("update download finished");
            },
        )
        .await
        .map_err(|e| format!("download_and_install: {e}"))?;

    log::info!("update installed; restarting…");
    app.restart();
}
