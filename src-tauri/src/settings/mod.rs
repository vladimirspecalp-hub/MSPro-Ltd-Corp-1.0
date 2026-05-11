//! App-level settings persisted to a JSON file in the user data dir.
//!
//! This is intentionally separate from SQLite — toggle state and UI prefs are
//! not business data. Storing them in a small JSON keeps concerns clean and
//! allows zero-cost reads at startup before the SQL plugin is initialized.
//!
//! File location:
//!   %APPDATA%\Roaming\ru.msproltd.corp\settings.json
//!
//! Atomic write strategy: write to `settings.json.tmp`, then rename over
//! `settings.json`. NTFS rename is atomic — no risk of half-written file
//! corrupting state if the process crashes mid-save.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppSettings {
    /// External Agent Gateway toggle. When true, the WebSocket gateway on
    /// 127.0.0.1:8899 is auto-started at app launch.
    pub external_agent_enabled: bool,
    /// User-friendly display name for the current Windows user (cosmetic).
    pub user_display_name: Option<String>,

    // ─── Step 4A: Hermes WSL2 bridge ──────────────────────────────────────
    /// Name of the WSL distribution that hosts Hermes. Default: "Ubuntu".
    /// On user's machine `wsl --list --quiet` may show different names
    /// (e.g. "Ubuntu-22.04") — Module 0 detection surfaces the actual list.
    #[serde(default = "default_hermes_distro")]
    pub hermes_distro: String,
    /// Hermes skill identifier passed as the first argument. Default: "/ceo".
    /// The leading slash matches Hermes' slash-command convention.
    #[serde(default = "default_hermes_skill")]
    pub hermes_skill_name: String,
    /// Hard timeout (seconds) for a single Hermes response. Default: 120.
    #[serde(default = "default_hermes_timeout")]
    pub hermes_timeout_sec: u64,
    /// LLM provider name passed to `hermes --provider`. Default: "deepseek".
    /// Custom providers must also be declared in `~/.hermes/config.yaml` and
    /// have an env var (e.g. DEEPSEEK_API_KEY) set in `~/.hermes/.env`.
    #[serde(default)]
    pub hermes_provider: Option<String>,
    /// Model identifier passed to `hermes -m`. Default: "deepseek-reasoner".
    #[serde(default)]
    pub hermes_model: Option<String>,
    /// Where the CEO chat routes its prompts:
    ///   • "hermes" — through the WSL2 Hermes bridge (DeepSeek, Ollama, etc.)
    ///   • "claude_external" — pushed over the External Agent WS gateway as
    ///     a `ceo-question` event; reply must come back via `ceo/respond`
    ///     RPC method within `claude_external_timeout_sec`.
    #[serde(default = "default_brain_mode")]
    pub brain_mode: String,
    /// How long Rust waits for an external Claude reply before falling back
    /// to an error message. Default: 600 s (10 min — Claude reasoning can be
    /// slow when the human-in-the-loop is multitasking).
    #[serde(default = "default_claude_timeout")]
    pub claude_external_timeout_sec: u64,
}

fn default_brain_mode() -> String { "hermes".to_string() }
fn default_claude_timeout() -> u64 { 600 }

fn default_hermes_distro() -> String { "Ubuntu".to_string() }
fn default_hermes_skill() -> String { "/ceo".to_string() }
fn default_hermes_timeout() -> u64 { 120 }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            external_agent_enabled: false,
            user_display_name: None,
            hermes_distro: default_hermes_distro(),
            hermes_skill_name: default_hermes_skill(),
            hermes_timeout_sec: default_hermes_timeout(),
            hermes_provider: None,
            hermes_model: None,
            brain_mode: default_brain_mode(),
            claude_external_timeout_sec: default_claude_timeout(),
        }
    }
}

#[tauri::command]
pub async fn set_brain_mode(
    mode: String,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    if mode != "hermes" && mode != "claude_external" {
        return Err(format!("invalid brain_mode '{mode}' (allowed: hermes, claude_external)"));
    }
    {
        let mut g = state.data.lock().unwrap();
        g.brain_mode = mode;
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}

/// Tauri-managed state holder so commands can mutate settings without
/// re-reading the file each time.
pub struct SettingsStore {
    pub data: Mutex<AppSettings>,
    pub path: PathBuf,
}

impl SettingsStore {
    pub fn load(app: &AppHandle) -> Self {
        let path = settings_path(app);
        let data = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                log::warn!("settings.json corrupt ({e}); falling back to defaults");
                AppSettings::default()
            }),
            Err(_) => AppSettings::default(),
        };
        log::info!("settings loaded from {}", path.display());
        Self {
            data: Mutex::new(data),
            path,
        }
    }

    pub fn save(&self) -> std::io::Result<()> {
        let snapshot = self.data.lock().unwrap().clone();
        let json = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
        // Atomic write: tmp → rename.
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp_path = self.path.with_extension("json.tmp");
        std::fs::write(&tmp_path, json)?;
        std::fs::rename(&tmp_path, &self.path)?;
        Ok(())
    }
}

fn settings_path(app: &AppHandle) -> PathBuf {
    app.path()
        .app_data_dir()
        .expect("app_data_dir resolution must succeed")
        .join("settings.json")
}

#[tauri::command]
pub async fn get_settings(state: tauri::State<'_, SettingsStore>) -> Result<AppSettings, String> {
    Ok(state.data.lock().unwrap().clone())
}

#[tauri::command]
pub async fn set_external_agent_enabled(
    enabled: bool,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    {
        let mut guard = state.data.lock().unwrap();
        guard.external_agent_enabled = enabled;
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}
