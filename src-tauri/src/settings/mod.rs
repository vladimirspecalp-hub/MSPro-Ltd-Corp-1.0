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

    // ─── Step 10: Two-circuit brain (Claude CLI + Qwen local) ─────────────
    /// Where the CEO chat routes its prompts:
    ///   • "claude_cli"      — local Claude Code CLI (primary, since v1.0.15)
    ///   • "qwen_local"      — local OpenAI-compat endpoint (Ollama / LM Studio)
    ///   • "claude_external" — pushed over External Agent WS (legacy, hidden in UI)
    #[serde(default = "default_brain_mode")]
    pub brain_mode: String,
    /// How long Rust waits for an external Claude reply via WS before
    /// falling back. Default: 600 s.
    #[serde(default = "default_claude_external_timeout")]
    pub claude_external_timeout_sec: u64,

    /// Path to the `claude` executable. Default: `"claude"` (PATH lookup).
    /// If installed in WSL only, set to `"wsl claude"`.
    #[serde(default = "default_claude_cli_path")]
    pub claude_cli_path: String,
    /// Claude model id passed via `--model`. Default: `claude-opus-4-7`.
    #[serde(default = "default_claude_cli_model")]
    pub claude_cli_model: String,
    /// Hard timeout (seconds) for a single `claude --print` invocation.
    #[serde(default = "default_claude_cli_timeout")]
    pub claude_cli_timeout_sec: u64,

    /// OpenAI-compatible endpoint for local Qwen 3 (Ollama default 11434, LM Studio 1234).
    #[serde(default = "default_qwen_endpoint")]
    pub qwen_endpoint: String,
    /// Qwen model id. Ollama-style `qwen3:32b` by default.
    #[serde(default = "default_qwen_model")]
    pub qwen_model: String,
    /// Hard timeout (seconds) for a single Qwen response.
    #[serde(default = "default_qwen_timeout")]
    pub qwen_timeout_sec: u64,

    /// When true and `brain_mode == "claude_cli"`, on Claude failure auto-fall
    /// back to Qwen with a system warning in chat.
    #[serde(default = "default_auto_fallback_qwen")]
    pub auto_fallback_qwen: bool,

    /// Сколько последних turn'ов диалога подмешивать в prompt CEO (0 = stateless).
    #[serde(default = "default_chat_history_turns")]
    pub chat_history_turns: u32,

    // ─── v1.0.22 Фаза 11C: Intelligent Dispatcher Hub ─────────────────────
    /// Master-toggle: Диспетчер активен и автоматически обрабатывает raw_request.
    /// Если false — задачи копятся в Inbox, Владелец может ручную маршрутизацию.
    #[serde(default = "default_dispatcher_enabled")]
    pub dispatcher_enabled: bool,
    /// "qwen_primary" (default — дёшево для простой маршрутизации) |
    /// "claude_primary" (всегда Claude Sonnet — точнее, дороже).
    #[serde(default = "default_dispatcher_brain_mode")]
    pub dispatcher_brain_mode: String,
    /// Qwen model для Диспетчера. Может быть та же что у Гендира (qwen3:14b).
    #[serde(default = "default_dispatcher_qwen_model")]
    pub dispatcher_qwen_model: String,
    /// Claude model для Диспетчера. По умолчанию Sonnet — он дешевле Opus
    /// и достаточно сильный для prompt rewriting / decomposition.
    #[serde(default = "default_dispatcher_claude_model")]
    pub dispatcher_claude_model: String,
    /// При qwen_primary: если Qwen упал/выдал мусор — fallback на Claude.
    #[serde(default = "default_dispatcher_auto_fallback")]
    pub dispatcher_auto_fallback_claude: bool,
    /// Cap на retry. После 3-й попытки Диспетчер автоматически reject_task.
    #[serde(default = "default_dispatcher_max_attempts")]
    pub dispatcher_max_attempts: u32,
    /// Hard timeout (sec) на одну попытку routing-вызова.
    #[serde(default = "default_dispatcher_routing_timeout")]
    pub dispatcher_routing_timeout_sec: u64,

    // ---- v1.0.30: Context window sizes for dynamic history budget ----
    /// Claude context window size in tokens. Used by context_assembler to
    /// calculate how much chat history fits alongside system prompt.
    #[serde(default = "default_claude_context_tokens")]
    pub claude_context_tokens: u32,
    /// Qwen context window size in tokens.
    #[serde(default = "default_qwen_context_tokens")]
    pub qwen_context_tokens: u32,

    // ---- v1.0.24 Phase 11B-1: Post Executor ----
    /// Hard timeout (sec) на spawn пост-агента (claude.exe в Outbox sandbox).
    /// Default 600 (10 минут) — Claude может думать + писать .docx/.xlsx через MCP.
    #[serde(default = "default_post_executor_timeout")]
    pub post_executor_timeout_sec: u64,
    /// Сколько пост-агентов могут работать одновременно (per app).
    /// Default 3 — каждый = отдельный claude.exe, RAM ~300-500 MB.
    #[serde(default = "default_post_executor_max_concurrent")]
    pub post_executor_max_concurrent: u32,
}

fn default_brain_mode() -> String { "claude_cli".to_string() }
fn default_claude_external_timeout() -> u64 { 600 }

fn default_claude_cli_path() -> String { "claude".to_string() }
fn default_claude_cli_model() -> String { "claude-opus-4-7".to_string() }
fn default_claude_cli_timeout() -> u64 { 180 }

fn default_qwen_endpoint() -> String { "http://localhost:11434/v1".to_string() }
fn default_qwen_model() -> String { "qwen3:14b".to_string() }
fn default_qwen_timeout() -> u64 { 120 }

fn default_auto_fallback_qwen() -> bool { true }
fn default_chat_history_turns() -> u32 { 20 }

fn default_claude_context_tokens() -> u32 { 200_000 }
fn default_qwen_context_tokens() -> u32 { 32_000 }

fn default_dispatcher_enabled() -> bool { true }
fn default_dispatcher_brain_mode() -> String { "qwen_primary".to_string() }
fn default_dispatcher_qwen_model() -> String { "qwen3:14b".to_string() }
fn default_dispatcher_claude_model() -> String { "claude-sonnet-4-7".to_string() }
fn default_dispatcher_auto_fallback() -> bool { true }
fn default_dispatcher_max_attempts() -> u32 { 3 }
fn default_dispatcher_routing_timeout() -> u64 { 60 }

fn default_post_executor_timeout() -> u64 { 600 }
fn default_post_executor_max_concurrent() -> u32 { 3 }

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            external_agent_enabled: false,
            user_display_name: None,
            brain_mode: default_brain_mode(),
            claude_external_timeout_sec: default_claude_external_timeout(),
            claude_cli_path: default_claude_cli_path(),
            claude_cli_model: default_claude_cli_model(),
            claude_cli_timeout_sec: default_claude_cli_timeout(),
            qwen_endpoint: default_qwen_endpoint(),
            qwen_model: default_qwen_model(),
            qwen_timeout_sec: default_qwen_timeout(),
            auto_fallback_qwen: default_auto_fallback_qwen(),
            chat_history_turns: default_chat_history_turns(),
            claude_context_tokens: default_claude_context_tokens(),
            qwen_context_tokens: default_qwen_context_tokens(),
            dispatcher_enabled: default_dispatcher_enabled(),
            dispatcher_brain_mode: default_dispatcher_brain_mode(),
            dispatcher_qwen_model: default_dispatcher_qwen_model(),
            dispatcher_claude_model: default_dispatcher_claude_model(),
            dispatcher_auto_fallback_claude: default_dispatcher_auto_fallback(),
            dispatcher_max_attempts: default_dispatcher_max_attempts(),
            dispatcher_routing_timeout_sec: default_dispatcher_routing_timeout(),
            post_executor_timeout_sec: default_post_executor_timeout(),
            post_executor_max_concurrent: default_post_executor_max_concurrent(),
        }
    }
}

#[tauri::command]
pub async fn set_brain_mode(
    mode: String,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    if !matches!(mode.as_str(), "claude_cli" | "qwen_local" | "claude_external") {
        return Err(format!(
            "invalid brain_mode '{mode}' (allowed: claude_cli, qwen_local, claude_external)"
        ));
    }
    {
        let mut g = state.data.lock().unwrap();
        g.brain_mode = mode;
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}

/// Шаг 10: универсальный setter для любого скалярного string-поля настроек.
/// UI-Settings экран его вызывает для claude_cli_path / claude_cli_model /
/// qwen_endpoint / qwen_model. Это дешевле чем плодить 6 отдельных Tauri-команд.
#[tauri::command]
pub async fn set_brain_string_field(
    field: String,
    value: String,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    {
        let mut g = state.data.lock().unwrap();
        match field.as_str() {
            "claude_cli_path" => g.claude_cli_path = value,
            "claude_cli_model" => g.claude_cli_model = value,
            "qwen_endpoint" => g.qwen_endpoint = value,
            "qwen_model" => g.qwen_model = value,
            // v1.0.22 — dispatcher fields
            "dispatcher_qwen_model" => g.dispatcher_qwen_model = value,
            "dispatcher_claude_model" => g.dispatcher_claude_model = value,
            "dispatcher_brain_mode" => {
                if !matches!(value.as_str(), "qwen_primary" | "claude_primary") {
                    return Err(format!(
                        "invalid dispatcher_brain_mode '{value}' (allowed: qwen_primary, claude_primary)"
                    ));
                }
                g.dispatcher_brain_mode = value;
            }
            _ => return Err(format!("unknown field '{field}'")),
        }
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}

#[tauri::command]
pub async fn set_auto_fallback_qwen(
    enabled: bool,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    {
        let mut g = state.data.lock().unwrap();
        g.auto_fallback_qwen = enabled;
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}

/// v1.0.22 — универсальный setter для bool-полей Диспетчера.
#[tauri::command]
pub async fn set_dispatcher_bool_field(
    field: String,
    value: bool,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    {
        let mut g = state.data.lock().unwrap();
        match field.as_str() {
            "dispatcher_enabled" => g.dispatcher_enabled = value,
            "dispatcher_auto_fallback_claude" => g.dispatcher_auto_fallback_claude = value,
            _ => return Err(format!("unknown bool field '{field}'")),
        }
    }
    state.save().map_err(|e| format!("settings save: {e}"))
}

/// v1.0.22 — setter для max_attempts (1..=10).
#[tauri::command]
pub async fn set_dispatcher_max_attempts(
    attempts: u32,
    state: tauri::State<'_, SettingsStore>,
) -> Result<(), String> {
    if !(1..=10).contains(&attempts) {
        return Err(format!("max_attempts {attempts} вне диапазона 1..=10"));
    }
    {
        let mut g = state.data.lock().unwrap();
        g.dispatcher_max_attempts = attempts;
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
        let mut data = match std::fs::read_to_string(&path) {
            Ok(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
                log::warn!("settings.json corrupt ({e}); falling back to defaults");
                AppSettings::default()
            }),
            Err(_) => AppSettings::default(),
        };
        // Step 10 migration: legacy brain_mode="hermes" → "claude_cli".
        // Old settings.json with hermes_* fields загружается серде'м мягко
        // (поля просто игнорируются — их больше нет в struct).
        if data.brain_mode == "hermes" {
            log::info!("Settings migration: brain_mode 'hermes' → 'claude_cli'");
            data.brain_mode = default_brain_mode();
        }
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
