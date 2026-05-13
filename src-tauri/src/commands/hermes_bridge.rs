//! Hermes Agent ↔ MSPro-Ltd Corp bridge over WSL2.
//!
//! Three responsibilities:
//!   1. **Detect** — discover whether WSL exists, the configured distro is
//!      reachable, Hermes is installed in it, and the `/ceo` skill is
//!      registered. Surface the result to the UI as a typed enum so the
//!      `HermesStatusBadge` can render the right action.
//!   2. **Spawn** — launch `wsl.exe -d <distro> -- hermes <skill> --stdin-json`
//!      with the system + user payload piped on stdin. We use
//!      `tokio::process::Command::kill_on_drop(true)` so any panic, cancel
//!      or window-close kills the WSL child immediately (anti-zombie).
//!   3. **Stream** — read child stdout line-by-line, emit `ceo-chunk` events
//!      to the UI for typing-effect rendering, accumulate the full response
//!      for SQLite persistence. Hard 120 s timeout enforced via
//!      `tokio::select!`.
//!
//! Cancellation has three independent escape hatches (R5 in the plan):
//!   • `cancel: AtomicBool` checked before each line read.
//!   • `Child::kill().await` invoked on cancel/timeout.
//!   • `kill_on_drop(true)` ensures SIGKILL even if the await chain panics.

use std::process::Stdio;

/// Windows-only flag for CreateProcess: подавляет создание видимого консольного окна
/// у дочернего процесса. На Unix не определён, на Windows импортируется через
/// `os::windows::process::CommandExt::creation_flags`.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Скрывает console-окно при spawn'е `wsl.exe`. Без этого Windows показывает
/// чёрное окно cmd.exe для каждого WSL-вызова (мерцает при каждом chat-turn).
/// Tokio Command exposes `creation_flags` через windows-расширение CommandExt.
fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

use crate::settings::{AppSettings, SettingsStore};

const WSL_EXE: &str = "wsl.exe";

/// Tauri-managed lifecycle state for the *currently running* Hermes
/// generation. Only one CEO request runs at a time (UI prevents concurrent
/// sends); this struct lets the cancel command find and kill it.
#[derive(Default)]
pub struct ChatLifecycle {
    pub current_child_pid: AsyncMutex<Option<u32>>,
    pub cancel: Arc<AtomicBool>,
}

#[derive(Debug, Serialize, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HermesStatus {
    /// Hermes is reachable and the `/ceo` skill exists.
    Available {
        distro: String,
        version: String,
        skill_path: Option<String>,
    },
    /// Hermes binary present but the configured skill is not.
    SkillMissing {
        distro: String,
        version: String,
        configured_skill: String,
    },
    /// WSL up + distro present, but `hermes` is not installed.
    HermesNotInstalled {
        distro: String,
    },
    /// Configured distro name is not in `wsl --list --quiet` output.
    DistroNotFound {
        configured_distro: String,
        available: Vec<String>,
    },
    /// `wsl.exe` itself failed (not installed, service not running, etc.).
    WslNotAvailable {
        error: String,
    },
}

#[tauri::command]
pub async fn detect_hermes_status(
    settings: State<'_, SettingsStore>,
) -> Result<HermesStatus, String> {
    let snapshot = settings.data.lock().unwrap().clone();
    Ok(detect_hermes_status_inner(&snapshot).await)
}

pub async fn detect_hermes_status_inner(settings: &AppSettings) -> HermesStatus {
    // 1. Is WSL itself reachable? `wsl --list --quiet` is fast and harmless.
    let mut wsl_list_cmd = Command::new(WSL_EXE);
    hide_console(&mut wsl_list_cmd);
    let wsl_list = match wsl_list_cmd
        .args(["--list", "--quiet"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
    {
        Ok(o) if o.status.success() => o,
        Ok(o) => {
            return HermesStatus::WslNotAvailable {
                error: String::from_utf8_lossy(&o.stderr).trim().to_string(),
            };
        }
        Err(e) => {
            return HermesStatus::WslNotAvailable {
                error: format!("wsl.exe spawn: {e}"),
            };
        }
    };
    // wsl --list --quiet is encoded UTF-16 LE on Windows; strip BOM + NULs.
    let list_text = decode_wsl_utf16(&wsl_list.stdout);
    let available: Vec<String> = list_text
        .lines()
        .map(|l| l.trim().trim_end_matches('\r').to_string())
        .filter(|l| !l.is_empty())
        .collect();

    let distro = &settings.hermes_distro;
    if !available.iter().any(|d| d.eq_ignore_ascii_case(distro)) {
        return HermesStatus::DistroNotFound {
            configured_distro: distro.clone(),
            available,
        };
    }

    // 2. Is hermes installed inside the distro?
    let which = wsl_run(distro, &["which", "hermes"]).await;
    if !which.success {
        return HermesStatus::HermesNotInstalled {
            distro: distro.clone(),
        };
    }

    // 3. Hermes version (best-effort — empty string if it fails).
    let version_out = wsl_run(distro, &["hermes", "--version"]).await;
    let version = if version_out.success {
        version_out
            .stdout
            .lines()
            .next()
            .unwrap_or("unknown")
            .to_string()
    } else {
        "unknown".to_string()
    };

    // 4. Skill check: ~/.hermes/skills/<name>/SKILL.md
    //    (skill name with leading "/" is normalized — Hermes uses "/ceo"
    //    on the CLI but the on-disk dir is just "ceo".)
    let skill_dir = settings
        .hermes_skill_name
        .trim_start_matches('/')
        .trim_end_matches('/');
    let skill_path = format!("$HOME/.hermes/skills/{skill_dir}/SKILL.md");
    let skill_check = wsl_run(distro, &["bash", "-lc", &format!("ls -1 {skill_path} 2>/dev/null")])
        .await;

    if skill_check.success && !skill_check.stdout.trim().is_empty() {
        return HermesStatus::Available {
            distro: distro.clone(),
            version,
            skill_path: Some(skill_check.stdout.trim().to_string()),
        };
    }

    HermesStatus::SkillMissing {
        distro: distro.clone(),
        version,
        configured_skill: settings.hermes_skill_name.clone(),
    }
}

/// Spawns Hermes with stdin-piped JSON envelope and stdout/stderr captured.
///
/// Returns the live `Child` so the caller can stream stdout, race a timeout
/// and send cancel signals. The caller MUST keep the `Child` until done —
/// dropping it triggers `kill_on_drop`, terminating the process.
pub async fn spawn_hermes(
    system: &str,
    user: &str,
    settings: &AppSettings,
) -> Result<Child, String> {
    // Hermes Agent v0.13 doesn't support a stdin JSON envelope, so we fold
    // the system prompt into the user message via a clearly-delimited block
    // and invoke its one-shot mode (`-z`).
    //
    // Wire format we hand to Hermes is plain text:
    //   <system context>
    //   ---
    //   USER: <message>
    //
    // The /ceo skill (~/.hermes/skills/ceo/SKILL.md) instructs the model to
    // treat anything before `---` as operational context and anything after
    // `USER:` as the request.
    let combined_prompt = format!("{system}\n\n---\nUSER: {user}");

    // Hermes resolve_provider doesn't auto-pick from `models.default` reliably;
    // pass --provider + -m explicitly. Fall back to deepseek-reasoner if the
    // user hasn't customised settings yet.
    let provider = settings
        .hermes_provider
        .as_deref()
        .unwrap_or("deepseek");
    let model = settings
        .hermes_model
        .as_deref()
        .unwrap_or("deepseek-reasoner");

    // Build a single bash command line so the login shell brings ~/.local/bin
    // (where uv installs hermes) onto PATH. Each argument is single-quoted
    // and `'` inside a value is escaped as `'\''`.
    fn shell_quote(s: &str) -> String {
        let mut out = String::with_capacity(s.len() + 2);
        out.push('\'');
        out.push_str(&s.replace('\'', "'\\''"));
        out.push('\'');
        out
    }
    let skill = skill_name_without_slash(&settings.hermes_skill_name);
    let bash_line = format!(
        "hermes -z {prompt} --provider {prov} -m {model} --skills {skill} --yolo",
        prompt = shell_quote(&combined_prompt),
        prov = shell_quote(provider),
        model = shell_quote(model),
        skill = shell_quote(&skill),
    );

    let mut cmd = Command::new(WSL_EXE);
    hide_console(&mut cmd);
    cmd.arg("-d")
        .arg(&settings.hermes_distro)
        .arg("--")
        .arg("bash")
        .arg("-lc")
        .arg(&bash_line)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    let child = cmd
        .spawn()
        .map_err(|e| format!("wsl spawn ({}): {e}", &settings.hermes_distro))?;

    Ok(child)
}

/// Hermes accepts skill names without leading slash on the `--skills` flag.
fn skill_name_without_slash(s: &str) -> String {
    s.trim_start_matches('/').to_string()
}

/// Streams Hermes stdout line-by-line, emitting `ceo-chunk` events to the UI.
/// Returns the accumulated full response on EOF, or an error on:
///   • cancel signal,
///   • read failure,
///   • global timeout (`hermes_timeout_sec` from settings),
///   • non-zero exit status (stderr included in error message).
pub async fn stream_hermes_response(
    mut child: Child,
    app: &AppHandle,
    cancel: Arc<AtomicBool>,
    timeout_sec: u64,
) -> Result<String, String> {
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "no stdout on Hermes child".to_string())?;

    let read_loop = async {
        let mut reader = BufReader::new(stdout).lines();
        let mut full = String::new();
        loop {
            if cancel.load(Ordering::Relaxed) {
                log::info!("ceo stream cancelled by user");
                return Err::<String, String>("cancelled by user".to_string());
            }
            match reader.next_line().await {
                Ok(Some(line)) => {
                    let trimmed = line.trim_end_matches('\r').to_string();
                    if !looks_like_diagnostic(&trimmed) {
                        let _ = app.emit("ceo-chunk", &trimmed);
                        full.push_str(&trimmed);
                        full.push('\n');
                    } else {
                        log::debug!("ceo stream diagnostic: {trimmed}");
                    }
                }
                Ok(None) => return Ok(full.trim().to_string()),
                Err(e) => return Err(format!("read line: {e}")),
            }
        }
    };

    let result = match timeout(Duration::from_secs(timeout_sec), read_loop).await {
        Ok(r) => r,
        Err(_) => {
            child.kill().await.ok();
            return Err(format!("Hermes response timeout ({timeout_sec}s)"));
        }
    };

    let status = child
        .wait()
        .await
        .map_err(|e| format!("wait child: {e}"))?;
    if !status.success() {
        let mut stderr_buf = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut stderr_buf).await;
        }
        return Err(format!(
            "Hermes exit {}: {}",
            status,
            stderr_buf.trim()
        ));
    }
    result
}

#[tauri::command]
pub async fn cancel_chat_response(
    lifecycle: State<'_, ChatLifecycle>,
) -> Result<(), String> {
    lifecycle.cancel.store(true, Ordering::Relaxed);
    if let Some(pid) = *lifecycle.current_child_pid.lock().await {
        log::info!("cancel_chat_response: taskkill PID {pid}");
        // Belt-and-braces: also kill the entire WSL spawn tree on Windows.
        // tokio's kill_on_drop will fire too, but this is synchronous.
        #[cfg(windows)]
        {
            let _ = std::process::Command::new("taskkill")
                .args(["/F", "/T", "/PID", &pid.to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn();
        }
    }
    Ok(())
}

// ─── helpers ────────────────────────────────────────────────────────────

struct WslRunOutput {
    success: bool,
    stdout: String,
    #[allow(dead_code)]
    stderr: String,
}

/// Runs a command inside the WSL distro through a **login bash shell** so
/// `~/.bashrc` / `~/.profile` PATH additions (e.g. `~/.local/bin` for uv-
/// installed binaries like Hermes) are present. Without `-lc` the spawned
/// process inherits a minimal `PATH=/usr/local/bin:/usr/bin:/bin` and we
/// would miss anything installed in user space.
async fn wsl_run(distro: &str, args: &[&str]) -> WslRunOutput {
    // Compose the args into a single shell-safe command line. Each arg gets
    // single-quoted (with embedded `'` escaped as `'\''`) so the shell sees
    // exactly what we passed.
    let mut quoted = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            quoted.push(' ');
        }
        quoted.push('\'');
        quoted.push_str(&a.replace('\'', "'\\''"));
        quoted.push('\'');
    }

    let mut cmd = Command::new(WSL_EXE);
    hide_console(&mut cmd);
    cmd.arg("-d")
        .arg(distro)
        .arg("--")
        .arg("bash")
        .arg("-lc")
        .arg(&quoted)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    match cmd.output().await {
        Ok(out) => WslRunOutput {
            success: out.status.success(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        },
        Err(e) => WslRunOutput {
            success: false,
            stdout: String::new(),
            stderr: format!("wsl spawn: {e}"),
        },
    }
}

/// `wsl.exe --list --quiet` writes UTF-16 LE on most Windows versions.
/// Decode that, otherwise fall back to lossy UTF-8.
fn decode_wsl_utf16(raw: &[u8]) -> String {
    if raw.len() >= 2 && raw.len() % 2 == 0 {
        let words: Vec<u16> = raw
            .chunks_exact(2)
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        if let Ok(s) = String::from_utf16(&words) {
            // Strip BOM if present.
            return s.trim_start_matches('\u{FEFF}').to_string();
        }
    }
    String::from_utf8_lossy(raw).into_owned()
}

/// Filter Hermes' own progress/log lines out of the user-visible stream.
/// Adjust as we learn the real format from the live binary.
fn looks_like_diagnostic(line: &str) -> bool {
    let l = line.trim_start();
    l.starts_with("[INFO")
        || l.starts_with("[DEBUG")
        || l.starts_with("[WARN")
        || l.starts_with("[ERROR")
        || l.starts_with("hermes:")
}
