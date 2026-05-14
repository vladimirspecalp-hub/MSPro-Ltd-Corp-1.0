//! Шаг 10 — Двухконтурный Мозг: основной контур Claude 4.7 Opus через CLI.
//!
//! Spawns `claude --print --agent mspro-ceo --model <id>` локально на машине
//! Владельца через `tokio::process::Command`. Авторизация — через
//! предварительно настроенную сессию Claude Code (`claude /login` в терминале
//! владельца). Никакого WSL, никакой Hermes-обвязки.
//!
//! Key trick: создаём `~/.claude/agents/mspro-ceo.md` с пустым `tools: []`
//! frontmatter. Это **физически отключает** у Claude'а его native skills
//! (Bash, Read, Write, WebFetch, и т.д.), и единственным каналом действия
//! остаётся **XML-формат `<tool_call>` в текстовом ответе**, который ловит
//! наш существующий парсер (`commands/tool_calls.rs`).
//!
//! Так решается основная проблема Hermes-эры — конкуренция tool-систем
//! больше не возможна, Claude не может «сгаллюцинировать» write_file.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter, State};
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, Command};
use tokio::sync::Mutex as AsyncMutex;
use tokio::time::timeout;

use crate::settings::{AppSettings, SettingsStore};

// ---------------------------------------------------------------------------
// hide-console + ChatLifecycle (общие для Claude CLI и Qwen)
// ---------------------------------------------------------------------------

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Скрывает консольное окно у дочернего процесса на Windows.
/// На *nix — no-op.
pub fn hide_console(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// Tauri-managed state с lifecycle текущего CEO-генерации.
/// В каждый момент времени работает ОДНА генерация (UI не даёт parallel send'ов).
/// Cancel-кнопка читает `cancel`, а если есть `current_child_pid` — убивает процесс.
#[derive(Default)]
pub struct ChatLifecycle {
    pub current_child_pid: AsyncMutex<Option<u32>>,
    pub cancel: Arc<AtomicBool>,
}

// ---------------------------------------------------------------------------
// Status detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClaudeCliStatus {
    Available { version: String, path: String },
    NotFound { configured_path: String, error: String },
}

#[tauri::command]
pub async fn detect_claude_cli(
    settings: State<'_, SettingsStore>,
) -> Result<ClaudeCliStatus, String> {
    let path = settings.data.lock().unwrap().claude_cli_path.clone();
    Ok(detect_claude_cli_inner(&path).await)
}

pub async fn detect_claude_cli_inner(path: &str) -> ClaudeCliStatus {
    let mut cmd = Command::new(path);
    hide_console(&mut cmd);
    cmd.arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    match timeout(Duration::from_secs(8), cmd.output()).await {
        Ok(Ok(out)) if out.status.success() => {
            let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
            ClaudeCliStatus::Available {
                version: if v.is_empty() { "unknown".into() } else { v },
                path: path.to_string(),
            }
        }
        Ok(Ok(out)) => ClaudeCliStatus::NotFound {
            configured_path: path.to_string(),
            error: format!(
                "claude exited with {}: {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ),
        },
        Ok(Err(e)) => ClaudeCliStatus::NotFound {
            configured_path: path.to_string(),
            error: format!("spawn failed: {e}"),
        },
        Err(_) => ClaudeCliStatus::NotFound {
            configured_path: path.to_string(),
            error: "--version timed out after 8s".into(),
        },
    }
}

// ---------------------------------------------------------------------------
// Agent file (~/.claude/agents/mspro-ceo.md) — auto-create
// ---------------------------------------------------------------------------

const AGENT_NAME: &str = "mspro-ceo";

const AGENT_MD: &str = r#"---
name: mspro-ceo
description: Гендир MSPro-Ltd Corp — отвечает строго по XML-протоколу tool_call. У него нет доступа к Bash/Read/Write/WebFetch, только текст в ответе.
tools: []
model: claude-opus-4-7
---

# Гендир MSPro-Ltd Corp

Тебе на вход придёт user-сообщение, начинающееся с блока
`# SYSTEM CONTEXT (MSPro-Ltd Corp)` — это правила игры, оргструктура,
HMT-состояния, Vault-память и JSON-схемы доступных инструментов.

Ниже него — блок `# USER` с конкретным запросом Владельца (Бровякова В.А.).

## Жёсткие правила работы

1. **Действуй строго по SYSTEM CONTEXT.** Никаких отсебятин по архитектуре.
2. **Единственный канал действия — XML-блок `<tool_call>` в твоём ответе.**
   Ядро Tauri-приложения парсит его, исполняет в SQLite, отвечает
   Владельцу зелёной/красной плашкой.
3. **У тебя нет native tools.** Не пытайся использовать Bash, Read, Write,
   WebFetch, Glob, Grep — они отключены через `tools: []` в этом
   frontmatter. Любая попытка — пустая трата токенов.
4. **Если в SYSTEM CONTEXT есть все параметры для tool_call — ИСПОЛНЯЙ
   немедленно, не переспрашивай.** Владелец уже подтвердил действие постановкой
   задачи. Уточнение допустимо только когда параметра реально нет.
5. **Reasoning** оборачивай в `<think>...</think>` — эти блоки скрыты от
   Владельца.

Подробные tools-схемы и формат `<tool_call>` — в `# SYSTEM CONTEXT` блоке
каждого user-сообщения.
"#;

/// Создаёт файл `~/.claude/agents/mspro-ceo.md` если его нет.
/// Идемпотент — если файл уже существует, ничего не делает.
pub fn ensure_mspro_ceo_agent() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or_else(|| "cannot resolve home dir".to_string())?;
    let dir = home.join(".claude").join("agents");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create agents dir: {e}"))?;
    let path = dir.join(format!("{AGENT_NAME}.md"));
    if !path.exists() {
        std::fs::write(&path, AGENT_MD).map_err(|e| format!("write agent: {e}"))?;
        log::info!("Step 10: created Claude agent file at {}", path.display());
    }
    Ok(path)
}

// ---------------------------------------------------------------------------
// Main runner
// ---------------------------------------------------------------------------

/// Запускает `claude --print --agent mspro-ceo --model <id>` и пишет в stdin
/// финальный prompt. Возвращает полный текст ответа.
///
/// На вход подаётся **уже собранный** prompt (system context + user message
/// в одном блоке). Шаг 10 формирует его в `chat.rs`.
pub async fn run_claude_cli(
    full_prompt: &str,
    settings: &AppSettings,
    lifecycle: &ChatLifecycle,
    app: &AppHandle,
) -> Result<String, String> {
    // Проверяем что CLI вообще доступен — иначе сразу ошибка для auto-fallback.
    let status = detect_claude_cli_inner(&settings.claude_cli_path).await;
    if let ClaudeCliStatus::NotFound { error, .. } = &status {
        return Err(format!("claude CLI недоступен: {error}"));
    }

    // Идемпотентно создаём agent file (если ещё нет).
    if let Err(e) = ensure_mspro_ceo_agent() {
        log::warn!("could not ensure mspro-ceo agent: {e}");
        // Не критично — пойдём без --agent (Claude получит native tools, но
        // text inside SYSTEM CONTEXT блок всё равно их запрещает в правилах).
    }

    let mut cmd = Command::new(&settings.claude_cli_path);
    hide_console(&mut cmd);
    cmd.arg("--print")
        .arg("--output-format")
        .arg("text")
        .arg("--agent")
        .arg(AGENT_NAME)
        .arg("--model")
        .arg(&settings.claude_cli_model)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    log::info!(
        "Step 10: spawning claude CLI (model={}, prompt_len={})",
        settings.claude_cli_model,
        full_prompt.len()
    );

    let mut child: Child = cmd
        .spawn()
        .map_err(|e| format!("claude spawn failed: {e}"))?;

    // Сохраняем PID для cancel-команды.
    if let Some(pid) = child.id() {
        *lifecycle.current_child_pid.lock().await = Some(pid);
    }
    lifecycle.cancel.store(false, Ordering::Relaxed);

    // Записываем prompt в stdin и закрываем pipe — Claude получит EOF и
    // начнёт генерировать.
    let mut stdin_pipe = child
        .stdin
        .take()
        .ok_or_else(|| "could not open claude stdin".to_string())?;
    if let Err(e) = stdin_pipe.write_all(full_prompt.as_bytes()).await {
        return Err(format!("write to claude stdin: {e}"));
    }
    drop(stdin_pipe); // close → EOF for child

    // Читаем stdout до конца с глобальным таймаутом и проверкой cancel.
    let cancel_flag = lifecycle.cancel.clone();
    let timeout_secs = settings.claude_cli_timeout_sec;

    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or_else(|| "could not open claude stdout".to_string())?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or_else(|| "could not open claude stderr".to_string())?;

    let read_fut = async move {
        let mut out = String::with_capacity(4096);
        let mut buf = [0u8; 4096];
        loop {
            if cancel_flag.load(Ordering::Relaxed) {
                return Err::<String, String>("cancelled".to_string());
            }
            match stdout_pipe.read(&mut buf).await {
                Ok(0) => break,
                Ok(n) => {
                    let chunk = String::from_utf8_lossy(&buf[..n]);
                    out.push_str(&chunk);
                    // Эмитим chunk для typing-эффекта в UI.
                    let _ = tauri::Emitter::emit(app, "ceo-chunk", chunk.to_string());
                }
                Err(e) => return Err(format!("stdout read: {e}")),
            }
        }
        Ok(out)
    };

    let result = match timeout(Duration::from_secs(timeout_secs), read_fut).await {
        Ok(Ok(text)) => Ok(text),
        Ok(Err(e)) if e == "cancelled" => Err("cancelled".to_string()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(format!("claude CLI timeout ({timeout_secs}s)")),
    };

    // Если ошибка — захватываем stderr для диагностики.
    if result.is_err() {
        let mut stderr_buf = String::new();
        let _ = stderr_pipe.read_to_string(&mut stderr_buf).await;
        if !stderr_buf.trim().is_empty() {
            log::warn!("claude stderr: {}", stderr_buf.trim());
        }
    }

    // Корректно закрываем процесс.
    let _ = child.kill().await;
    *lifecycle.current_child_pid.lock().await = None;

    result
}

// ---------------------------------------------------------------------------
// Cancel command — общая для Claude CLI и Qwen
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn cancel_chat_response(
    lifecycle: State<'_, ChatLifecycle>,
) -> Result<(), String> {
    lifecycle.cancel.store(true, Ordering::Relaxed);

    // Если PID известен — пытаемся убить процесс напрямую через sysinfo
    // (на случай если read_fut завис на чём-то долгом и не проверяет флаг).
    if let Some(pid) = *lifecycle.current_child_pid.lock().await {
        log::info!("cancel_chat_response: killing child pid={pid}");
        use sysinfo::{Pid, System};
        let mut sys = System::new();
        sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        if let Some(proc) = sys.process(Pid::from_u32(pid)) {
            proc.kill();
        }
    }
    Ok(())
}
