//! ClaudeCliDriver — обёртка над боевым `claude.exe --print --agent` flow.
//!
//! Phase 1 / Срез 1: повторяет ТЕКУЩИЙ argv 1-в-1 (включая
//! `--dangerously-skip-permissions`). Замена опасного флага (allowlist /
//! --permission-mode) — Срез 1.5 по результату flag-спайка.
//!
//! Источник истины: `Vault/03-Phases/phase-1-claude-cli-driver-IMPL-REFERENCE.md`.
//! Инвариант: ноль `unwrap()/expect()`.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;
use tokio::sync::Mutex as AsyncMutex;

use super::{
    Capabilities, HealthStatus, PostRuntimeProvider, ProviderError, ProviderKind,
    ProviderRequest, ProviderResponse, TokenUsage,
};

/// PAL-нейтральная регистрация PID: (running-map, task_id). Драйвер вписывает
/// PID спавненного child сразу после spawn — чтобы `cancel_post_executor` мог
/// убить процесс по task_id (I1). Тип = просто `HashMap<String,u32>` — никакой
/// зависимости от post_executor (слоистость PAL сохранена); post_executor
/// клонирует тот же Arc из `PostExecutorRegistry.running`.
pub type PidRegistration = (Arc<AsyncMutex<HashMap<String, u32>>>, String);

/// Драйвер Claude через локальный CLI (`claude.exe`).
#[derive(Clone)]
pub struct ClaudeCliDriver {
    /// id в provider_registry (`claude_cli`).
    id: String,
    /// Путь к claude.exe (из settings.claude_cli_path).
    claude_cli_path: String,
    /// Дефолтная модель (алиас `opus` / `sonnet` или полное имя).
    default_model: String,
    /// I1: куда вписать PID спавненного child (для cancel). None → не регистрируем
    /// (health_check / unit-тесты).
    pid_reg: Option<PidRegistration>,
}

impl ClaudeCliDriver {
    pub fn new(id: String, claude_cli_path: String, default_model: String) -> Self {
        Self { id, claude_cli_path, default_model, pid_reg: None }
    }

    /// I1: включить регистрацию PID в общий running-map по task_id.
    pub fn with_pid_registration(
        mut self,
        running: Arc<AsyncMutex<HashMap<String, u32>>>,
        task_id: String,
    ) -> Self {
        self.pid_reg = Some((running, task_id));
        self
    }
}

/// Pure-функция сборки argv — вынесена для unit-теста.
/// ВАЖНО: `--dangerously-skip-permissions` load-bearing в Срезе 1
/// (без него `--print` молча отказывает Write/Edit/Bash).
pub fn build_cli_args(agent_name: &str, model: &str) -> Vec<String> {
    vec![
        "--print".into(),
        "--output-format".into(),
        "text".into(),
        "--agent".into(),
        agent_name.to_string(),
        "--model".into(),
        model.to_string(),
        "--dangerously-skip-permissions".into(),
    ]
}

/// Маппинг exit/stderr → ProviderError (trait v3 §3.3 / IMPL-REFERENCE §3.3).
pub fn map_stderr_to_error(code: Option<i32>, stderr: &str) -> ProviderError {
    let low = stderr.to_lowercase();
    if low.contains("rate limit") || low.contains("quota") {
        return ProviderError::QuotaExceeded(first_line(stderr));
    }
    if low.contains("not authenticated") || low.contains("not logged in") || low.contains("login") {
        return ProviderError::Auth(first_line(stderr));
    }
    if low.contains("model not found") || low.contains("unknown model") || low.contains("unsupported model") {
        return ProviderError::ModelUnavailable(first_line(stderr));
    }
    if low.contains("connection") || low.contains("network") || low.contains("dns") {
        return ProviderError::Network(first_line(stderr));
    }
    if low.contains("timed out") || code == Some(124) {
        return ProviderError::Timeout { timeout_secs: 0 };
    }
    ProviderError::Server(format!("exit={:?}; {}", code, tail(stderr, 8)))
}

fn first_line(s: &str) -> String {
    s.lines().next().unwrap_or("").trim().to_string()
}

fn tail(s: &str, n: usize) -> String {
    let lines: Vec<&str> = s.trim().lines().collect();
    let start = lines.len().saturating_sub(n);
    lines[start..].join("\n")
}

#[async_trait]
impl PostRuntimeProvider for ClaudeCliDriver {
    async fn invoke(&self, request: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let started = Instant::now();

        let model = request
            .model_override
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| self.default_model.clone());

        let workspace = request
            .workspace_path
            .clone()
            .ok_or_else(|| ProviderError::BadRequest("ClaudeCli requires workspace_path".into()))?;

        let agent = request
            .agent_slug
            .clone()
            .filter(|a| !a.trim().is_empty())
            .ok_or_else(|| ProviderError::BadRequest("ClaudeCli requires agent_slug".into()))?;

        if !request.mcp_bindings.is_empty() {
            log::warn!(
                "ClaudeCliDriver Phase 1: dropping {} mcp_bindings (per-post MCP не поддерживается)",
                request.mcp_bindings.len()
            );
        }

        let args = build_cli_args(&agent, &model);
        let mut cmd = Command::new(&self.claude_cli_path);
        crate::commands::claude_bridge::hide_console(&mut cmd);
        cmd.args(&args)
            .current_dir(&workspace)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(task_id) = &request.trace.dispatcher_log_id {
            cmd.env("MSPRO_TASK_ID", task_id);
        }

        log::info!(
            "ClaudeCliDriver: spawn agent={agent} model={model} cwd={}",
            workspace.display()
        );

        let mut child = cmd
            .spawn()
            .map_err(|e| ProviderError::Server(format!("claude spawn failed: {e}")))?;

        // I1: регистрируем PID в общий running-map → cancel_post_executor может
        // убить процесс по task_id. kill_on_drop остаётся как backstop.
        if let (Some((running, task_id)), Some(pid)) = (&self.pid_reg, child.id()) {
            running.lock().await.insert(task_id.clone(), pid);
            log::info!("ClaudeCliDriver: registered pid={pid} for task={task_id}");
        }

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(request.user_message.as_bytes())
                .await
                .map_err(|e| ProviderError::Server(format!("stdin write: {e}")))?;
            let _ = stdin.shutdown().await;
        }

        // Orchestrator владеет outer timeout; при его срабатывании future
        // дропается → kill_on_drop убивает child. Здесь ждём штатно.
        let output = child
            .wait_with_output()
            .await
            .map_err(|e| ProviderError::Server(format!("wait child: {e}")))?;

        let latency_ms = started.elapsed().as_millis() as u64;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            return Err(map_stderr_to_error(output.status.code(), &stderr));
        }

        // Phase 1: usage недоступен в --output-format text → (0,0);
        // artifacts обнаруживает post_executor через diff_dir → пусто.
        Ok(ProviderResponse {
            text: stdout,
            usage: TokenUsage::default(),
            latency_ms,
            model_used: model,
            provider_used: ProviderKind::ClaudeCli,
            stop_reason: "end_turn".to_string(),
            artifacts: Vec::new(),
        })
    }

    async fn health_check(&self) -> HealthStatus {
        let mut cmd = Command::new(&self.claude_cli_path);
        crate::commands::claude_bridge::hide_console(&mut cmd);
        cmd.arg("--version")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        match tokio::time::timeout(Duration::from_secs(2), cmd.output()).await {
            Ok(Ok(out)) if out.status.success() => HealthStatus::Alive,
            Ok(Ok(_)) => HealthStatus::Unreachable,
            Ok(Err(_)) => HealthStatus::Unreachable,
            Err(_) => HealthStatus::Unreachable, // >2с — считаем недоступным
        }
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::ClaudeCli
    }

    fn provider_id(&self) -> String {
        self.id.clone()
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            supports_tools: true,
            supports_mcp: false, // Phase 1: per-post MCP не поддержан (R-T-001)
            supports_streaming: false,
            supports_prompt_caching: false,
            max_context_tokens: 200_000,
            max_output_tokens: 64_000,
            supports_vision: true,
        }
    }

    fn cost_per_1k_tokens(&self) -> (f64, f64) {
        let m = self.default_model.to_lowercase();
        if m.contains("opus") {
            (15.0, 75.0)
        } else if m.contains("sonnet") {
            (3.0, 15.0)
        } else if m.contains("haiku") {
            (0.8, 4.0)
        } else {
            (0.0, 0.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_args_have_all_flags_in_order() {
        let a = build_cli_args("mspro-office-manager", "opus");
        assert_eq!(
            a,
            vec![
                "--print",
                "--output-format",
                "text",
                "--agent",
                "mspro-office-manager",
                "--model",
                "opus",
                "--dangerously-skip-permissions",
            ]
        );
    }

    #[test]
    fn stderr_maps_to_correct_error() {
        assert!(matches!(
            map_stderr_to_error(Some(1), "Error: rate limit exceeded"),
            ProviderError::QuotaExceeded(_)
        ));
        assert!(matches!(
            map_stderr_to_error(Some(1), "you are not logged in"),
            ProviderError::Auth(_)
        ));
        assert!(matches!(
            map_stderr_to_error(Some(1), "model not found: bogus"),
            ProviderError::ModelUnavailable(_)
        ));
        assert!(matches!(
            map_stderr_to_error(Some(1), "connection refused"),
            ProviderError::Network(_)
        ));
        assert!(matches!(
            map_stderr_to_error(Some(124), "request timed out"),
            ProviderError::Timeout { .. }
        ));
        assert!(matches!(
            map_stderr_to_error(Some(2), "some other failure"),
            ProviderError::Server(_)
        ));
    }
}
