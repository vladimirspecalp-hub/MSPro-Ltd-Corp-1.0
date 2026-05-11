//! Tauri-команды для работы с файловой памятью Гендира (см. `vault.rs`).
//!
//! Намеренно отделено от `commands/vault.rs` (Security Vault, Шаг 5),
//! чтобы не путать «memory vault» и «secrets vault».

use serde::Deserialize;
use tauri::State;

use crate::vault::{self, VaultState, PATTERNS_DIR, WINS_DIR};

#[derive(Debug, Deserialize)]
pub struct SaveVaultInput {
    pub title: String,
    pub content: String,
}

/// Сохраняет markdown в `<Vault>/02-Patterns/<slug-from-title>.md`. Перезаписывает.
#[tauri::command]
pub async fn save_pattern(
    input: SaveVaultInput,
    v: State<'_, VaultState>,
) -> Result<String, String> {
    vault::save_to(&v.root, PATTERNS_DIR, &input.title, &input.content)
        .map(|p| p.display().to_string())
}

/// Сохраняет markdown в `<Vault>/04-Wins/<slug-from-title>.md`. Перезаписывает.
#[tauri::command]
pub async fn save_win(
    input: SaveVaultInput,
    v: State<'_, VaultState>,
) -> Result<String, String> {
    vault::save_to(&v.root, WINS_DIR, &input.title, &input.content)
        .map(|p| p.display().to_string())
}

/// Debug: показывает Vault-блок ровно так, как его увидит CEO в system prompt.
/// Используется кнопкой «🧠 Показать память Гендира» в Settings.
#[tauri::command]
pub async fn get_vault_preview(v: State<'_, VaultState>) -> Result<String, String> {
    vault::read_vault_context(v.root.clone()).await
}
