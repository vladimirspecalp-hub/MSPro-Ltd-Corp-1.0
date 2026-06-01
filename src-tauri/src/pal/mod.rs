//! PAL — Provider Abstraction Layer (Phase 1 / Iteration B).
//!
//! Единый Rust-trait `PostRuntimeProvider`, через который вызывается любая
//! LLM-модель/CLI/локальный сервер. Реализации (`ClaudeCliDriver`,
//! `QwenHttpDriver`, `ExternalGatewayDriver`) знают как разговаривать с
//! конкретным провайдером; общая логика (timeout, fallback, run logging) —
//! в `orchestrator`.
//!
//! Контракт зафиксирован в `Vault/03-Phases/phase-1-pal-trait-spec.md` v3.
//! Драйверы — по `*-IMPL-REFERENCE.md`.

// PAL — foundation layer. Полный API (health_check, capabilities, provider_id,
// fallback chain, ExternalGatewayDriver) дотягивается в Срезах 2-3 (health
// monitor, provider_registry, fallback). До тех пор часть items не вызывается
// из non-test кода — ожидаемо для walking skeleton, не мёртвый код.
#![allow(dead_code)]

pub mod claude_cli_driver;
pub mod external_gateway_driver;
pub mod orchestrator;
pub mod qwen_http_driver;

use std::path::PathBuf;
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Trait
// ---------------------------------------------------------------------------

/// Единый контракт исполнителя LLM-вызова в MSPro AgentPod.
#[async_trait]
pub trait PostRuntimeProvider: Send + Sync {
    /// Основной вызов модели. Драйвер делает фактический IO.
    ///
    /// Timeout policy: драйвер НЕ применяет outer `tokio::time::timeout` —
    /// им владеет `orchestrator::pal_invoke`. При отмене future (drop) драйвер
    /// обязан корректно закрыть subprocess/connection (`kill_on_drop` и т.п.).
    async fn invoke(&self, request: ProviderRequest) -> Result<ProviderResponse, ProviderError>;

    /// Лёгкая health-проверка (≤2с, без расхода боевых токенов).
    async fn health_check(&self) -> HealthStatus;

    /// Тип провайдера — для маршрутизации / логов / UI.
    fn provider_kind(&self) -> ProviderKind;

    /// Уникальный id инстанса в `provider_registry`.
    fn provider_id(&self) -> String;

    /// Декларация возможностей (читается один раз, кешируется PAL).
    fn capabilities(&self) -> Capabilities;

    /// Стоимость в USD за 1k токенов: (input, output). Локальные модели → (0,0).
    fn cost_per_1k_tokens(&self) -> (f64, f64);
}

// ---------------------------------------------------------------------------
// ProviderRequest
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    /// Системный промпт поста (≤130 строк по правилу проекта).
    pub system_prompt: String,
    /// Тело задачи: refined prompt от Диспетчера или raw task.
    pub user_message: String,
    /// Tier — даёт default timeout/max_turns если не заданы явно.
    pub tier: Tier,
    /// Override timeout. None → `Tier::default_timeout()`. Hint для orchestrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,
    /// Override max_turns. None → `Tier::default_max_turns()`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Per-request override модели (DEC-002 hot-swap). None → default драйвера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,
    /// Sandbox workspace (Outbox/<task_id>). ClaudeCli: Some → `current_dir`.
    /// Qwen/ExternalGateway: None (нет subprocess workspace).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<PathBuf>,
    /// CLI agent profile (`--agent mspro-{slug}`). ClaudeCli: Some.
    /// Qwen/ExternalGateway: None (молча игнорируют).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_slug: Option<String>,
    /// MCP-биндинги поста. Если provider не supports_mcp — PAL отбросит с warning.
    #[serde(default)]
    pub mcp_bindings: Vec<McpBinding>,
    /// Трассировка — связь с dispatcher_logs/run_logs.
    pub trace: RequestTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpBinding {
    pub mcp_name: String,
    /// Ссылка на secret/config в OS Keychain (не значение).
    pub config_ref: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestTrace {
    /// Slug поста-инициатора (audit, не путать с agent_slug).
    pub post_slug: String,
    /// FK на dispatcher_logs.id (TEXT, формат task-<uuid>). None для прямых вызовов.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatcher_log_id: Option<String>,
    /// Уникальный id попытки (для различения в fallback chain).
    pub attempt_id: String,
    /// Номер попытки в chain (0 = primary, 1+ = fallback).
    pub attempt_number: u8,
}

// ---------------------------------------------------------------------------
// ProviderResponse
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    /// Финальный текстовый ответ модели.
    pub text: String,
    /// Нормализованный учёт токенов.
    pub usage: TokenUsage,
    /// Wall-clock длительность invoke (мс).
    pub latency_ms: u64,
    /// Фактически отработавшая модель.
    pub model_used: String,
    /// Фактический провайдер (аудит fallback chain).
    pub provider_used: ProviderKind,
    /// Причина остановки. Phase 1: свободная строка ("end_turn" и т.п.).
    pub stop_reason: String,
    /// Артефакты. Phase 1 (ClaudeCli/Qwen/ExternalGateway): всегда пусто
    /// (post_executor сканирует Outbox через diff_dir).
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    #[serde(default)]
    pub cache_read_tokens: u32,
    #[serde(default)]
    pub cache_write_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub rel_path: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// ProviderError
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, thiserror::Error, Serialize, Deserialize)]
pub enum ProviderError {
    #[error("timeout after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },
    #[error("auth failed: {0}")]
    Auth(String),
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),
    #[error("server error: {0}")]
    Server(String),
    #[error("network error: {0}")]
    Network(String),
    #[error("bad request: {0}")]
    BadRequest(String),
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),
    #[error("mcp failure: {0}")]
    McpFailure(String),
    #[error("tool loop limit hit")]
    ToolLoopLimit,
    #[error("not implemented: {0}")]
    NotImplemented(String),
    #[error("other: {0}")]
    Other(String),
}

impl ProviderError {
    /// Стоит ли PAL пытаться fallback на следующего провайдера в chain.
    pub fn should_fallback(&self) -> bool {
        match self {
            ProviderError::QuotaExceeded(_)
            | ProviderError::Server(_)
            | ProviderError::Network(_)
            | ProviderError::Timeout { .. }
            | ProviderError::Auth(_)
            | ProviderError::ModelUnavailable(_) => true,

            ProviderError::BadRequest(_)
            | ProviderError::McpFailure(_)
            | ProviderError::ToolLoopLimit
            | ProviderError::NotImplemented(_)
            | ProviderError::Other(_) => false,
        }
    }

    /// Короткий машинный код для `run_logs.error_kind`.
    pub fn kind_str(&self) -> &'static str {
        match self {
            ProviderError::Timeout { .. } => "timeout",
            ProviderError::Auth(_) => "auth",
            ProviderError::QuotaExceeded(_) => "quota_exceeded",
            ProviderError::Server(_) => "server",
            ProviderError::Network(_) => "network",
            ProviderError::BadRequest(_) => "bad_request",
            ProviderError::ModelUnavailable(_) => "model_unavailable",
            ProviderError::McpFailure(_) => "mcp_failure",
            ProviderError::ToolLoopLimit => "tool_loop_limit",
            ProviderError::NotImplemented(_) => "not_implemented",
            ProviderError::Other(_) => "other",
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderKind / Tier / HealthStatus / Capabilities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    ClaudeCli,
    QwenHttp,
    ExternalGateway,
}

impl ProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ProviderKind::ClaudeCli => "claude_cli",
            ProviderKind::QwenHttp => "qwen_http",
            ProviderKind::ExternalGateway => "external_gateway",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Opus-class — сложные посты, юрист, главы отделов.
    T1,
    /// Sonnet-class — frontend, аналитик, дизайнер.
    T2,
    /// Qwen local — рутина, копирайтер, простой ОТК.
    T3,
}

impl Tier {
    /// Default timeout per tier. Source of truth для PAL.
    /// T1=600 соответствует `settings.post_executor_timeout_sec` (НЕ claude_cli=360).
    pub fn default_timeout(self) -> Duration {
        match self {
            Tier::T1 => Duration::from_secs(600),
            Tier::T2 => Duration::from_secs(360),
            Tier::T3 => Duration::from_secs(60),
        }
    }

    pub fn default_max_turns(self) -> u32 {
        match self {
            Tier::T1 => 80,
            Tier::T2 => 40,
            Tier::T3 => 20,
        }
    }

    /// Hard cap для всех tiers — защита от runaway.
    pub const fn hard_cap_timeout() -> Duration {
        Duration::from_secs(600)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Tier::T1 => "T1",
            Tier::T2 => "T2",
            Tier::T3 => "T3",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    Alive,
    QuotaExceeded,
    AuthFailed,
    Unreachable,
    ServerError,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    pub supports_tools: bool,
    pub supports_mcp: bool,
    pub supports_streaming: bool,
    pub supports_prompt_caching: bool,
    pub max_context_tokens: u32,
    pub max_output_tokens: u32,
    pub supports_vision: bool,
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_fallback_matrix() {
        // Транзиентные / инфраструктурные → fallback имеет смысл.
        assert!(ProviderError::Timeout { timeout_secs: 60 }.should_fallback());
        assert!(ProviderError::QuotaExceeded("x".into()).should_fallback());
        assert!(ProviderError::Server("x".into()).should_fallback());
        assert!(ProviderError::Network("x".into()).should_fallback());
        assert!(ProviderError::Auth("x".into()).should_fallback());
        assert!(ProviderError::ModelUnavailable("x".into()).should_fallback());
        // Логические → fallback не поможет.
        assert!(!ProviderError::BadRequest("x".into()).should_fallback());
        assert!(!ProviderError::McpFailure("x".into()).should_fallback());
        assert!(!ProviderError::ToolLoopLimit.should_fallback());
        assert!(!ProviderError::NotImplemented("x".into()).should_fallback());
        assert!(!ProviderError::Other("x".into()).should_fallback());
    }

    #[test]
    fn tier_defaults() {
        assert_eq!(Tier::T1.default_timeout().as_secs(), 600);
        assert_eq!(Tier::T2.default_timeout().as_secs(), 360);
        assert_eq!(Tier::T3.default_timeout().as_secs(), 60);
        assert_eq!(Tier::hard_cap_timeout().as_secs(), 600);
        assert_eq!(Tier::T1.default_max_turns(), 80);
    }

    #[test]
    fn kind_and_error_strings() {
        assert_eq!(ProviderKind::ClaudeCli.as_str(), "claude_cli");
        assert_eq!(ProviderKind::QwenHttp.as_str(), "qwen_http");
        assert_eq!(ProviderKind::ExternalGateway.as_str(), "external_gateway");
        assert_eq!(
            ProviderError::Timeout { timeout_secs: 1 }.kind_str(),
            "timeout"
        );
        assert_eq!(
            ProviderError::NotImplemented("x".into()).kind_str(),
            "not_implemented"
        );
    }
}
