//! Security Vault — UI-facing CRUD over the `security_vault` SQLite table
//! and the underlying DPAPI-encrypted Windows Credential Manager entries.
//!
//! The two stores are kept in sync by these commands:
//!   • `vault_add_secret` writes the value to Cred Manager AND upserts a
//!     metadata row in SQLite (key_name, access_level, description, ...).
//!   • `vault_remove_secret` deletes from BOTH stores.
//!   • `vault_reveal_secret` reads ONLY from Cred Manager; the SQL row never
//!     carries the value.
//!
//! All commands log the `key_name` only — never the secret value. We add a
//! `log::info!` line on reveal so the audit trail in `pnpm tauri dev` stderr
//! shows who looked up what (full audit table integration: Step 6).

use once_cell::sync::Lazy;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use tauri::State;

use crate::db::WritePool;

static KEY_NAME_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"^[a-z0-9](?:[a-z0-9_\-]{0,58}[a-z0-9])?$").unwrap());

const MIN_ACCESS: i64 = 0;
const MAX_ACCESS: i64 = 3;

#[derive(Debug, Serialize, FromRow)]
pub struct VaultMeta {
    pub id: String,
    pub key_name: String,
    pub description: Option<String>,
    pub access_level: i64,
    pub credential_target: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Deserialize)]
pub struct AddSecretInput {
    pub key_name: String,
    pub value: String,
    pub description: Option<String>,
    pub access_level: i64,
}

fn validate_key_name(key: &str) -> Result<(), String> {
    if !KEY_NAME_RE.is_match(key) {
        return Err(format!(
            "key_name '{key}' invalid (allowed: a-z 0-9 _ -, 2-60 chars, no leading/trailing dash)"
        ));
    }
    Ok(())
}

fn validate_access_level(level: i64) -> Result<(), String> {
    if !(MIN_ACCESS..=MAX_ACCESS).contains(&level) {
        return Err(format!(
            "access_level must be {MIN_ACCESS}..={MAX_ACCESS} (0=public, 1=heads, 2=ceo, 3=owner), got {level}"
        ));
    }
    Ok(())
}

#[tauri::command]
pub async fn vault_list_secrets(db: State<'_, WritePool>) -> Result<Vec<VaultMeta>, String> {
    sqlx::query_as::<_, VaultMeta>(
        "SELECT id, key_name, description, access_level, credential_target,
                created_at, updated_at
         FROM security_vault
         ORDER BY updated_at DESC",
    )
    .fetch_all(&db.0)
    .await
    .map_err(|e| format!("list secrets: {e}"))
}

#[tauri::command]
pub async fn vault_add_secret(
    input: AddSecretInput,
    db: State<'_, WritePool>,
) -> Result<VaultMeta, String> {
    validate_key_name(&input.key_name)?;
    validate_access_level(input.access_level)?;

    // 1) Write value to DPAPI / Windows Credential Manager.
    //    NB: input.value is intentionally NOT logged anywhere.
    crate::secrets::dpapi::secret_set(input.key_name.clone(), input.value).await?;

    // 2) Upsert metadata row in SQLite.
    let id = format!("vault-{}", uuid::Uuid::new_v4());
    let target = format!("mspro-ltd-corp/{}", input.key_name);
    sqlx::query(
        "INSERT INTO security_vault
            (id, key_name, description, access_level, credential_target)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT(key_name) DO UPDATE SET
            description = excluded.description,
            access_level = excluded.access_level,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(&id)
    .bind(&input.key_name)
    .bind(&input.description)
    .bind(input.access_level)
    .bind(&target)
    .execute(&db.0)
    .await
    .map_err(|e| format!("upsert vault meta: {e}"))?;

    log::info!(
        "vault_add_secret key={} level={}",
        input.key_name,
        input.access_level
    );
    fetch_vault_meta_by_key(&db, &input.key_name).await
}

#[tauri::command]
pub async fn vault_remove_secret(
    key_name: String,
    db: State<'_, WritePool>,
) -> Result<(), String> {
    validate_key_name(&key_name)?;
    // Delete from DPAPI first; ignore "not found" errors so a half-orphaned
    // record (metadata without value) can still be cleaned up via this path.
    let _ = crate::secrets::dpapi::secret_delete(key_name.clone()).await;
    sqlx::query("DELETE FROM security_vault WHERE key_name = ?")
        .bind(&key_name)
        .execute(&db.0)
        .await
        .map_err(|e| format!("delete vault meta: {e}"))?;
    log::info!("vault_remove_secret key={key_name}");
    Ok(())
}

#[tauri::command]
pub async fn vault_reveal_secret(key_name: String) -> Result<String, String> {
    validate_key_name(&key_name)?;
    // Audit trail — value is never logged, only the key name.
    log::info!("vault_reveal_secret key={key_name}");
    crate::secrets::dpapi::secret_get(key_name).await
}

async fn fetch_vault_meta_by_key(db: &WritePool, key: &str) -> Result<VaultMeta, String> {
    sqlx::query_as::<_, VaultMeta>(
        "SELECT id, key_name, description, access_level, credential_target,
                created_at, updated_at
         FROM security_vault
         WHERE key_name = ?",
    )
    .bind(key)
    .fetch_one(&db.0)
    .await
    .map_err(|e| format!("fetch vault meta: {e}"))
}
