//! QwenHttpDriver — Ollama / LM Studio (OpenAI-compatible) через HTTP.
//!
//! Phase 1 / Срез 2. Источник истины:
//! `Vault/03-Phases/phase-1-qwen-http-driver-IMPL-REFERENCE.md` v1.1.
//! Повторяет боевой flow `qwen_bridge.rs` (POST /chat/completions, SSE
//! accumulate), но как PAL-драйвер: без AtomicBool cancel (outer timeout у
//! orchestrator), supports_mcp=false. Инвариант: ноль `unwrap()/expect()`.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;

use super::{
    Capabilities, HealthStatus, PostRuntimeProvider, ProviderError, ProviderKind, ProviderRequest,
    ProviderResponse, TokenUsage,
};

/// Драйвер Qwen через локальный OpenAI-compatible endpoint (Ollama / LM Studio).
#[derive(Clone)]
pub struct QwenHttpDriver {
    /// id в provider_registry (`qwen_http`).
    id: String,
    /// Базовый endpoint, напр. `http://localhost:11434/v1`.
    endpoint: String,
    /// Дефолтная модель (`qwen3:14b`).
    default_model: String,
}

impl QwenHttpDriver {
    pub fn new(id: String, endpoint: String, default_model: String) -> Self {
        Self {
            id,
            endpoint,
            default_model,
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.endpoint.trim_end_matches('/'))
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.endpoint.trim_end_matches('/'))
    }
}

// ---------------------------------------------------------------------------
// SSE chunk structures (OpenAI-compatible streaming)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
    #[serde(default)]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct StreamUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
    /// При `stream_options.include_usage=true` приходит в финальном событии.
    #[serde(default)]
    usage: Option<StreamUsage>,
    /// Ollama при неизвестной модели шлёт `{"error": "..."}` внутри SSE.
    #[serde(default)]
    error: Option<String>,
}

// ---------------------------------------------------------------------------
// Pure helpers (unit-tested)
// ---------------------------------------------------------------------------

/// Строит request body для /chat/completions. Pure — для unit-теста.
pub fn build_request_body(
    system_prompt: &str,
    user_message: &str,
    model: &str,
) -> serde_json::Value {
    json!({
        "model": model,
        "messages": [
            {"role": "system", "content": system_prompt},
            {"role": "user", "content": user_message},
        ],
        "stream": true,
        "stream_options": {"include_usage": true},
        "temperature": 0.3,
        "max_tokens": 4096,
    })
}

/// Результат парсинга накопленного SSE-потока.
#[derive(Debug, Default, PartialEq)]
pub struct ParsedStream {
    pub text: String,
    pub prompt_tokens: u32,
    pub completion_tokens: u32,
    pub stop_reason: String,
    /// Ошибка из `error`-чанка (model not found и т.п.).
    pub error: Option<String>,
}

/// Парсит ПОЛНЫЙ накопленный SSE-буфер (все события через `\n\n`).
/// Pure — для unit-теста на mock-данных. accumulate уже склеил байты.
pub fn parse_sse_buffer(buffer: &str) -> ParsedStream {
    let mut out = ParsedStream {
        stop_reason: "end_turn".to_string(),
        ..Default::default()
    };
    for event in buffer.split("\n\n") {
        for line in event.lines() {
            let payload = match line.strip_prefix("data:") {
                Some(p) => p.trim(),
                None => continue,
            };
            if payload == "[DONE]" || payload.is_empty() {
                continue;
            }
            match serde_json::from_str::<StreamChunk>(payload) {
                Ok(chunk) => {
                    if let Some(err) = chunk.error {
                        out.error = Some(err);
                        return out;
                    }
                    if let Some(u) = chunk.usage {
                        out.prompt_tokens = u.prompt_tokens;
                        out.completion_tokens = u.completion_tokens;
                    }
                    if let Some(choice) = chunk.choices.into_iter().next() {
                        if let Some(c) = choice.delta.content {
                            out.text.push_str(&c);
                        }
                        if let Some(fr) = choice.finish_reason {
                            out.stop_reason = fr;
                        }
                    }
                }
                Err(_) => continue, // частичный/битый chunk — пропускаем (accumulate доберёт)
            }
        }
    }
    out
}

/// HTTP status + тело → ProviderError (trait v3 §3.3). Pure — unit-тест.
pub fn map_http_error(status: u16, body: &str) -> ProviderError {
    let first = body.lines().next().unwrap_or("").trim().to_string();
    match status {
        401 | 403 => ProviderError::Auth(format!("Qwen HTTP {status}: {first}")),
        404 => ProviderError::ModelUnavailable(format!("Qwen HTTP 404: {first}")),
        400 | 422 => ProviderError::BadRequest(format!("Qwen HTTP {status}: {first}")),
        429 => ProviderError::QuotaExceeded(format!("Qwen HTTP 429: {first}")),
        s if s >= 500 => ProviderError::Server(format!("Qwen HTTP {s}: {first}")),
        s => ProviderError::Server(format!("Qwen HTTP {s}: {first}")),
    }
}

#[async_trait]
impl PostRuntimeProvider for QwenHttpDriver {
    async fn invoke(&self, request: ProviderRequest) -> Result<ProviderResponse, ProviderError> {
        let started = Instant::now();

        let model = request
            .model_override
            .clone()
            .filter(|m| !m.trim().is_empty())
            .unwrap_or_else(|| self.default_model.clone());

        if !request.mcp_bindings.is_empty() {
            log::warn!(
                "QwenHttpDriver Phase 1: dropping {} mcp_bindings (MCP не поддерживается)",
                request.mcp_bindings.len()
            );
        }

        let body = build_request_body(&request.system_prompt, &request.user_message, &model);

        // Internal safety cap = hard cap (600с); реальный outer timeout — orchestrator.
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(600))
            .build()
            .map_err(|e| ProviderError::Network(format!("qwen client build: {e}")))?;

        let resp = client
            .post(self.chat_url())
            .header("Authorization", "Bearer ollama") // dummy для OAI-compat
            .json(&body)
            .send()
            .await
            .map_err(|e| ProviderError::Network(format!("qwen request: {e}")))?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(map_http_error(status.as_u16(), &text));
        }

        // SSE accumulate: байты → буфер → парс полных событий. accumulate склеивает
        // чанки в цельный text ДО возврата (redact в run_logger увидит целое).
        let mut byte_stream = resp.bytes_stream();
        let mut buffer = String::with_capacity(4096);
        while let Some(chunk_result) = byte_stream.next().await {
            let chunk =
                chunk_result.map_err(|e| ProviderError::Network(format!("qwen stream: {e}")))?;
            buffer.push_str(&String::from_utf8_lossy(&chunk));
        }

        let parsed = parse_sse_buffer(&buffer);
        if let Some(err) = parsed.error {
            return Err(ProviderError::ModelUnavailable(format!(
                "Ollama error: {err}"
            )));
        }

        let latency_ms = started.elapsed().as_millis() as u64;
        Ok(ProviderResponse {
            text: parsed.text,
            usage: TokenUsage {
                input_tokens: parsed.prompt_tokens,
                output_tokens: parsed.completion_tokens,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
            },
            latency_ms,
            model_used: model,
            provider_used: ProviderKind::QwenHttp,
            stop_reason: parsed.stop_reason,
            artifacts: Vec::new(),
        })
    }

    async fn health_check(&self) -> HealthStatus {
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(2))
            .build()
        {
            Ok(c) => c,
            Err(_) => return HealthStatus::Unreachable,
        };
        match client.get(self.models_url()).send().await {
            Ok(r) if r.status().is_success() => HealthStatus::Alive,
            Ok(r) if r.status().as_u16() >= 500 => HealthStatus::ServerError,
            Ok(_) => HealthStatus::Unreachable,
            Err(_) => HealthStatus::Unreachable,
        }
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
            supports_mcp: false, // Phase 1: per-post MCP не поддержан (R-T-001)
            supports_streaming: false, // PAL отдаёт цельный ответ; inner-streaming не наружу
            supports_prompt_caching: false,
            max_context_tokens: 32_000,
            max_output_tokens: 4_096,
            supports_vision: false,
        }
    }

    fn cost_per_1k_tokens(&self) -> (f64, f64) {
        (0.0, 0.0) // локальная модель — бесплатно
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_body_has_required_fields() {
        let b = build_request_body("sys", "hello", "qwen3:14b");
        assert_eq!(b["model"], "qwen3:14b");
        assert_eq!(b["stream"], true);
        assert_eq!(b["stream_options"]["include_usage"], true);
        assert_eq!(b["messages"][0]["role"], "system");
        assert_eq!(b["messages"][0]["content"], "sys");
        assert_eq!(b["messages"][1]["role"], "user");
        assert_eq!(b["messages"][1]["content"], "hello");
    }

    #[test]
    fn parse_accumulates_content_and_usage() {
        let buf = "data: {\"choices\":[{\"delta\":{\"content\":\"Прив\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{\"content\":\"ет\"}}]}\n\n\
                   data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n\
                   data: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":5}}\n\n\
                   data: [DONE]\n\n";
        let p = parse_sse_buffer(buf);
        assert_eq!(p.text, "Привет");
        assert_eq!(p.prompt_tokens, 12);
        assert_eq!(p.completion_tokens, 5);
        assert_eq!(p.stop_reason, "stop");
        assert!(p.error.is_none());
    }

    #[test]
    fn parse_handles_empty_and_malformed_chunks() {
        let buf = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n\
                   data: {битый json}\n\n\
                   data: \n\n\
                   data: [DONE]\n\n";
        let p = parse_sse_buffer(buf);
        assert_eq!(p.text, "ok"); // битый/пустой пропущены, не паника
    }

    #[test]
    fn parse_detects_ollama_error_chunk() {
        let buf = "data: {\"error\":\"model 'bogus' not found, try pulling it first\"}\n\n";
        let p = parse_sse_buffer(buf);
        assert!(p.error.is_some());
        assert!(p.error.unwrap().contains("not found"));
    }

    #[test]
    fn map_http_error_matrix() {
        assert!(matches!(
            map_http_error(401, "no auth"),
            ProviderError::Auth(_)
        ));
        assert!(matches!(map_http_error(403, "x"), ProviderError::Auth(_)));
        assert!(matches!(
            map_http_error(404, "model"),
            ProviderError::ModelUnavailable(_)
        ));
        assert!(matches!(
            map_http_error(400, "bad"),
            ProviderError::BadRequest(_)
        ));
        assert!(matches!(
            map_http_error(422, "bad"),
            ProviderError::BadRequest(_)
        ));
        assert!(matches!(
            map_http_error(429, "rate"),
            ProviderError::QuotaExceeded(_)
        ));
        assert!(matches!(
            map_http_error(500, "boom"),
            ProviderError::Server(_)
        ));
        assert!(matches!(
            map_http_error(503, "down"),
            ProviderError::Server(_)
        ));
    }

    #[test]
    fn caps_no_mcp_no_tools_zero_cost() {
        let d = QwenHttpDriver::new(
            "qwen_http".into(),
            "http://localhost:11434/v1".into(),
            "qwen3:14b".into(),
        );
        let c = d.capabilities();
        assert!(!c.supports_mcp && !c.supports_tools);
        assert_eq!(d.cost_per_1k_tokens(), (0.0, 0.0));
        assert_eq!(d.provider_kind(), ProviderKind::QwenHttp);
        assert_eq!(d.chat_url(), "http://localhost:11434/v1/chat/completions");
        assert_eq!(d.models_url(), "http://localhost:11434/v1/models");
    }
}
