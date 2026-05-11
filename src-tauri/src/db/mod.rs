pub mod pool;

pub use pool::{open_readonly_pool, open_write_pool, ReadonlyPool, WritePool};

use std::path::PathBuf;
use tauri::{AppHandle, Manager};

/// Resolves the path to `app.db` relative to the Tauri app data dir.
/// Mirror of the URL `tauri-plugin-sql` resolves from `sqlite:app.db`.
pub fn app_db_path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| format!("app_data_dir: {e}"))?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("create app_data_dir: {e}"))?;
    Ok(dir.join("app.db"))
}
