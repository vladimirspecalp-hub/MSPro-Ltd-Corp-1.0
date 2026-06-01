//! ExternalGatewayDriver — STUB (Phase 1).
//!
//! Все методы возвращают NotImplemented / Unknown. Зарегистрирован в
//! provider_registry для UI demo (DEC-001 «3 провайдера»). Реальная реализация
//! (reuse WS gateway 8899 + `pod/invoke`/`pod/respond`) — Phase 2.
//! Источник истины: `phase-1-external-gateway-driver-IMPL-REFERENCE.md`.

use async_trait::async_trait;

use super::{
    Capabilities, HealthStatus, PostRuntimeProvider, ProviderError, ProviderKind, ProviderRequest,
    ProviderResponse,
};

pub struct ExternalGatewayDriver {
    id: String,
}

impl ExternalGatewayDriver {
    pub fn new(id: String) -> Self {
        Self { id }
    }
}

#[async_trait]
impl PostRuntimeProvider for ExternalGatewayDriver {
    async fn invoke(&self, _request: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        Err(ProviderError::NotImplemented(
            "ExternalGateway driver — Phase 2 R&D (reuse external_agent WS gateway 8899)".into(),
        ))
    }

    async fn health_check(&self) -> HealthStatus {
        // Phase 1: нет endpoint для ping → Unknown (Phase 2: WS ping к :8899).
        HealthStatus::Unknown
    }

    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::ExternalGateway
    }

    fn provider_id(&self) -> String {
        self.id.clone()
    }

    fn capabilities(&self) -> Capabilities {
        // Честный stub — ничего не умеет пока.
        Capabilities {
            supports_tools: false,
            supports_mcp: false,
            supports_streaming: false,
            supports_prompt_caching: false,
            max_context_tokens: 0,
            max_output_tokens: 0,
            supports_vision: false,
        }
    }

    fn cost_per_1k_tokens(&self) -> (f64, f64) {
        (0.0, 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pal::{ProviderRequest, RequestTrace, Tier};

    fn dummy_request() -> ProviderRequest {
        ProviderRequest {
            system_prompt: "s".into(),
            user_message: "u".into(),
            tier: Tier::T2,
            timeout: None,
            max_turns: None,
            model_override: None,
            workspace_path: None,
            agent_slug: None,
            mcp_bindings: Vec::new(),
            trace: RequestTrace {
                post_slug: "p".into(),
                dispatcher_log_id: None,
                attempt_id: "a".into(),
                attempt_number: 0,
            },
        }
    }

    #[tokio::test]
    async fn invoke_returns_not_implemented() {
        let d = ExternalGatewayDriver::new("external_gateway".into());
        let r = d.invoke(dummy_request()).await;
        match r {
            Err(ProviderError::NotImplemented(msg)) => assert!(msg.contains("Phase 2")),
            other => panic!("expected NotImplemented, got {other:?}"),
        }
        assert!(!ProviderError::NotImplemented("x".into()).should_fallback());
    }

    #[tokio::test]
    async fn health_is_unknown_and_caps_all_false() {
        let d = ExternalGatewayDriver::new("external_gateway".into());
        assert_eq!(d.health_check().await, HealthStatus::Unknown);
        let c = d.capabilities();
        assert!(!c.supports_tools && !c.supports_mcp && !c.supports_vision);
        assert_eq!(c.max_context_tokens, 0);
    }
}
