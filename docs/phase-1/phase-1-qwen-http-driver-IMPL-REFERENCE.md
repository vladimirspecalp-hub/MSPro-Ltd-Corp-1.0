# Phase 1 — QwenHttpDriver IMPLEMENTATION REFERENCE

- **Version:** v1.1 (2026-05-28)
- **Author:** Claude Code (skill `mspro-programmer`)
- **Status:** Approved Владельцем + Cursor review (4 фактических правки + 2 опциональных применены) — источник истины для реализации `pal/qwen_http_driver.rs`.
- **Relation to other docs:**
  - Контракт trait — `phase-1-pal-trait-spec.md` v3 (`PostRuntimeProvider`, типы, `ProviderError` variants).
  - Сестринский reference — `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1 (паттерн структуры, §6 наглядное сравнение).
  - Декларативный playbook сборки/релиза — `02-Patterns/rebuild-msi-playbook-v1.0.33.md`.
  - Применён pattern — `02-Patterns/документы-с-кодовыми-путями-из-реального-репозитория.md` (pre-write sync с реальным `qwen_bridge.rs`).

**Назначение документа:** один источник истины «как реально работает Qwen HTTP integration в v1.0.33 (`qwen_bridge.rs`) и как `QwenHttpDriver` должен реализовать `PostRuntimeProvider` trait v3 поверх этого flow». Все цифры строк указаны для `src-tauri/src/commands/qwen_bridge.rs` boevoy v1.0.33 (verified read-only 2026-05-28).

---

## §1. Реальный flow v1.0.33 (источник истины)

Источники: `qwen_bridge.rs` строки 44-83 (health), 138-256 (CEO `run_qwen`), 268-364 (Dispatcher `run_qwen_for_dispatcher`). Оба flow OpenAI-compatible, отличаются только моделью/timeout/temperature/emit-каналом.

### §1.1. Endpoint + HTTP метод + headers

```
URL:    {settings.qwen_endpoint}/chat/completions
        trim_end_matches('/') нормализует endpoint
        (qwen_bridge.rs:146-149, 275-278)

Default endpoint: http://localhost:11434/v1   (Ollama)
Альт. endpoint:   http://localhost:1234/v1    (LM Studio)
Оба OpenAI-compatible — driver работает с обоими без code-branch.

Method: POST

Headers:
- Content-Type: application/json  (через reqwest `.json(&body)`)
- Authorization: Bearer ollama    (qwen_bridge.rs:193, 307 — dummy header
                                   для OAI-compat; Ollama игнорирует)
```

### §1.2. Request body schema (lines 89-102, 169-175, 280-289)

```rust
// Реальная структура в qwen_bridge.rs:
struct ChatRequest<'a> {
    model: &'a str,                      // settings.qwen_model = "qwen3:14b"
    messages: Vec<ChatMessage<'a>>,
    stream: bool,                        // true — SSE streaming
    temperature: f32,                    // 0.3 для CEO, 0.2 для Dispatcher
    max_tokens: u32,                     // 4096 CEO, 2048 Dispatcher
}

struct ChatMessage<'a> {
    role: &'a str,                       // "system" | "user" | "assistant"
    content: &'a str,
}

// Сборка messages (qwen_bridge.rs:151-167):
messages = [
    {role: "system",     content: system_prompt},
    {role: "user"|"assistant", content: history items},  // role mapping
    {role: "user",       content: user_text},
]

// Role mapping в history (qwen_bridge.rs:157-160):
// "owner" → "user"
// любое другое → "assistant"
```

**Для PAL driver:** базовая структура та же. Добавляется поле `stream_options` (см. §7.1) — опционально для парсинга `usage` от Ollama.

### §1.3. SSE streaming response format + accumulator (lines 207-256)

```
Response Content-Type: text/event-stream

Format: chunks разделённые "\n\n", каждое событие = одна или несколько строк,
        обычно одна строка начинающаяся с "data: " + JSON payload или "[DONE]".

Алгоритм accumulator (qwen_bridge.rs:207-256):
1. resp.bytes_stream() → futures Stream<Bytes>           (line 207)
2. накапливаем bytes в String buffer                     (line 216)
3. пока buffer.find("\n\n") → drain до этой точки → parse event  (line 219)
4. event.lines() → strip_prefix("data:") → trim          (line 224)
5. если payload == "[DONE]" → return accumulated         (line 228)
6. иначе serde_json::from_str::<StreamChunk>(payload)    (line 234)
7. choices[0].delta.content → push в accumulated         (line 236-244)
   + (в legacy: app.emit("ceo-chunk", delta) для UI typing-эффекта;
      PAL driver НЕ emit-ит chunks наружу — accumulate only)

StreamChunk schema (qwen_bridge.rs:104-120):
struct StreamChunk { choices: Vec<StreamChoice> }
struct StreamChoice { delta: StreamDelta }
struct StreamDelta { content: Option<String> }

Реальный JSON chunk от Ollama:
{
  "id": "chatcmpl-...",
  "object": "chat.completion.chunk",
  "created": 1234567890,
  "model": "qwen3:14b",
  "choices": [
    {
      "index": 0,
      "delta": { "content": "слово " },
      "finish_reason": null         // финальный chunk: "stop"
    }
  ]
  // usage: {...}  ← опционально, ТОЛЬКО при stream_options.include_usage=true
}

Финальное событие: data: [DONE]\n\n
Если поток закрылся без [DONE] (network drop) — `run_qwen` (line 255)
возвращает накопленное без error. Для PAL driver — warning лог + return Ok.
```

### §1.4. Health check механизм (lines 44-83)

```
GET {endpoint}/models                  (qwen_bridge.rs:45 — НЕ "/api/tags",
                                        это OAI-compat path; ⚠ см. ниже)
Client timeout: 2 sec                  (v1.1 fix: trait v3 §4 требует
                                        health_check ≤2s — для соответствия
                                        правилу «DEC-001 health обновление ≤30s
                                        при poll 5 мин». Legacy `detect_qwen_inner`
                                        использует 4s — driver Phase 1 будет 2s.)

Success response (OAI-compat):
{ "data": [
    {"id": "qwen3:14b", "object": "model", ...},
    {"id": "llama3:8b", ...}
  ]
}

Текущий парсер (line 62-67): `.get("data").and_then(as_array).map(.len())`
— возвращает model_count.

Для PAL HealthStatus mapping (sync с trait v3 §3.4 — 6 states):
- 200 + data.len() > 0          → Alive
- 200 + data.len() == 0         → Unknown + message "no models loaded"
                                  (рекомендация: Alive + warning log; не ошибка
                                  для health, в invoke вернётся ModelUnavailable)
- 401/403                       → AuthFailed (rare для local Ollama;
                                  возможно при LM Studio с custom token)
- network err / timeout 2s      → Unreachable
- HTTP 5xx                      → ServerError
- любая другая ошибка парсинга  → Unreachable + message с диагностикой
```

> ⚠ **v1.1 sync note (Cursor #3):** trait v3 §4 содержит **устаревшее** упоминание `GET /api/tags` (native Ollama API path) для `health_check` Qwen. **Источник истины для driver — этот reference §1.4 (GET `/models` — OpenAI-compat path, verified в `qwen_bridge.rs:45`).** `/models` работает на Ollama + LM Studio + любом OAI-compat backend; `/api/tags` — только native Ollama. Micro-backlog: обновить trait §4 формулировку с `/api/tags` на `/models` (это поправка к чистоте документа; код driver уже правильный).

### §1.5. Cancellation (для PAL — через drop future, не AtomicBool)

Реальный `qwen_bridge::run_qwen` использует `lifecycle.cancel: Arc<AtomicBool>` + явная проверка в каждой итерации цикла чтения SSE (lines 189, 212, 303, 324) — это **legacy паттерн** для UI cancel-кнопки Гендира/Диспетчера.

**Для PAL `QwenHttpDriver`:**
- НЕ нужен `AtomicBool`. Orchestrator делает `tokio::time::timeout(outer, driver.invoke(req))` — при истечении future дропается.
- При drop futures `reqwest` in-flight request **сам отменяется** (tokio runtime закрывает TCP socket).
- НЕ нужен `kill_on_drop` (это паттерн для subprocess, не HTTP).
- НЕ нужен явный signal handler — Rust Drop trait делает всё автоматически.

**Phase 2 backlog (не Phase 1):** если появится UI cancel для пост-задач — можно либо вернуть AtomicBool в orchestrator (общий механизм для всех driver), либо использовать `tokio::sync::Notify` / `CancellationToken`. Не блокер Phase 1 — cancel пост-задач сейчас = ручной fail через `dispatcher::fail_task_inner`.

### §1.6. Settings поля (settings/mod.rs lines 50-103)

```rust
qwen_endpoint:       String  // default "http://localhost:11434/v1"
qwen_model:          String  // default "qwen3:14b"
qwen_timeout_sec:    u64     // default 120 (legacy для CEO, не PAL)
qwen_context_tokens: u32     // default 32_000 (для context_assembler.rs)
auto_fallback_qwen:  bool    // default true (Claude → Qwen в claude_bridge,
                             //               не PAL fallback chain)
dispatcher_qwen_model: String // default "qwen3:14b" (Dispatcher router)
```

**Для PAL driver Phase 1:**
- `qwen_endpoint` → `QwenHttpDriver::endpoint: String` (immutable после `new()`).
- `qwen_model` → `QwenHttpDriver::default_model: String`.
- `qwen_timeout_sec`, `auto_fallback_qwen` — **легаси для CEO/`claude_bridge::*`**, НЕ используются PAL (см. §5).
- `qwen_context_tokens` — относится к `context_assembler.rs` (бюджет истории при сборке prompt), не к driver.

### §1.7. Reqwest client config (lines 46-49, 177-180, 291-294)

```rust
// Реальный код v1.0.33:
let client = reqwest::Client::builder()
    .timeout(Duration::from_secs(settings.qwen_timeout_sec))  // 120s CEO
    .build()
    .map_err(|e| format!("qwen client build: {e}"))?;
```

**Для PAL driver:**
- НЕ ставим outer timeout (orchestrator владеет через `tokio::time::timeout`).
- **Можно** internal safety cap `.timeout(Duration::from_secs(600))` = hard cap из trait v3 §6 — защита от reqwest hang без drop сигнала. Это **второй timeout** (outer от orchestrator, inner safety от driver), и они не конфликтуют — orchestrator истечёт первым в обычном случае.
- `rustls-tls` уже в features (Cargo.toml line 46: `reqwest = { ..., features = ["json", "stream", "rustls-tls"] }`) — TLS работает на любых endpoints (включая https reverse-proxy если кто-то поставит nginx перед Ollama).
- `Client::builder()` строится **один раз** в `QwenHttpDriver::new()`, переиспользуется на все invoke (connection pooling).

---

## §2. Integration boundary — что PAL берёт / что post_executor оставляет

Аналог §2 ClaudeCli IMPL-REFERENCE. Для Qwen существенное отличие: **нет filesystem-based artifacts**.

| Операция | post_executor (без изменений) | QwenHttpDriver::invoke |
|---|---|---|
| Tauri state lookup (`WritePool`, `VaultState`, `SettingsStore`, `PostExecutorRegistry`) | ✅ остаётся | — |
| Posts lookup + system_prompt + model resolve | ✅ остаётся | — |
| `ensure_post_agent_md` | ❌ **НЕ вызывается для Qwen** — нет CLI `--agent` | — |
| `task_outbox_dir` mkdir | ✅ остаётся (для UI Awaiting consistency) | — |
| Registry duplicate guard | ✅ остаётся | — |
| `pre_snapshot` + `diff_dir` + `register_artifact` | ✅ остаётся (см. ниже) | — |
| HTTP POST + SSE parse + accumulate | — | ✅ в driver |
| Cancel | — | через drop future (нет AtomicBool) |

### Архитектурное решение Phase 1: Qwen artifacts

Qwen возвращает **text-only** ответ — не пишет файлы как Claude через native Write tool. Два варианта обработки в post_executor:

**Вариант A (рекомендуется Phase 1 MVP — consistency с Claude flow):**
- После `pal.invoke()` для Qwen-постов: post_executor **сам пишет** `response.text` → `Outbox/<task_id>/result.txt`.
- Затем стандартный `diff_dir` подхватит `result.txt` как artifact.
- UI Awaiting показывает task с одним artifact (как Claude посты).
- Approve flow одинаковый для обоих видов постов.

**Вариант B (требует UI правок — отложено):**
- post_executor пишет text напрямую в `dispatcher_logs.result_text_inline` (новое поле migration 10?).
- UI Awaiting рендерит inline text без artifact.
- Более естественно для chat-ответов, но ломает существующий approve UX.

**Phase 1 решение: Вариант A.** post_executor для Qwen-flow делает `fs::write(task_dir.join("result.txt"), &response.text)` после успешного `pal.invoke()`. `diff_dir` подхватит, всё едино с Claude.

### Net delta для post_executor

Из IMPL-REFERENCE ClaudeCli §2.3: net `140 → ~90 строк`. Для Qwen-flow добавится **+5 строк** (write result.txt из response.text) — итого `~95 строк` если поддерживаются оба провайдера в одной функции. Acceptable.

---

## §3. ProviderResponse контракт для Phase 1

### §3.1. Поля + значения для QwenHttpDriver Phase 1 MVP
*(сверено с trait v3 §3.2 `ProviderResponse` — 7 полей)*

| Поле | QwenHttpDriver Phase 1 значение | Источник |
|---|---|---|
| `text: String` | `accumulated` String после получения `[DONE]` или закрытия потока | SSE accumulator (qwen_bridge.rs:209-256) |
| `usage: TokenUsage` | **Если `stream_options.include_usage=true`** — реальные `prompt_tokens`/`completion_tokens` из финального chunk перед `[DONE]`. **Иначе** `(0, 0, 0, 0)` + warning лог. См. §7.1. | Ollama API (опционально) |
| `latency_ms: u64` | `Instant::now() - started_at` (полный invoke wall-clock включая HTTP setup + stream) | local time |
| `model_used: String` | `request.model_override.clone().unwrap_or(self.default_model.clone())` | echo input |
| `provider_used: ProviderKind::QwenHttp` | constant для драйвера | trait v3 |
| `stop_reason: String` | `"end_turn"` если получили `[DONE]`. Финальный chunk choices[0].finish_reason если есть и не null (обычно `"stop"`). | соглашение Phase 1 (enum — Phase 2) |
| `artifacts: Vec<ArtifactRef>` | `vec![]` (Qwen не пишет файлы) | trait v3 §3.2 «Phase 1 always empty» |

### §3.2. ProviderRequest — какие поля используются / игнорируются

| Поле | QwenHttp использование |
|---|---|
| `system_prompt: String` | → `messages[0] = {role: "system", content: system_prompt}` |
| `user_message: String` | → `messages[last] = {role: "user", content: user_message}` |
| `tier: Tier` | T3 ожидается; orchestrator берёт `Tier::default_timeout(T3) = 60s` для outer timeout |
| `timeout: Option<Duration>` | hint для orchestrator, driver не применяет |
| `max_turns: Option<u32>` | **не используется** (Qwen single-turn в Phase 1; multi-turn — Phase 2 tool-loop) |
| `model_override: Option<String>` | если `Some` — переопределяет `default_model` в request body |
| `workspace_path: Option<PathBuf>` | **игнорируется** silently (Qwen без sandbox; trait v3 §3.1 явно описывает это как valid case — driver НЕ возвращает `BadRequest`, в отличие от ClaudeCli с `None` workspace) |
| `agent_slug: Option<String>` | **игнорируется** silently (нет CLI `--agent`); `Some(_)` не вызывает ошибку — driver просто не использует поле. Trait v3 §3.1 описывает поле как «для Qwen/External — None». |
| `mcp_bindings: Vec<McpBinding>` | **игнорируется** с warning лог (см. §4) |
| `trace: RequestTrace` | прокидывается в `run_logs` (post_slug, task_id, dispatcher_log_id) для аудита |

### §3.3. ProviderError mapping (HTTP → trait v3)
*(сверено с trait v3 §3.3 — 11 variants; `Auth(String)`, `Server(String)`, `Network(String)` — все tuple)*

| Условие | ProviderError variant (v3) | `should_fallback()` |
|---|---|---|
| `reqwest::Error::is_connect()` / DNS / connection refused | `Network(format!("connect: {e}"))` | true |
| `reqwest::Error::is_timeout()` — **driver НЕ возвращает сам** (это «inner reqwest timeout»; внешний выставляет orchestrator через `tokio::time::timeout`) | (orchestrator формирует `Timeout { timeout_secs }`) | — |
| HTTP 401 | `Auth("Ollama auth failed: {body_first_line}".to_string())` | true |
| HTTP 403 | `Auth("Ollama forbidden: {body}".to_string())` | true |
| HTTP 404 + body содержит "model" (или request с явной моделью) | `ModelUnavailable("model {m} not found in Ollama; try `ollama pull {m}`".to_string())` | true |
| HTTP 400 / unparseable JSON body | `BadRequest(body_first_line)` | **false** (логическая ошибка, fallback не поможет) |
| HTTP 422 (LM Studio validation) | `BadRequest(body_first_line)` | **false** |
| HTTP 429 | `QuotaExceeded(body_first_line)` (rare local Ollama, возможно LM Studio с rate limit) | true |
| HTTP 5xx | `Server(format!("HTTP {status}: {body}"))` | true |
| SSE parse fail (broken JSON chunk) | log::warn + skip chunk; если 0 valid chunks → `Server("no parsable chunks".to_string())` | true |
| Ollama `data: {"error": "..."}` event внутри SSE (model not found mid-stream) | `ModelUnavailable("Ollama error: {error_msg}".to_string())` | true |
| HTTP success но `[DONE]` не получили (поток закрылся) | возвращаем `Ok(ProviderResponse { text: accumulated })` + warning лог; **НЕ** error | — |

**Конкретные конструкторы (для копирования в код драйвера):**
```rust
return Err(ProviderError::Network(format!("connect: {e}")));
return Err(ProviderError::Auth(format!("Ollama auth failed: {body}")));
return Err(ProviderError::ModelUnavailable(format!("model {model} not found in Ollama")));
return Err(ProviderError::BadRequest(body_first_line.to_string()));
return Err(ProviderError::QuotaExceeded(body_first_line.to_string()));
return Err(ProviderError::Server(format!("HTTP {status}: {body}")));
// ProviderError::Other(...) — НЕ использовать в driver (last-resort)
```

---

## §4. MCP политика Phase 1

**`QwenHttpDriver::capabilities().supports_mcp = false` — hard.**

### Обоснование
Qwen 3 натренирован на Hermes-style XML `<tool_call>` (явный комментарий в `qwen_bridge.rs:13` — «этот формат как раз и создан Nous Research на основе Qwen-семейства»). Это значит модель **умеет** генерировать XML tool-calls, но:
- Ollama API сам по себе **не имеет** встроенного tool_use protocol (как Claude API с `tool_use`/`tool_result` content blocks).
- MCP servers — отдельные процессы; tool-loop оркестрация (вызвал → выполнил MCP → ответ обратно → повторил) должна жить внутри driver или orchestrator.
- В Phase 1 это **выходит за scope** PAL MVP.

**Поведение Phase 1:** `request.mcp_bindings` игнорируются с warning лог:
```rust
if !request.mcp_bindings.is_empty() {
    log::warn!(
        "QwenHttpDriver Phase 1 MVP: dropping {} mcp_bindings (per-post MCP не поддерживается, Phase 2 R&D)",
        request.mcp_bindings.len()
    );
}
```

### Phase 2+ R&D backlog
Добавить XML tool-loop в `QwenHttpDriver`:
1. Driver формирует system prompt с описанием доступных MCP tools в XML-формате.
2. Парсит Qwen response на наличие `<tool_call>...</tool_call>` блоков.
3. Выполняет tool через MCP gateway (отдельный сервис).
4. Отдаёт `tool_result` обратно Qwen в новом messages turn.
5. Цикл до `<final_answer>` или `max_turns`.

Это **сложная work item** (нужны Hermes prompt template + MCP shim layer + tool-loop limit). Не блокер Phase 1.

---

## §5. Timeout правильная attribution

### §5.1. Расхождение: settings.qwen_timeout_sec=120 vs Tier::T3=60

| Источник | Значение | Назначение | Файл/строка |
|---|---|---|---|
| `settings.qwen_timeout_sec` | **120** (default) | `reqwest::Client.timeout` в legacy `run_qwen` (CEO Гендир) | `qwen_bridge.rs:178` + `settings/mod.rs:124` |
| `settings.dispatcher_routing_timeout_sec` | **60** (default) ¹ | Legacy `run_qwen_for_dispatcher` | `qwen_bridge.rs:292` + `settings/mod.rs:138` |
| `Tier::T3::default_timeout()` | **60** | PAL orchestrator outer timeout для T3 (Qwen рутина) | trait v3 §3.4 |

¹ **Сноска (v1.1 fix):** `settings.json` у Владельца **может содержать другое значение** — например, live в момент написания этого reference было `dispatcher_routing_timeout_sec: 180`. Это **legacy override** из ранних настроек; **default** в коде = 60 (verified `settings/mod.rs:138 default_dispatcher_routing_timeout() -> u64 { 60 }`). PAL driver не читает это поле — оно остаётся для `claude_bridge::run_claude_cli_for_dispatcher` (CEO/Дисп legacy путь). В предыдущей редакции v1 было ошибочно указано 180 как default (перепутано с `claude_cli_timeout_sec=360`, который в живом settings ранее был 180 до bump'a 2026-05-24).

**Откуда несоответствие:**
- `qwen_timeout_sec = 120` — был выбран на Шаге 10 (полгода назад) когда Qwen рассматривался как fallback для CEO (где задача = «думать 1-2 минуты, потом ответить text»). 120s — комфортный margin.
- `Tier::T3 = 60s` — design decision Phase 1: Qwen для **рутины** (короткие тексты, ОТК, копирайтер). CEO остаётся на legacy `claude_bridge` (НЕ PAL в Phase 1, см. DoD §«Phase 1 scope vs DEC»).

### §5.2. Решение Phase 1
- **PAL для пост-агентов с T3:** orchestrator timeout = `Tier::T3 = 60s`. Этого достаточно для типичных Qwen-задач (короткий ответ <100 токенов output).
- **`settings.qwen_timeout_sec = 120s`** остаётся только для legacy CEO/Гендир (`qwen_bridge.rs::run_qwen`) — не PAL.
- **Driver internal safety cap = 600s** в `reqwest::Client::builder().timeout(...)` — защита от хунгового HTTP без сигнала о drop. Это `hard_cap` из trait v3 §6 — в нормальном flow никогда не срабатывает (orchestrator истечёт первым на 60s).
- **Tier применимость:**
  - **T3 (60s)** — основной use case Qwen в Phase 1: рутина, простой ОТК, копирайтер коротких текстов.
  - **T2 (360s)** — если Qwen используется для длинных документов (rare; обычно T2 = Claude Sonnet). Driver работает корректно.
  - **T1 (600s)** — Qwen 14b не сравним с Opus, нет смысла использовать; driver работает, но качество слабое.

### §5.3. Sanity check для имплементации
```rust
// В driver или orchestrator startup:
debug_assert!(
    Tier::T3.default_timeout().as_secs() <= settings.post_executor_timeout_sec,
    "Tier::T3 timeout ({}) must be <= post_executor_timeout_sec ({}) hard cap",
    Tier::T3.default_timeout().as_secs(),
    settings.post_executor_timeout_sec
);
```

---

## §6. Отличия от ClaudeCliDriver (наглядно)

Этот раздел — **главный contrast** для имплементации; помогает не перепутать паттерны.

| Аспект | ClaudeCliDriver | QwenHttpDriver |
|---|---|---|
| **Mechanism** | subprocess (`claude.exe` через `tokio::process::Command`) | HTTP (`reqwest::Client.post(...)`) |
| **Sandbox** | `current_dir = workspace_path` (Outbox/<task_id>) | НЕТ |
| **stdin format** | plain text `refined_prompt` (без JSON-обёртки) | JSON `{model, messages, stream, temperature, max_tokens, ...}` |
| **stdout/response format** | plain text (`--output-format text`) | SSE `data: {json}\n\n` chunks + `[DONE]` |
| **Cancel mechanism** | `kill_on_drop(true)` + Tokio убивает subprocess при drop | `reqwest` drop отменяет in-flight request (TCP close) |
| **Artifacts source** | filesystem `diff_dir` в post_executor (агент сам создаёт файлы через Write tool) | **НЕТ** (text-only; post_executor пишет `result.txt = response.text` как Phase 1 MVP) |
| **Usage tokens** | **0 в Phase 1** (`--output-format text` не содержит) | **Реальные** если `stream_options.include_usage=true` (§7.1); иначе 0 |
| **MCP support** | `false` Phase 1 (R&D Phase 2 через `.mcp.json` в CWD) | `false` Phase 1 (R&D Phase 2 через XML tool-loop) |
| **Cost per 1k tokens** | `(15.00, 75.00)` USD Opus / `(3.00, 15.00)` Sonnet | `(0.0, 0.0)` (локальная модель, бесплатно) |
| **Default model** | `claude-opus-4-7` (T1) / `claude-sonnet-4-6` (T2) | `qwen3:14b` (T3) |
| **Default tier** | T1 (Opus) / T2 (Sonnet) | T3 |
| **Endpoint config** | `settings.claude_cli_path` (filesystem path) | `settings.qwen_endpoint` (URL) |
| **Orchestrator outer timeout** | 600s (T1 = `post_executor_timeout_sec`) | 60s (T3) |
| **`agent_slug` использование** | `--agent mspro-{slug}` argv | игнорируется |
| **`workspace_path` использование** | sandbox через `current_dir` | игнорируется |
| **Auth** | OAuth Claude (читается CLI из `~/.claude/`) | dummy `Authorization: Bearer ollama` header |
| **`hide_console`** | требуется на Windows (`CREATE_NO_WINDOW`) | НЕ требуется (HTTP, нет окна) |
| **Driver state** | stateless по запросам | `reqwest::Client` reused (connection pool) |
| **Health check** | `claude --version` subprocess (≤2 сек) | `GET /models` HTTP (≤2s PAL; legacy `detect_qwen_inner` 4s) |
| **Error mapping source** | stderr text patterns + exit codes | HTTP status codes + body inspection |

---

## §7. Open questions (новые в Phase 1)

### §7.1. `stream_options: {include_usage: true}` — включать ли Phase 1?

**Проблема:** В Ollama API при `stream: true` финальный chunk **не содержит** `usage`. Если добавить в request body `"stream_options": {"include_usage": true}`, Ollama пришлёт перед `[DONE]` дополнительный event:
```
data: {"choices": [], "usage": {"prompt_tokens": 245, "completion_tokens": 312, "total_tokens": 557}}\n\n
data: [DONE]\n\n
```

**Преимущества:** реальный `usage` → `run_logs.tokens_in`/`tokens_out` → cost dashboard показывает контекст-потребление и output-размер (нужно для T1/T2/T3 sizing decisions и для проверки качества модели). Даже если cost_per_1k=0 — численная метрика ценна.

**Риски:** не все OAI-compat backend поддерживают `stream_options` (старые версии LM Studio могут вернуть HTTP 400 «unknown field»). Это решается test:
- Если первый invoke вернул 400 «stream_options» → driver запомнит флаг `supports_stream_options: false` (in-memory state) → следующие запросы без этого поля. Это compatibility layer.

**Рекомендация Phase 1:** **включить** `stream_options.include_usage = true` с graceful fallback:
1. Driver добавляет поле в request.
2. Парсит финальный chunk с `usage`.
3. Если 400 — `driver.supports_stream_options.store(false)` → retry без поля → usage=(0,0,0,0) + warning лог.

Реализуемо в ~20 строк дополнительного кода. Не блокер.

### §7.2. `stream: true` vs `stream: false` для PAL invoke

**Текущий `run_qwen`** использует `stream: true` для UI typing-эффекта (emit `ceo-chunk` events каждый delta — `qwen_bridge.rs:243`).

**Для PAL driver Phase 1:**
- **`stream: true`** остаётся (driver внутри accumulate в String, **не emit** events наружу). Это совместимо с `Capabilities.supports_streaming = false` (PAL не отдаёт chunks наружу через trait; driver inner streams).
- Преимущество: long-running response не висит в одном HTTP wait — данные приходят постепенно, easier для timeout detection.

**Phase 2 backlog:** если trait добавит `stream() → impl Stream<Chunk>` метод, `QwenHttpDriver` сможет проксировать chunks наружу через `tokio_stream::wrappers::ReceiverStream`. Сейчас это не нужно.

**Альтернатива (`stream: false`):** Ollama возвращает один JSON ответ целиком. Проще парсинг (один `serde_json::from_str`), но медленнее на длинных ответах (нужно дождаться полного результата). **НЕ рекомендуется** — текущая SSE инфраструктура стабильная.

### §7.3. LM Studio vs Ollama parity

Driver работает с `qwen_endpoint` URL — не знает что за бэкенд (Ollama / LM Studio / vLLM / любой OAI-compat). API surface одинаковый.

**Risk:** LM Studio может иметь отличия:
- response schema (доп. поля, missing usage даже при `stream_options`).
- error format (HTTP 422 вместо 400 для некоторых validation errors).
- model list format (data array но id-формат другой, например без префикса `qwen3:`).

**Mitigation:** `health_check()` пробует `/models`, parsing tolerant (не падает на extra fields). Error mapping (§3.3) включает 422 → `BadRequest`. Если будут проблемы — backlog Phase 1.5 «LM Studio compatibility shim» (отдельный driver `LmStudioDriver` или feature flag в `QwenHttpDriver`).

**Phase 1 решение:** один driver на оба backend, integration-тест **только на Ollama** (Владелец использует Ollama); LM Studio support — best-effort, фиксируем в backlog.

### §7.4. Ollama "model not found" — 404 или 400 или SSE error?

**Ollama специфика (verified на Ollama v0.4+):**
- **Streaming mode (`stream: true`):** запрос проходит, HTTP 200 OK; в первом же SSE event приходит:
  ```
  data: {"error": "model 'nonexistent' not found, try pulling it first"}\n\n
  ```
  Затем поток закрывается без `[DONE]`.
- **Non-streaming mode:** HTTP 404 + JSON body `{"error": "model not found"}`.

**Driver обработка Phase 1:**
- При `stream: true` парсить **каждый chunk** на ключ `error` (рядом с `choices`). Если есть — вернуть `ProviderError::ModelUnavailable(error_msg)`.
- Test: request с заведомо неверной моделью (`qwen-this-does-not-exist:bogus`) → driver возвращает `ModelUnavailable`, не `Server` / `Other`.

### §7.5. Cancel: AtomicBool legacy vs drop future PAL

`qwen_bridge.rs::run_qwen` использует `lifecycle.cancel: Arc<AtomicBool>` для UI cancel-кнопки CEO/Дисп. PAL trait v3 §6 говорит: orchestrator владеет timeout через `tokio::time::timeout`, driver полагается на drop future.

**Решение Phase 1:** `QwenHttpDriver` НЕ использует AtomicBool. Cancel-кнопка UI работает **только** для legacy `qwen_bridge.rs` paths (CEO/Гендир + Dispatcher router) — не для пост-агентов через PAL.

**Cancel пост-задачи в Phase 1** = ручной fail через UI:
- Владелец в Awaiting → кнопка «Cancel task» → backend вызывает `dispatcher::fail_task_inner(task_id, "user cancelled")`.
- task ставится failed; in-flight PAL invoke добежит до конца (или истечёт по 60s) и его результат игнорируется.

Это **acceptable Phase 1**; полноценный cancel пост-агентов с прерыванием in-flight HTTP — Phase 2 backlog (через `CancellationToken` в `ProviderRequest`).

---

## §8. Verification чек-лист (для пост-имплементации)

Запускать ПОСЛЕ команды Владельца на имплементацию + `cargo build` без ошибок.

### §8.1. Unit-тесты (без сети)
1. **`build_request_body`:** mock `ProviderRequest { system_prompt: "you are helpful", user_message: "ping", mcp_bindings: [Foo], model_override: Some("qwen3:32b") }` → `ChatRequest { model: "qwen3:32b", messages: [{role:"system",...}, {role:"user","ping"}], stream: true, stream_options: {include_usage: true}, temperature: 0.3, max_tokens: 4096 }`. Verify: `mcp_bindings` игнорируются с warning лог.
2. **`parse_sse_chunk` accumulator:** прогнать 6 chunks:
   - regular `delta.content = "hello "`
   - regular `delta.content = "world"`
   - empty `delta` (skip)
   - chunk с `usage: {prompt_tokens: 10, completion_tokens: 5}`
   - malformed JSON (log warn, skip, не падать)
   - `data: [DONE]` (exit loop с `Ok(accumulated)`)

   Verify: `accumulated == "hello world"`, `usage == (10, 5, 0, 0)`.
3. **`map_http_status_to_error`:** 7 сценариев (401 → Auth, 403 → Auth, 404+model body → ModelUnavailable, 400 → BadRequest, 422 → BadRequest, 429 → QuotaExceeded, 500/502 → Server) → правильный `ProviderError` variant + verified `should_fallback()`.

### §8.2. Integration-тесты (mock-Ollama)

> ⚠ **Prerequisite (v1.1 fix, Cursor #4):** ни `wiremock`, ни `httpmock` сейчас **не присутствуют** в `src-tauri/Cargo.toml` `[dev-dependencies]` (там только `tempfile = "3"`). **Перед написанием integration tests** добавить в `src-tauri/Cargo.toml`:
> ```toml
> [dev-dependencies]
> tempfile = "3"
> # Один из (выбор автора имплементации):
> wiremock = "0.6"     # Async-first, удобный matcher API, отлично работает с reqwest.
> # или
> httpmock = "0.7"     # Sync API + async, проще mental model.
> ```
> Рекомендация: **`wiremock = "0.6"`** — async-native (Phase 1 driver полностью async на tokio), better integration с reqwest, no extra runtime setup. `cargo add --dev wiremock@0.6` в `src-tauri/`.

4. **Happy path:** `wiremock-rs` мокает `POST /chat/completions` с SSE stream «Hello world\n[DONE]» → driver `invoke()` возвращает `ProviderResponse { text: "Hello world", usage: parsed, latency_ms: <100ms, model_used: "qwen3:14b", provider_used: QwenHttp, stop_reason: "end_turn", artifacts: vec![] }`.
5. **Drop при orchestrator timeout:** mock-Ollama sleep 5s между chunks → orchestrator `tokio::time::timeout(Duration::from_secs(1), driver.invoke(...))` → driver future dropped → mock receives TCP disconnection within 100ms (verify через `wiremock` `received_count` или log inspection).
6. **`health_check`:**
   - mock `/models` returns `{"data": [{"id": "qwen3:14b"}]}` → `HealthStatus::Alive`.
   - mock returns 503 → `Unreachable`.
   - mock connection refused (no listener) → `Unreachable`.
   - mock returns `{"data": []}` (empty) → `Alive` с warning лог (или `Unknown` — выбрать).

### §8.3. E2E (на реальном Ollama)
7. **Smoke E2E:** запустить реальный Ollama с `ollama pull qwen3:14b` → создать тестовый пост `qwen-test-pod` (post_runtime: `tier=T3, primary_provider=qwen_http, primary_model=qwen3:14b`) → дать задачу «верни одно слово: ok» через post_executor → проверить:
   - `Outbox/<task_id>/result.txt` существует и содержит «ok» (или похожее короткое).
   - `run_logs.provider_used = "qwen_http"`, `model_used = "qwen3:14b"`.
   - `tokens_in > 0`, `tokens_out > 0` (если `include_usage` работает) — это положительная проверка §7.1.
   - `latency_ms` в разумных пределах (< 60_000).
   - UI Awaiting показывает task с одним artifact (result.txt).

---

## §9. Связанные документы

- **trait контракт** — `phase-1-pal-trait-spec.md` **v3 (approved 2026-05-26)** — actual source of truth для типов и сигнатур (`PostRuntimeProvider`, `ProviderRequest`, `ProviderResponse`, `ProviderError`, `Tier`, `HealthStatus`).
- **Сестринский driver reference** — `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1 (паттерн структуры; §6 наглядное сравнение в этом документе ссылается на ClaudeCli аналогично).
- **DoD контекст** — `phase-1-definition-of-done.md` v1.1 (AC-002.3 = «QwenHttpDriver реализован», `commands/post_executor.rs` интеграция).
- **Risk register** — `phase-1-risk-register.md` **v1.1** (R-T-009 fallback chain exhaustion, R-T-014 Test connection blocking — оба применимы к QwenHttp; v1.1 содержит бизнес/процессные риски Гендира + cross-links Cursor).
- **БД схема** — `phase-1-current-db-schema.sql` (v1.0.33, 13 таблиц) + migration 08 добавит `provider_registry` row для `qwen_http`.
- **Boevoy код для сверки при имплементации:**
  - `src-tauri/src/commands/qwen_bridge.rs` — legacy run_qwen + run_qwen_for_dispatcher (паттерны health, SSE, OAI-compat). НЕ копировать AtomicBool cancel в driver.
  - `src-tauri/src/settings/mod.rs` — 6 qwen-полей (особенно `qwen_endpoint` + `qwen_model`).
- **Применённый pattern** — `02-Patterns/документы-с-кодовыми-путями-из-реального-репозитория.md` (этот reference написан pre-write sync с `qwen_bridge.rs`).

---

## §10. Changelog

- **v1.1 followup (2026-05-28)** — Cursor косметика: §6 сравнительная таблица строка «Health check» синхронизирована с §1.4 (`≤4 сек` → `≤2s PAL; legacy detect_qwen_inner 4s`) — устранено внутреннее расхождение.
- **v1.1 (2026-05-28)** — Cursor verify: 4 фактических правки + 2 опциональных применены.
  1. **§5.1 timeout таблица** (Cursor #1): `settings.dispatcher_routing_timeout_sec` default **180 → 60** (verified `settings/mod.rs:138` — `default_dispatcher_routing_timeout() -> u64 { 60 }`). 180 был ошибочно перенесён из live settings.json у Владельца (legacy override). Сноска в таблице.
  2. **§1.4 health timeout** (Cursor #2): **4s → 2s** для PAL driver (соответствие trait v3 §4 «health_check ≤2s»). Legacy `detect_qwen_inner` остаётся на 4s.
  3. **§1.4 / §9 `/api/tags` устаревший** (Cursor #3): добавлена sync-note — trait v3 §4 содержит устаревшее упоминание `/api/tags`; driver использует `/models` (OAI-compat); micro-backlog для trait update.
  4. **§8.2 dev-dependencies** (Cursor #4): явное prerequisite — добавить `wiremock = "0.6"` (рекомендуется) или `httpmock = "0.7"` в `src-tauri/Cargo.toml [dev-dependencies]` ДО integration tests; сейчас там только `tempfile`.
  5. **§3.2 agent_slug** (опционально): уточнено что `Some(_)` для Qwen silently игнорируется без ошибки (trait v3 §3.1 явно описывает); аналогично для `workspace_path: Some`.
  6. **§9 risk register** (опционально): ссылка `v1.0` → `v1.1` (актуальная версия с бизнес/процессными рисками Гендира + Cursor cross-links).
- **v1 (2026-05-28):** первая редакция. Read-only анализ `qwen_bridge.rs` v1.0.33 (365 строк) + `settings/mod.rs` qwen-fields + trait v3 §3.2-3.4 + ClaudeCli IMPL-REFERENCE v1.1 (для паттерна структуры). 10 разделов, 5 open questions с рекомендациями. По решению Владельца этот документ — источник истины для реализации `pal/qwen_http_driver.rs` на Этапе 2 Phase 1.

*End of QwenHttpDriver IMPL REFERENCE.*
