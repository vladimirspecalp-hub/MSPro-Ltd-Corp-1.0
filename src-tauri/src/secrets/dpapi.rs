//! Secure secret storage backed by Windows Credential Manager (DPAPI underneath).
//!
//! The SQLite `security_vault` table holds only metadata
//! (`key_name`, `access_level`, `credential_target`). Actual secret bytes live
//! in Windows Credential Manager, encrypted by DPAPI keyed to the current
//! Windows user account — copying `app.db` does NOT leak any secret.
//!
//! Frontend workflow:
//!   1. `secret_set("n8n_api_key", "sk-xxxx")` — Rust writes to Cred Manager.
//!   2. UI separately INSERTs metadata row into `security_vault` (key_name,
//!      access_level, credential_target).
//!   3. To read: UI lists metadata from SQLite, then calls `secret_get(...)`.

use serde::Serialize;

/// Service name used as the prefix in Windows Credential Manager.
/// Final stored target reads as `mspro-ltd-corp:<key_name>` (Windows
/// formatting; library handles the colon under the hood).
const SERVICE_PREFIX: &str = "mspro-ltd-corp";

#[derive(Debug, Serialize)]
pub struct SecretMeta {
    pub key_name: String,
    /// Display path users see in Credential Manager UI.
    pub credential_target: String,
}

#[cfg(windows)]
fn make_target(key_name: &str) -> String {
    format!("{SERVICE_PREFIX}/{key_name}")
}

#[cfg(windows)]
fn entry_for(key_name: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE_PREFIX, key_name)
        .map_err(|e| format!("keyring init: {e}"))
}

#[cfg(windows)]
#[tauri::command]
pub async fn secret_set(key_name: String, value: String) -> Result<SecretMeta, String> {
    let entry = entry_for(&key_name)?;
    entry
        .set_password(&value)
        .map_err(|e| format!("keyring set: {e}"))?;
    Ok(SecretMeta {
        credential_target: make_target(&key_name),
        key_name,
    })
}

#[cfg(windows)]
#[tauri::command]
pub async fn secret_get(key_name: String) -> Result<String, String> {
    let entry = entry_for(&key_name)?;
    entry
        .get_password()
        .map_err(|e| format!("keyring get: {e}"))
}

#[cfg(windows)]
#[tauri::command]
pub async fn secret_delete(key_name: String) -> Result<(), String> {
    let entry = entry_for(&key_name)?;
    entry
        .delete_credential()
        .map_err(|e| format!("keyring delete: {e}"))?;
    Ok(())
}

// Non-Windows stubs so the build still works on macOS/Linux dev boxes
// (DPAPI is Windows-only; on other platforms `keyring` falls back to
// Secret Service / Keychain — but our v1.0 target is Windows only).
#[cfg(not(windows))]
#[tauri::command]
pub async fn secret_set(_key_name: String, _value: String) -> Result<SecretMeta, String> {
    Err("DPAPI secret storage is only supported on Windows".to_string())
}

#[cfg(not(windows))]
#[tauri::command]
pub async fn secret_get(_key_name: String) -> Result<String, String> {
    Err("DPAPI secret storage is only supported on Windows".to_string())
}

#[cfg(not(windows))]
#[tauri::command]
pub async fn secret_delete(_key_name: String) -> Result<(), String> {
    Err("DPAPI secret storage is only supported on Windows".to_string())
}
