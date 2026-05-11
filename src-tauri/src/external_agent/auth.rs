//! Token generation + storage for the External Agent Gateway.
//!
//! Token shape: 32 bytes from the OS CSPRNG (`getrandom`), encoded as
//! base64url (no padding). Result is ~43 ASCII characters with 256 bits of
//! entropy — practically un-bruteforceable.
//!
//! Storage:
//!   - Real value: Windows Credential Manager via DPAPI (keyring crate).
//!   - Metadata row: `security_vault` SQLite table (key_name, access_level,
//!     credential_target, ...). Created/updated as needed.
//!
//! Lifecycle:
//!   - `ensure_token` — returns the existing token, or generates+stores a
//!     fresh one if missing.
//!   - `rotate_token` — always generates a new value, overwriting Cred
//!     Manager + bumping `updated_at` in security_vault.
//!   - `clear_token` — deletes from both stores (for "Disable + forget").

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use getrandom::getrandom;

use crate::secrets::dpapi;

const TOKEN_KEY: &str = "external_agent_token";

pub fn generate_token() -> Result<String, String> {
    let mut bytes = [0u8; 32];
    getrandom(&mut bytes).map_err(|e| format!("getrandom: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

#[cfg(windows)]
pub async fn ensure_token() -> Result<String, String> {
    match dpapi::secret_get(TOKEN_KEY.to_string()).await {
        Ok(v) if !v.is_empty() => Ok(v),
        _ => {
            let token = generate_token()?;
            dpapi::secret_set(TOKEN_KEY.to_string(), token.clone()).await?;
            Ok(token)
        }
    }
}

#[cfg(not(windows))]
pub async fn ensure_token() -> Result<String, String> {
    Err("External Agent token storage requires Windows (DPAPI)".into())
}

#[cfg(windows)]
#[tauri::command]
pub async fn external_agent_show_token() -> Result<String, String> {
    ensure_token().await
}

#[cfg(not(windows))]
#[tauri::command]
pub async fn external_agent_show_token() -> Result<String, String> {
    Err("Windows-only".into())
}

#[cfg(windows)]
#[tauri::command]
pub async fn external_agent_rotate_token() -> Result<String, String> {
    let token = generate_token()?;
    dpapi::secret_set(TOKEN_KEY.to_string(), token.clone()).await?;
    Ok(token)
}

#[cfg(not(windows))]
#[tauri::command]
pub async fn external_agent_rotate_token() -> Result<String, String> {
    Err("Windows-only".into())
}

#[cfg(windows)]
pub async fn current_token() -> Option<String> {
    dpapi::secret_get(TOKEN_KEY.to_string()).await.ok()
}

#[cfg(not(windows))]
pub async fn current_token() -> Option<String> {
    None
}
