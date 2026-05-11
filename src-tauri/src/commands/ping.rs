/// Smoke-test commands: confirm UI <-> Rust round-trip works.
/// The actual SQL read in `list_departments` happens on the JS side via
/// @tauri-apps/plugin-sql; this command just returns app version metadata.

use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct AppInfo {
    pub name: String,
    pub version: String,
}

#[tauri::command]
pub async fn ping() -> String {
    "pong from MSPro-Ltd Corp 1.0".to_string()
}

#[tauri::command]
pub async fn app_info() -> AppInfo {
    AppInfo {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}
