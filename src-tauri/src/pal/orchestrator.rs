//! PAL orchestrator — владелец outer timeout (trait v3 §6).
//!
//! Срез 1: single-provider invoke с outer `tokio::time::timeout`.
//! Срез 2: fallback chain (по `ProviderError::should_fallback()`).

use std::sync::Arc;

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

/// Результат прохода по fallback chain.
pub struct ChainOutcome {
    pub result: Result<ProviderResponse, ProviderError>,
    /// Индекс провайдера, давшего итог (0 = primary). При полном провале — последний.
    pub attempt_idx: usize,
    /// true если итог получен НЕ первым провайдером (был fallback).
    pub fallback_used: bool,
}

/// Прогон запроса по цепочке провайдеров с fallback.
///
/// На каждом провайдере — `pal_invoke` (свой outer timeout; НЕ оборачиваем chain
/// ещё одним — BL-P1-009). Success → возврат. Err && `should_fallback()` && есть
/// следующий → переходим к нему (attempt_number++). Err && !should_fallback →
/// немедленный возврат (логическая ошибка, fallback не поможет). Chain исчерпан →
/// агрегированная `Other` со всеми ошибками (R-T-009).
///
/// `request.timeout`/`tier` применяются к КАЖДОЙ попытке (outer timeout per attempt).
pub async fn pal_invoke_chain(
    chain: &[Arc<dyn PostRuntimeProvider>],
    request: ProviderRequest,
) -> ChainOutcome {
    if chain.is_empty() {
        return ChainOutcome {
            result: Err(ProviderError::Other("empty provider chain".into())),
            attempt_idx: 0,
            fallback_used: false,
        };
    }

    let mut errors: Vec<String> = Vec::new();
    let last = chain.len() - 1;

    for (idx, provider) in chain.iter().enumerate() {
        // Каждой попытке — свежий request с обновлённым attempt_number/attempt_id.
        let mut req = request.clone();
        req.trace.attempt_number = idx as u8;
        req.trace.attempt_id = format!("{}-{}", request.trace.attempt_id, idx);

        match pal_invoke(provider.as_ref(), req).await {
            Ok(resp) => {
                return ChainOutcome {
                    result: Ok(resp),
                    attempt_idx: idx,
                    fallback_used: idx > 0,
                };
            }
            Err(e) => {
                let fallbackable = e.should_fallback();
                errors.push(format!("{}: {}", provider.provider_id(), e));
                log::warn!(
                    "pal_invoke_chain: provider[{idx}] {} failed: {e} (should_fallback={fallbackable})",
                    provider.provider_id()
                );
                // Не fallbackable ИЛИ это последний — возвращаем.
                if !fallbackable {
                    return ChainOutcome {
                        result: Err(e),
                        attempt_idx: idx,
                        fallback_used: idx > 0,
                    };
                }
                if idx == last {
                    return ChainOutcome {
                        result: Err(ProviderError::Other(format!(
                            "all {} providers failed: [{}]",
                            chain.len(),
                            errors.join("; ")
                        ))),
                        attempt_idx: idx,
                        fallback_used: idx > 0,
                    };
                }
                // иначе — следующий провайдер
            }
        }
    }
    // недостижимо (last всегда возвращает), но для полноты:
    ChainOutcome {
        result: Err(ProviderError::Other(format!(
            "chain exhausted: [{}]",
            errors.join("; ")
        ))),
        attempt_idx: last,
        fallback_used: chain.len() > 1,
    }
}

#[cfg(test)]
mod tests {
    use super::{pal_invoke, pal_invoke_chain};
    use crate::pal::*;
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::time::Duration;

    struct MockDriver {
        id: String,
        delay: Duration,
        outcome: Result<String, ProviderError>,
    }

    impl MockDriver {
        fn ok(id: &str, text: &str) -> Self {
            Self {
                id: id.into(),
                delay: Duration::from_millis(0),
                outcome: Ok(text.into()),
            }
        }
        fn err(id: &str, e: ProviderError) -> Self {
            Self {
                id: id.into(),
                delay: Duration::from_millis(0),
                outcome: Err(e),
            }
        }
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
                    model_used: self.id.clone(),
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
            self.id.clone()
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
            id: "slow".into(),
            delay: Duration::from_millis(500),
            outcome: Ok("never".into()),
        };
        let r = pal_invoke(&d, req(50)).await;
        assert!(matches!(r, Err(ProviderError::Timeout { .. })));
    }

    #[tokio::test]
    async fn success_passes_through() {
        let d = MockDriver::ok("m", "hi");
        let r = pal_invoke(&d, req(5000)).await;
        match r {
            Ok(resp) => assert_eq!(resp.text, "hi"),
            Err(e) => panic!("expected Ok, got {e:?}"),
        }
    }

    #[tokio::test]
    async fn provider_error_passes_through_not_timeout() {
        let d = MockDriver::err("m", ProviderError::BadRequest("nope".into()));
        let r = pal_invoke(&d, req(5000)).await;
        assert!(matches!(r, Err(ProviderError::BadRequest(_))));
    }

    // ----- fallback chain (Срез 2) -----

    #[tokio::test]
    async fn chain_primary_fails_fallback_succeeds() {
        // primary Server (fallbackable) → fallback Alive.
        let chain: Vec<Arc<dyn PostRuntimeProvider>> = vec![
            Arc::new(MockDriver::err(
                "claude_cli",
                ProviderError::Server("down".into()),
            )),
            Arc::new(MockDriver::ok("qwen_http", "ответ-fallback")),
        ];
        let out = pal_invoke_chain(&chain, req(5000)).await;
        match out.result {
            Ok(resp) => assert_eq!(resp.model_used, "qwen_http"),
            Err(e) => panic!("expected fallback success, got {e:?}"),
        }
        assert_eq!(out.attempt_idx, 1);
        assert!(out.fallback_used);
    }

    #[tokio::test]
    async fn chain_bad_request_does_not_fallback() {
        // primary BadRequest (НЕ fallbackable) → стоп на idx 0, fallback не зовётся.
        let chain: Vec<Arc<dyn PostRuntimeProvider>> = vec![
            Arc::new(MockDriver::err(
                "claude_cli",
                ProviderError::BadRequest("invalid".into()),
            )),
            Arc::new(MockDriver::ok("qwen_http", "не должен сработать")),
        ];
        let out = pal_invoke_chain(&chain, req(5000)).await;
        assert!(matches!(out.result, Err(ProviderError::BadRequest(_))));
        assert_eq!(out.attempt_idx, 0);
        assert!(!out.fallback_used);
    }

    #[tokio::test]
    async fn chain_all_fail_aggregated() {
        // оба fallbackable, оба упали → агрегированная Other со всеми ошибками.
        let chain: Vec<Arc<dyn PostRuntimeProvider>> = vec![
            Arc::new(MockDriver::err(
                "claude_cli",
                ProviderError::Server("e1".into()),
            )),
            Arc::new(MockDriver::err(
                "qwen_http",
                ProviderError::Network("e2".into()),
            )),
        ];
        let out = pal_invoke_chain(&chain, req(5000)).await;
        match out.result {
            Err(ProviderError::Other(msg)) => {
                assert!(msg.contains("all 2 providers failed"));
                assert!(msg.contains("claude_cli") && msg.contains("qwen_http"));
            }
            other => panic!("expected aggregated Other, got {other:?}"),
        }
        assert_eq!(out.attempt_idx, 1);
    }

    #[tokio::test]
    async fn chain_single_provider_equals_pal_invoke() {
        // chain из одного = поведение pal_invoke (без fallback).
        let chain: Vec<Arc<dyn PostRuntimeProvider>> = vec![Arc::new(MockDriver::ok("solo", "ok"))];
        let out = pal_invoke_chain(&chain, req(5000)).await;
        assert!(out.result.is_ok());
        assert_eq!(out.attempt_idx, 0);
        assert!(!out.fallback_used);
    }
}
