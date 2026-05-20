//! Шаг 10 — Двухконтурный Мозг: резервный контур Qwen 3 локально.
//!
//! Qwen 3 запущен у Владельца через Ollama (`http://localhost:11434/v1`)
//! или LM Studio (`http://localhost:1234/v1`) — оба OpenAI-compatible.
//! Если интернет недоступен или Claude CLI отказал — `chat.rs` дёргает
//! `run_qwen` для офлайн-fallback.
//!
//! Реализация: `reqwest` POST на `/chat/completions` с `stream: true`,
//! SSE-парсинг `data: {json}\n\n`, accumulator финального text + emit
//! `ceo-chunk` для UI typing-эффекта. TOOLS_PREAMBLE с XML-форматом
//! идёт в system message — Qwen 3 натренирован понимать Hermes-style
//! `<tool_call>` (этот формат как раз и создан Nous Research на основе
//! Qwen-семейства).

use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

use crate::commands::claude_bridge::ChatLifecycle;
use crate::settings::{AppSettings, SettingsStore};

// ---------------------------------------------------------------------------
// Status detection
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum QwenStatus {
    Available { endpoint: String, model_count: usize },
    Unreachable { endpoint: String, error: String },
}

#[tauri::command]
pub async fn detect_qwen(
    settings: State<'_, SettingsStore>,
) -> Result<QwenStatus, String> {
    let endpoint = settings.data.lock().unwrap().qwen_endpoint.clone();
    Ok(detect_qwen_inner(&endpoint).await)
}

pub async fn detect_qwen_inner(endpoint: &str) -> QwenStatus {
    let url = format!("{}/models", endpoint.trim_end_matches('/'));
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(4))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return QwenStatus::Unreachable {
                endpoint: endpoint.to_string(),
                error: format!("client build: {e}"),
            }
        }
    };
    match client.get(&url).send().await {
        Ok(resp) if resp.status().is_success() => {
            // Стандартный OpenAI-compat ответ: {"data":[{"id":...},...]}
            let count = match resp.json::<serde_json::Value>().await {
                Ok(json) => json
                    .get("data")
                    .and_then(|v| v.as_array())
                    .map(|a| a.len())
                    .unwrap_or(0),
                Err(_) => 0,
            };
            QwenStatus::Available {
                endpoint: endpoint.to_string(),
                model_count: count,
            }
        }
        Ok(resp) => QwenStatus::Unreachable {
            endpoint: endpoint.to_string(),
            error: format!("HTTP {}", resp.status()),
        },
        Err(e) => QwenStatus::Unreachable {
            endpoint: endpoint.to_string(),
            error: format!("{e}"),
        },
    }
}

// ---------------------------------------------------------------------------
// OpenAI streaming payload structures
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ChatRequest<'a> {
    model: &'a str,
    messages: Vec<ChatMessage<'a>>,
    stream: bool,
    temperature: f32,
    max_tokens: u32,
}

#[derive(Debug, Serialize)]
struct ChatMessage<'a> {
    role: &'a str,
    content: &'a str,
}

#[derive(Debug, Deserialize)]
struct StreamChunk {
    #[serde(default)]
    choices: Vec<StreamChoice>,
}

#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: StreamDelta,
}

#[derive(Debug, Deserialize, Default)]
struct StreamDelta {
    #[serde(default)]
    content: Option<String>,
}

// ---------------------------------------------------------------------------
// History type (defined here to avoid cyclic import chat ↔ qwen_bridge)
// ---------------------------------------------------------------------------

pub struct BrainHistoryMsg<'a> {
    pub role: &'a str,    // "owner" | "ceo"
    pub content: &'a str,
}

// ---------------------------------------------------------------------------
// Main runner
// ---------------------------------------------------------------------------

/// Отправляет system + history + user prompt на локальный OpenAI-compat endpoint
/// (Ollama / LM Studio), стримит ответ через SSE, эмитит `ceo-chunk` для UI,
/// возвращает полный текст ответа после `data: [DONE]`.
pub async fn run_qwen(
    system_prompt: &str,
    user_text: &str,
    history: &[BrainHistoryMsg<'_>],
    settings: &AppSettings,
    lifecycle: &ChatLifecycle,
    app: &AppHandle,
) -> Result<String, String> {
    let url = format!(
        "{}/chat/completions",
        settings.qwen_endpoint.trim_end_matches('/')
    );

    let mut messages = vec![ChatMessage {
        role: "system",
        content: system_prompt,
    }];
    for msg in history {
        messages.push(ChatMessage {
            role: match msg.role {
                "owner" => "user",
                _ => "assistant",
            },
            content: msg.content,
        });
    }
    messages.push(ChatMessage {
        role: "user",
        content: user_text,
    });

    let body = ChatRequest {
        model: &settings.qwen_model,
        messages,
        stream: true,
        temperature: 0.3,
        max_tokens: 4096,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.qwen_timeout_sec))
        .build()
        .map_err(|e| format!("qwen client build: {e}"))?;

    log::info!(
        "Step 10: POST {url} (model={}, system_len={}, user_len={})",
        settings.qwen_model,
        system_prompt.len(),
        user_text.len()
    );

    lifecycle.cancel.store(false, Ordering::Relaxed);

    let resp = client
        .post(&url)
        .header("Authorization", "Bearer ollama") // dummy, для OAI-compat
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("qwen request: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("qwen HTTP {status}: {}", text.trim()));
    }

    // SSE парсинг — поток chunks разделённых `\n\n`, каждый chunk это
    // `data: {json}` или `data: [DONE]`.
    let mut byte_stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut accumulated = String::with_capacity(4096);

    while let Some(chunk_result) = byte_stream.next().await {
        if lifecycle.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let chunk = chunk_result.map_err(|e| format!("qwen stream: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        // Парсим сколько полных событий накопилось (separator `\n\n`).
        while let Some(pos) = buffer.find("\n\n") {
            let event: String = buffer.drain(..pos + 2).collect();
            // Каждое событие может содержать несколько `data:` lines —
            // обычно одну. Берём первую line начинающуюся с `data: `.
            for line in event.lines() {
                let payload = match line.strip_prefix("data:") {
                    Some(p) => p.trim(),
                    None => continue,
                };
                if payload == "[DONE]" {
                    return Ok(accumulated);
                }
                if payload.is_empty() {
                    continue;
                }
                match serde_json::from_str::<StreamChunk>(payload) {
                    Ok(parsed) => {
                        if let Some(delta) = parsed
                            .choices
                            .into_iter()
                            .next()
                            .and_then(|c| c.delta.content)
                        {
                            accumulated.push_str(&delta);
                            let _ = app.emit("ceo-chunk", delta);
                        }
                    }
                    Err(e) => {
                        log::warn!("qwen SSE parse: {e} (payload: {payload})");
                    }
                }
            }
        }
    }

    // Поток закрылся без явного [DONE] — возвращаем накопленное.
    Ok(accumulated)
}

// ---------------------------------------------------------------------------
// v1.0.22 Фаза 11C — Qwen для Диспетчера (отдельная модель/timeout/lifecycle)
// ---------------------------------------------------------------------------

use crate::commands::claude_bridge::DispatcherLifecycle;

/// Аналог `run_qwen` но использует `dispatcher_qwen_model` +
/// `dispatcher_routing_timeout_sec` из settings, свой DispatcherLifecycle,
/// и эмитит `dispatcher-chunk` для UI Диспетчера. Никаких изменений в
/// существующем `run_qwen` (Гендир продолжает работать как раньше).
pub async fn run_qwen_for_dispatcher(
    system_prompt: &str,
    user_text: &str,
    settings: &AppSettings,
    lifecycle: &DispatcherLifecycle,
    app: &AppHandle,
) -> Result<String, String> {
    let url = format!(
        "{}/chat/completions",
        settings.qwen_endpoint.trim_end_matches('/')
    );

    let body = ChatRequest {
        model: &settings.dispatcher_qwen_model,
        messages: vec![
            ChatMessage { role: "system", content: system_prompt },
            ChatMessage { role: "user", content: user_text },
        ],
        stream: true,
        temperature: 0.2, // ниже чем у Гендира (0.3) — Диспетчер более detеrministic
        max_tokens: 2048,
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(settings.dispatcher_routing_timeout_sec))
        .build()
        .map_err(|e| format!("dispatcher qwen client build: {e}"))?;

    log::info!(
        "v1.0.22: POST {url} (dispatcher_model={}, system_len={}, user_len={})",
        settings.dispatcher_qwen_model,
        system_prompt.len(),
        user_text.len()
    );

    lifecycle.cancel.store(false, Ordering::Relaxed);

    let resp = client
        .post(&url)
        .header("Authorization", "Bearer ollama")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("dispatcher qwen request: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("dispatcher qwen HTTP {status}: {}", text.trim()));
    }

    let mut byte_stream = resp.bytes_stream();
    let mut buffer = String::new();
    let mut accumulated = String::with_capacity(4096);

    while let Some(chunk_result) = byte_stream.next().await {
        if lifecycle.cancel.load(Ordering::Relaxed) {
            return Err("cancelled".into());
        }
        let chunk = chunk_result.map_err(|e| format!("dispatcher qwen stream: {e}"))?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));

        while let Some(pos) = buffer.find("\n\n") {
            let event: String = buffer.drain(..pos + 2).collect();
            for line in event.lines() {
                let payload = match line.strip_prefix("data:") {
                    Some(p) => p.trim(),
                    None => continue,
                };
                if payload == "[DONE]" {
                    return Ok(accumulated);
                }
                if payload.is_empty() {
                    continue;
                }
                match serde_json::from_str::<StreamChunk>(payload) {
                    Ok(parsed) => {
                        if let Some(delta) = parsed
                            .choices
                            .into_iter()
                            .next()
                            .and_then(|c| c.delta.content)
                        {
                            accumulated.push_str(&delta);
                            let _ = app.emit("dispatcher-chunk", delta);
                        }
                    }
                    Err(e) => {
                        log::warn!("dispatcher qwen SSE parse: {e} (payload: {payload})");
                    }
                }
            }
        }
    }

    Ok(accumulated)
}
