//! PAL orchestrator — владелец outer timeout (trait v3 §6).
//!
//! Срез 1: single-provider invoke с outer `tokio::time::timeout`.
//! Срез 2: fallback chain (по `ProviderError::should_fallback()`).

use super::{PostRuntimeProvider, ProviderError, ProviderRequest, ProviderResponse, Tier};

/// Единственная точка outer timeout. Драйвер свой outer timeout НЕ ставит.
/// effective = request.timeout || Tier::default_timeout(), clamp ≤ hard cap (600с).
/// При истечении future драйвера дропается → kill_on_drop у subprocess.
pub async fn pal_invoke(
    provider: &dyn PostRuntimeProvider,
    request: ProviderRequest,
) -> Result<ProviderResponse, ProviderError> {
    let effective = request
        .timeout
        .unwrap_or_else(|| request.tier.default_timeout())
        .min(Tier::hard_cap_timeout());

    match tokio::time::timeout(effective, provider.invoke(request)).await {
        Ok(Ok(resp)) => Ok(resp),
        Ok(Err(provider_err)) => Err(provider_err),
        Err(_elapsed) => Err(ProviderError::Timeout {
            timeout_secs: effective.as_secs(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::pal_invoke;
    use crate::pal::*;
    use async_trait::async_trait;
    use std::time::Duration;

    struct MockDriver {
        delay: Duration,
        outcome: Result<String, ProviderError>,
    }

    #[async_trait]
    impl PostRuntimeProvider for MockDriver {
        async fn invoke(
            &self,
            _request: ProviderRequest,
        ) -> Result<ProviderResponse, ProviderError> {
            tokio::time::sleep(self.delay).await;
            match &self.outcome {
                Ok(text) => Ok(ProviderResponse {
                    text: text.clone(),
                    usage: TokenUsage::default(),
                    latency_ms: 0,
                    model_used: "mock".into(),
                    provider_used: ProviderKind::QwenHttp,
                    stop_reason: "end_turn".into(),
                    artifacts: Vec::new(),
                }),
                Err(e) => Err(e.clone()),
            }
        }
        async fn health_check(&self) -> HealthStatus {
            HealthStatus::Alive
        }
        fn provider_kind(&self) -> ProviderKind {
            ProviderKind::QwenHttp
        }
        fn provider_id(&self) -> String {
            "mock".into()
        }
        fn capabilities(&self) -> Capabilities {
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

    fn req(timeout_ms: u64) -> ProviderRequest {
        ProviderRequest {
            system_prompt: "sys".into(),
            user_message: "u".into(),
            tier: Tier::T3,
            timeout: Some(Duration::from_millis(timeout_ms)),
            max_turns: None,
            model_override: None,
            workspace_path: None,
            agent_slug: None,
            mcp_bindings: Vec::new(),
            trace: RequestTrace {
                post_slug: "p".into(),
                dispatcher_log_id: None,
                attempt_id: "a0".into(),
                attempt_number: 0,
            },
        }
    }

    #[tokio::test]
    async fn timeout_fires_when_driver_too_slow() {
        let d = MockDriver {
            delay: Duration::from_millis(500),
            outcome: Ok("never".into()),
        };
        let r = pal_invoke(&d, req(50)).await;
        assert!(matches!(r, Err(ProviderError::Timeout { .. })));
    }

    #[tokio::test]
    async fn success_passes_through() {
        let d = MockDriver {
            delay: Duration::from_millis(0),
            outcome: Ok("hi".into()),
        };
        let r = pal_invoke(&d, req(5000)).await;
        match r {
            Ok(resp) => assert_eq!(resp.text, "hi"),
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn provider_error_passes_through_not_timeout() {
        let d = MockDriver {
            delay: Duration::from_millis(0),
            outcome: Err(ProviderError::BadRequest("nope".into())),
        };
        let r = pal_invoke(&d, req(5000)).await;
        assert!(matches!(r, Err(ProviderError::BadRequest(_))));
    }
}
