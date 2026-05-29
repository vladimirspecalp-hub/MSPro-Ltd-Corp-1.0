# Phase 1 SPEC — раздел 1: PAL trait PostRuntimeProvider

**Версия:** v3
**Дата:** 2026-05-26
**Статус:** Approved (Owner + Cursor)
**Source of truth:** этот файл
**Связанные артефакты:**
- `Vault/decisions-log.md` — DEC-001 (Service Bureau + PAL), DEC-002, DEC-003
- `Vault/03-Phases/phase-1-current-db-schema.sql` — актуальная схема БД v1.0.33
- `Vault/02-Patterns/документация-следует-за-работающим-кодом-а-не-наоборот.md`
- ClaudeCliDriver implementation reference — ведёт программист (по реальному коду v1.0.33)

---

## Changelog

### v3 (2026-05-26) — реальность v1.0.33 vs гипотезы

Программист сделал экспертный анализ skeleton ClaudeCliDriver против реального кода и вскрыл что trait v2 содержит несколько архитектурных допущений, расходящихся с фактическим post_executor flow. Правки:

- **ProviderRequest.workspace_path: Option<PathBuf>** — добавлено. Для `ClaudeCliDriver` это Outbox/<task_id> sandbox через `Command::current_dir`. Для Qwen/ExternalGateway = None.
- **ProviderRequest.agent_slug: Option<String>** — добавлено. Для `--agent mspro-{slug}` Claude CLI. Explicit поле, не выводится из `trace.post_slug` — это разные семантики (trace.post_slug = origin задачи, agent_slug = конкретный CLI agent profile).
- **ProviderRequest.model_override: Option<String>** — добавлено. Per-request override Tier-дефолтной модели. Нужен для hot-swap модели без создания нового driver instance (DEC-002 «UI Model Switcher»).
- **Tier presets ИСПРАВЛЕНЫ:** T1=600s (было 360), T2=360s (было 180), T3=60s. Hard cap = 600s = `post_executor_timeout_sec`. Обоснование в §7 v3.
- **ProviderResponse.artifacts** — помечено «Phase 1: всегда empty». В реальном flow post_executor сам сканирует Outbox через diff_dir; driver не владеет sandbox-семантикой и не должен пытаться агрегировать артефакты. В Phase 2 stream-json driver может агрегировать `tool_use_results`.
- **stop_reason** — оставлено `String` для Phase 1. Enum нормализация отложена в Phase 2 (multi-turn/tool-loop потребуют структурного типа).
- **§7 Timeout reconciliation** — переписана. В v2 ошибочно связывал `claude_cli_timeout_sec=360` с post_executor. Факт: в проекте три разных таймаута для трёх разных runtime-каналов (CEO/Гендир, dispatcher router, post_executor). Tier::T1=600 соответствует именно `post_executor_timeout_sec`.

### v2 (2026-05-25) — после Cursor review

- FIX #1 `RequestTrace.dispatcher_log_id: Option<String>` (TEXT в БД, не INTEGER).
- FIX #2 убран phantom `ProviderConfig`, default timeout через `Tier::default_timeout()`.
- FIX #3 `ProviderError::Timeout { timeout_secs: u64 }` — serde-friendly.
- FIX #4 timeout ownership: orchestrator владеет outer timeout, driver — нет.
- FIX #5 §7 Timeout reconciliation таблица (в v3 пересмотрена).
- §8 Phase 1 MVP scope интеграции + 7-шаговый integration path + feature flag.

### v1 (2026-05-25) — первоначальный draft

Исходный SPEC trait по DEC-001 acceptance criteria.

---

## 1. Цель

Единый Rust-trait для всех провайдеров LLM в MSPro AgentPod. Обеспечивает:

- Vendor-agnostic вызов модели (Claude CLI / Qwen HTTP / External Gateway / любой будущий).
- Унифицированный учёт токенов и стоимости.
- Контролируемая fallback chain при сбоях.
- Health observability для UI Service Bureau (DEC-001).
- Возможность hot-swap модели/провайдера через UI без рестарта (DEC-002, DEC-003).

Trait — это **контракт**. Конкретные имплементации (driver-ы) — отдельные SPEC + реальный код. Driver implementation reference ведёт программист по фактическому коду v1.0.33.

---

## 2. Архитектурный контекст

```
┌─────────────────────────────────────────────────────────────┐
│  Pod Runtime (post_executor.rs в Phase 1)                  │
│  читает posts.preferred_model → Tier → выбирает provider   │
└────────────────────────┬────────────────────────────────────┘
                         │
                         ▼
┌─────────────────────────────────────────────────────────────┐
│  PAL Orchestrator (pal::invoke)                             │
│  - owns tokio::time::timeout (outer)                        │
│  - управляет fallback chain                                 │
│  - пишет run_log entry                                      │
│  - emit Tauri event при изменении health                    │
└──────┬───────────────┬───────────────┬─────────────────────┘
       │               │               │
       ▼               ▼               ▼
  ClaudeCliDriver  QwenHttpDriver  ExternalGatewayDriver
  (subprocess)    (HTTP OpenAI)   (stub, NotImplemented)
```

Каждый driver реализует `PostRuntimeProvider`. Orchestrator не знает деталей провайдера — только контракт trait.

---

## 3. Типы данных

### 3.1 ProviderRequest

```rust
use std::path::PathBuf;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// Унифицированный запрос к провайдеру.
/// Orchestrator конструирует из (Tier, post settings, task payload).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRequest {
    /// Системный prompt поста (≤130 строк по правилу проекта).
    pub system_prompt: String,

    /// Тело задачи: user-message от Диспетчера (refined prompt) или CEO.
    pub user_message: String,

    /// Tier для timeout/max_turns defaults.
    pub tier: Tier,

    /// Опциональный override timeout (если None → берётся Tier::default_timeout()).
    /// НЕ применяется внутри driver — это hint для orchestrator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Duration>,

    /// Опциональный override max_turns (если None → Tier::default_max_turns()).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,

    /// Per-request model override (DEC-002 hot-swap).
    /// Если None → driver использует свою default model (из provider_registry).
    /// Если Some("claude-opus-4-7") → driver обязан использовать именно её.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_override: Option<String>,

    /// Sandbox workspace для file-system tools.
    /// ClaudeCliDriver: Some(Outbox/<task_id>) → Command::current_dir.
    /// QwenHttpDriver / ExternalGatewayDriver: None (нет subprocess workspace).
    /// post_executor отвечает за создание директории и diff_dir сканирование.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_path: Option<PathBuf>,

    /// Имя CLI agent profile.
    /// ClaudeCliDriver: Some("mspro-office-manager") → --agent mspro-office-manager.
    /// QwenHttpDriver / ExternalGatewayDriver: None.
    /// ВАЖНО: explicit поле, НЕ выводить из trace.post_slug.
    /// trace.post_slug = origin задачи (audit), agent_slug = конкретный CLI agent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_slug: Option<String>,

    /// MCP биндинги поста (по post_mcp_bindings таблице, Phase 1+).
    /// Если provider не supports_mcp — orchestrator отбрасывает с warning без error.
    #[serde(default)]
    pub mcp_bindings: Vec<McpBinding>,

    /// Telemetry / связка с dispatcher_logs.
    pub trace: RequestTrace,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpBinding {
    pub mcp_name: String,
    pub config_ref: String, // ссылка на secret/config в OS Keychain
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestTrace {
    /// Slug поста-инициатора (audit, не путать с agent_slug).
    pub post_slug: String,

    /// FK на dispatcher_logs.id (TEXT в БД, формат task-uuid).
    /// Может быть None для прямых CEO-вызовов в обход Диспетчера.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatcher_log_id: Option<String>,

    /// Уникальный id попытки (для fallback chain нужно различать попытки).
    pub attempt_id: String,

    /// Номер попытки в fallback chain (0 = primary, 1+ = fallback).
    pub attempt_number: u8,
}
```

### 3.2 ProviderResponse

```rust
/// Унифицированный ответ от провайдера.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderResponse {
    /// Текстовый ответ модели (assistant message).
    pub text: String,

    /// Расход токенов (нормализованный учёт).
    pub usage: TokenUsage,

    /// Фактическая длительность invoke (без orchestrator overhead).
    pub latency_ms: u64,

    /// Фактически использованная модель (может отличаться от запрошенной при routing).
    pub model_used: String,

    /// Фактический провайдер (для аудита fallback chain).
    pub provider_used: ProviderKind,

    /// Причина остановки генерации.
    /// Phase 1: свободная строка ("end_turn" / "max_tokens" / "tool_use" / "stop_sequence").
    /// Phase 2: enum для нормализации (см. §10).
    pub stop_reason: String,

    /// Артефакты, агрегированные driver-ом.
    ///
    /// Phase 1 (ClaudeCli/Qwen/ExternalGateway): ВСЕГДА empty.
    /// post_executor сам сканирует Outbox/<task_id> через diff_dir.
    /// Driver не владеет sandbox-семантикой.
    ///
    /// Phase 2 (stream-json ClaudeCliDriver): может агрегировать tool_use_results
    /// в реальном времени из stream. Тогда поле станет реально полезным.
    #[serde(default)]
    pub artifacts: Vec<ArtifactRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    /// Кэшированные input (Claude prompt caching). 0 если провайдер не поддерживает.
    #[serde(default)]
    pub cache_read_tokens: u32,
    /// Записанные в кэш input. 0 если провайдер не поддерживает.
    #[serde(default)]
    pub cache_write_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub rel_path: String,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
}
```

### 3.3 ProviderError

```rust
/// Унифицированные ошибки. Driver обязан мапить native errors в эти варианты.
#[derive(Debug, thiserror::Error, Clone, Serialize, Deserialize)]
pub enum ProviderError {
    /// Превышен timeout. Сам по себе не вызывает fallback —
    /// решение принимает orchestrator (см. should_fallback).
    #[error("timeout after {timeout_secs}s")]
    Timeout { timeout_secs: u64 },

    /// Аутентификация: невалидный/просроченный токен/OAuth.
    #[error("auth failed: {0}")]
    Auth(String),

    /// Rate limit / квота провайдера исчерпана.
    #[error("quota exceeded: {0}")]
    QuotaExceeded(String),

    /// Серверная ошибка провайдера (5xx, internal).
    #[error("server error: {0}")]
    Server(String),

    /// Network / connectivity (DNS, refused, reset).
    #[error("network error: {0}")]
    Network(String),

    /// Invalid request (4xx, bad input).
    #[error("bad request: {0}")]
    BadRequest(String),

    /// Модель недоступна (deprecated, неизвестная).
    #[error("model unavailable: {0}")]
    ModelUnavailable(String),

    /// MCP startup failed / tool error.
    /// should_fallback=false (другой провайдер с той же MCP не поможет).
    #[error("mcp failure: {0}")]
    McpFailure(String),

    /// Tool-loop защита сработала.
    #[error("tool loop limit hit")]
    ToolLoopLimit,

    /// Драйвер не реализован (ExternalGatewayDriver в Phase 1).
    #[error("not implemented: {0}")]
    NotImplemented(String),

    /// Прочее (сюда не сваливать всё подряд — это последний резерв).
    #[error("other: {0}")]
    Other(String),
}

impl ProviderError {
    /// Стоит ли пытаться fallback на следующего провайдера в chain.
    /// Orchestrator принимает решение по этому методу.
    pub fn should_fallback(&self) -> bool {
        match self {
            // Сетевые/серверные/квота — другой провайдер может помочь.
            ProviderError::QuotaExceeded(_) => true,
            ProviderError::Server(_) => true,
            ProviderError::Network(_) => true,
            ProviderError::Timeout { .. } => true,
            ProviderError::Auth(_) => true,
            ProviderError::ModelUnavailable(_) => true,

            // Логические — fallback не поможет, проблема в запросе/инфре.
            ProviderError::BadRequest(_) => false,
            ProviderError::McpFailure(_) => false,
            ProviderError::ToolLoopLimit => false,
            ProviderError::NotImplemented(_) => false,
            ProviderError::Other(_) => false,
        }
    }
}
```

### 3.4 ProviderKind, Tier, HealthStatus

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProviderKind {
    ClaudeCli,
    QwenHttp,
    ExternalGateway,
    // Phase 2: OpenAI, OpenRouter, Gemini, AnthropicSDK
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tier {
    /// Opus-class: сложные посты, юрист, главы отделов.
    T1,
    /// Sonnet-class: frontend, аналитик, дизайнер.
    T2,
    /// Qwen local: рутина, копирайтер, простой ОТК.
    T3,
}

impl Tier {
    /// Default timeout per Tier. Source of truth для PAL.
    /// ВАЖНО: T1=600 соответствует settings.post_executor_timeout_sec, НЕ claude_cli_timeout_sec.
    /// См. §7 «Timeout reconciliation».
    pub fn default_timeout(self) -> Duration {
        match self {
            Tier::T1 => Duration::from_secs(600), // 10 мин — для долгих document-задач
            Tier::T2 => Duration::from_secs(360), // 6 мин — для средних задач
            Tier::T3 => Duration::from_secs(60),  // 1 мин — локальная модель, быстро
        }
    }

    pub fn default_max_turns(self) -> u32 {
        match self {
            Tier::T1 => 80,
            Tier::T2 => 40,
            Tier::T3 => 20,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HealthStatus {
    /// Провайдер отвечает, токены не исчерпаны, auth валидна.
    Alive,
    /// Отвечает, но квота близка/исчерпана (warning).
    QuotaExceeded,
    /// Auth просрочена/невалидна.
    AuthFailed,
    /// Network / connection refused.
    Unreachable,
    /// 5xx от провайдера.
    ServerError,
    /// Health не проверялся (свежий driver, до первого invoke / health_check).
    Unknown,
}
```

### 3.5 Capabilities

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Capabilities {
    /// Поддержка tool_use / function calling.
    pub supports_tools: bool,
    /// Поддержка MCP биндингов.
    pub supports_mcp: bool,
    /// Поддержка streaming (Phase 2+).
    pub supports_streaming: bool,
    /// Поддержка prompt caching (Anthropic).
    pub supports_prompt_caching: bool,
    /// Максимальный контекст (input tokens).
    pub max_context_tokens: u32,
    /// Максимальный output (для одного response).
    pub max_output_tokens: u32,
    /// Vision / image input.
    pub supports_vision: bool,
}
```

---

## 4. Trait PostRuntimeProvider

```rust
use async_trait::async_trait;

#[async_trait]
pub trait PostRuntimeProvider: Send + Sync {
    // ─── Identity ────────────────────────────────────────────────

    /// Тип провайдера (для маршрутизации, аудита, UI).
    fn provider_kind(&self) -> ProviderKind;

    /// Уникальный id экземпляра провайдера в provider_registry.
    /// Один ProviderKind может иметь несколько экземпляров
    /// (например, два разных Claude CLI с разными OAuth-профилями).
    fn provider_id(&self) -> String;

    /// Capabilities — что провайдер физически умеет.
    fn capabilities(&self) -> Capabilities;

    /// Стоимость токенов для нормализованного учёта.
    /// Tuple: (input_cost_per_1k_usd, output_cost_per_1k_usd).
    /// Для local моделей (Qwen) обычно (0.0, 0.0).
    fn cost_per_1k_tokens(&self) -> (f64, f64);

    // ─── Operations ──────────────────────────────────────────────

    /// Основной вызов модели.
    ///
    /// **ВАЖНО**: driver НЕ применяет outer timeout (request.timeout — это hint).
    /// Orchestrator оборачивает invoke() в tokio::time::timeout (§6).
    /// Driver может использовать internal polling timeouts (например 5s tick на read_line)
    /// как защиту от тихого зависания, но не как замену outer timeout.
    ///
    /// При обнаружении future cancellation (drop) driver обязан корректно завершить
    /// все subprocess/connection ресурсы (см. §6).
    async fn invoke(&self, request: ProviderRequest) -> Result<ProviderResponse, ProviderError>;

    /// Lightweight проверка доступности.
    /// Не должна расходовать токены (для платных провайдеров).
    /// Реализация:
    /// - ClaudeCliDriver: `claude --version` + проверка наличия валидной сессии.
    /// - QwenHttpDriver: `GET /api/tags` на Ollama endpoint.
    /// - ExternalGatewayDriver: WS ping (Phase 2).
    ///
    /// Время выполнения должно быть ≤2 сек.
    async fn health_check(&self) -> HealthStatus;
}
```

---

## 5. Tier presets (default values)

| Tier | Назначение         | Provider (default) | Model (default)     | timeout | max_turns |
|------|--------------------|--------------------|---------------------|---------|-----------|
| T1   | Opus-class         | anthropic_cli      | claude-opus-4-7     | 600s    | 80        |
| T2   | Sonnet-class       | anthropic_cli      | claude-sonnet-4-6   | 360s    | 40        |
| T3   | Qwen local         | qwen_local         | qwen3:14b           | 60s     | 20        |

Конкретные default-модели хранятся в `provider_registry` (Итерация B миграции 08-09). `Tier::default_timeout()` и `Tier::default_max_turns()` — hardcoded константы в коде PAL (см. §3.4).

**Изменение default-моделей** — через provider_registry в БД, без правки кода (DEC-002).
**Изменение Tier::default_timeout()** — через PR + миграция (это контрактная константа).

---

## 6. Timeout policy (orchestrator vs driver)

### 6.1 Кто владеет outer timeout

Орчестратор (PAL wrapper) — **единственный** владелец outer timeout:

```rust
// Псевдокод PAL orchestrator
pub async fn pal_invoke(
    provider: &dyn PostRuntimeProvider,
    request: ProviderRequest,
) -> Result<ProviderResponse, ProviderError> {
    let timeout = request.timeout.unwrap_or_else(|| request.tier.default_timeout());
    let hard_cap = Duration::from_secs(600);
    let effective = std::cmp::min(timeout, hard_cap);

    match tokio::time::timeout(effective, provider.invoke(request.clone())).await {
        Ok(Ok(response)) => Ok(response),
        Ok(Err(provider_err)) => Err(provider_err),
        Err(_elapsed) => Err(ProviderError::Timeout {
            timeout_secs: effective.as_secs(),
        }),
    }
}
```

### 6.2 Что делает driver

Driver **не оборачивает** свой код в `tokio::time::timeout`. Driver:

- Получает request, выполняет subprocess/HTTP вызов.
- Может использовать **internal polling timeouts** для подопераций (например, read_line с 5s tick — защита от тихого зависания stdout).
- При timeout от orchestrator (через future cancellation) driver обязан **корректно завершить ресурсы**:
  - subprocess через `kill_on_drop(true)` + явный `child.kill().await`;
  - HTTP клиент через drop (reqwest сам отменит in-flight request);
  - WebSocket через close frame.

### 6.3 Cancellation & Drop safety

Tokio future cancellation происходит когда `tokio::time::timeout` истёк. В этот момент `provider.invoke()` future будет dropped без завершения. Driver обязан:

1. Хранить subprocess/connection handles в `self` или в captured variables future.
2. На drop — освобождать ресурсы (Rust Drop trait отрабатывает автоматически).
3. Для subprocess: `Command::kill_on_drop(true)` — критично, иначе orphan процессы (грабля проекта — 12 зависших claude.exe, наблюдалась).

### 6.4 Hard cap

PAL hard cap = **600 сек** = `settings.post_executor_timeout_sec` = `Tier::T1::default_timeout()`. 

Даже если request.timeout = 9999s, orchestrator применит `min(request.timeout, 600s)`. Это защита от ошибочно высоких значений в БД и согласованность с outer kill timeout процесса post_executor.

---

## 7. Timeout reconciliation (v3 — переписана)

### 7.1 Три разных таймаута в проекте — три разных runtime канала

Программист сделал code investigation v1.0.33 — в проекте сейчас **три независимых таймаута** для трёх разных каналов:

| Источник                                  | Значение | Где используется                       | Что измеряет                            |
|-------------------------------------------|----------|----------------------------------------|-----------------------------------------|
| `settings.claude_cli_timeout_sec`         | 360 сек  | `claude_bridge.rs:434` (CEO/Гендир)    | CEO-вызов Claude через `--print`        |
| `settings.dispatcher_routing_timeout_sec` | 180 сек  | `claude_bridge.rs:310` (router brain)  | Диспетчер refining raw → refined prompt |
| `settings.post_executor_timeout_sec`      | 600 сек  | `post_executor.rs:264` (пост-агенты)   | Полный run post-агента в Outbox sandbox |

**Семантика разная:**

- **CEO/Гендир** — быстрые tool_call решения, тексты для UI, без heavy file operations. 360s достаточно с запасом.
- **Dispatcher router** — короткий routing brain (refining 1-2 sec обычно). 180s — sanity cap.
- **Post executor** — реальный агент пишет .docx/.xlsx, сканирует Vault, запускает MCP. Документы занимают 5-8 минут реальной работы. 600s — обязательный минимум.

### 7.2 Какой таймаут источник для PAL

PAL обслуживает **post-агенты в Phase 1** (см. §8). Соответственно:

- `Tier::T1::default_timeout = 600s` = `settings.post_executor_timeout_sec`.
- PAL hard cap = 600s = `post_executor_timeout_sec`.
- `settings.claude_cli_timeout_sec=360` для PAL **не применяется** — это таймаут CEO/claude_bridge.rs, отдельного канала.

### 7.3 Что было неверно в v2

В v2 §7 содержал таблицу, где `claude_cli_timeout_sec=360` был связан с `Tier::T1` и post_executor. **Это была ошибка по памяти** — реально post_executor использует `post_executor_timeout_sec=600`, а `claude_cli_timeout_sec` обслуживает другой код-путь (CEO).

Если бы trait был принят в v2 — `Tier::T1=360s` вызвал бы регрессию: написание .docx/.xlsx занимает 5-8 минут, 360s было бы недостаточно. Post-агенты падали бы по timeout на легитимных задачах. Зафиксировано как кейс «план следует за работающим кодом».

### 7.4 Phase 2 migration plan

После миграции CEO на PAL (Phase 2):

- `claude_cli_timeout_sec` станет legacy alias для CEO Tier (отдельный CEO Tier, скорее всего «T0» или встроенный default).
- `post_executor_timeout_sec` останется как outer subprocess kill timeout (внешний по отношению к PAL).
- Все три таймаута будут читаться из БД через `provider_registry` / `post_runtime`, не из глобальных settings.

До Phase 2 settings остаются нетронутыми — это interim state.

---

## 8. Phase 1 MVP — scope интеграции

### 8.1 Что PAL заменяет в Phase 1

- **post_executor.rs** — прямой `spawn claude.exe` заменяется на `pal.invoke(request)`. Outbox sandbox семантика остаётся (workspace_path в request, post_executor сам делает diff_dir).
- **Run Logger** — единая запись `run_logs` через PAL после каждого invoke (нормализованные tokens/cost/provider/model/fallback_used).
- **Tier resolution** — interim mapping через `posts.preferred_model` → Tier (до миграции 08 с полноценным `post_runtime`).
- **Lightweight Health Monitor** — `health_check()` всех зарегистрированных провайдеров по расписанию (5 мин active + lazy перед invoke при ошибке primary).

### 8.2 Что PAL НЕ трогает в Phase 1

- **CEO brain** (`claude_bridge.rs`) — остаётся как есть. CEO продолжает использовать `claude_cli_timeout_sec=360` напрямую. Миграция CEO на PAL — Phase 2 (когда придёт время mainstream-усиления CEO под другие модели).
- **Гендир tool_calls** — отдельный путь, не PAL (это управление UI/БД, не модельные вызовы).
- **dispatcher_brain routing** — отдельный канал refining (claude_bridge.rs:310). Может мигрировать на PAL в Phase 1.5 или Phase 2, но не блокер Phase 1.
- **MCP startup** — остаётся внешним к PAL (управляется post_executor / Tauri-уровнем).
- **Vault tools** (write/patch/delete_vault_file) — уровень Гендира, не PAL.

### 8.3 Phase 1 integration path (7 шагов)

1. **Code investigation** ✅ выполнен (Vault/03-Phases/phase-1-current-db-schema.sql + ClaudeCliDriver реальный flow от программиста).
2. **Trait + types** ✅ этот SPEC v3.
3. **ClaudeCliDriver implementation reference** — ведёт программист по реальному коду v1.0.33.
4. **QwenHttpDriver** — следующий концептуальный draft (моя зона, после v3 одобрения).
5. **ExternalGatewayDriver contract-stub** — все методы `NotImplemented`, регистрируется в `provider_registry` для UI demo.
6. **PAL orchestrator** — `pal::invoke()` с outer timeout + fallback chain + run_log запись (программист).
7. **post_executor integration** — feature flag `settings.pal_enabled` (default false), параллельный rollout, метрики, потом switch.

### 8.4 CEO migration — Phase 2 plan (out of scope Phase 1)

CEO/Гендир brain сейчас использует свой собственный путь к Claude (`claude_bridge.rs`). Перевод на PAL — отдельная задача:

- Создать CEO-Tier (T0 или встроенный) с timeout 360s = `claude_cli_timeout_sec`.
- Перевести `claude_bridge::invoke_ceo` на `pal.invoke()`.
- Сохранить совместимость с UI плашками (⚡ success / ⚠️ warning / ❌ error).

До Phase 2 CEO остаётся как work in progress — не трогать в Phase 1.

---

## 9. Открытые вопросы (для Phase 1 implementation / Phase 2)

1. **MCP config механизм для Claude CLI** — глобальный `~/.claude/mcp.json` vs per-post флаг. Если глобальный — per-post MCP биндинги (DEC-002 #4) технически невозможны без хака. Программист поднимет на implementation.
2. **claude --agent stdin schema** — точный формат payload (plain text vs JSON). Зафиксирует программист в driver reference.
3. **stream-json driver** — Phase 2 кандидат. Позволит реальный artifacts aggregation + token usage online.
4. **OAuth refresh** — DEC-001 явно вынесен в Phase 2. До тех пор Claude CLI session manual.
5. **Tool-loop heuristic** — определение «identical» tool_call: `sha1(canonical_json(args))`. Орчестратор отслеживает 5 одинаковых подряд → ToolLoopLimit error. Может быть реализовано в Phase 1 или отложено если не встретим в боях.

---

## 10. Phase 2+ deferred

- **stop_reason enum** — нормализация {EndTurn, MaxTokens, ToolUse, StopSequence, Cancelled, Error} вместо свободной строки. Потребуется для multi-turn агентов и автоматической tool-loop детекции.
- **streaming API** — `stream(request) → Stream<Chunk>` метод в trait. Для real-time UI и tool_use_results aggregation.
- **dynamic cost** — `cost_per_1k_tokens()` может вернуть разные значения для cache_read vs regular input. Сейчас одна tuple на провайдера, упрощение.
- **Prompt caching detailed accounting** — TokenUsage уже содержит cache_read/cache_write поля, но cost модель пока их не учитывает.
- **Capabilities runtime detection** — сейчас статика. В будущем driver может query provider API за актуальными capabilities (модель может «потерять» supports_vision при rate limit).

---

## 11. Naming alignment

- `ProviderKind::ExternalGateway` — используется в SPEC.
- `ExternalAgentProvider` — упоминание в DEC-001 (старая формулировка).
- При следующем обновлении `Vault/decisions-log.md` синхронизировать на `ExternalGateway` для консистентности.
- `HealthStatus::QuotaExceeded` vs `ProviderError::QuotaExceeded` — оставлены оба, разная семантика:
  - `HealthStatus` — наблюдение при health-check (не во время invoke).
  - `ProviderError` — фактический отказ во время invoke.
  - UI должен отображать по-разному (warning vs error плашка) — backlog для wireframes.

---

## 12. Acceptance criteria (DEC-001 mapping)

| DEC-001 criterion                                      | Покрывается в trait                              |
|--------------------------------------------------------|--------------------------------------------------|
| Регистрация API-провайдера ≤10 мин                     | provider_id + provider_kind + provider_registry  |
| Health провайдеров ≤30 сек                             | health_check() + orchestrator scheduler          |
| Auto-fallback при сбое primary                         | ProviderError::should_fallback() + run_logs      |
| Смена модели Pod ≤5 мин                                | model_override в ProviderRequest                 |
| PAL unit-тесты: mock driver, fallback, error mapping   | Trait абстрактный → тривиально мокается          |

---

## 13. Roles & responsibilities

- **Гендир (этот SPEC)** — trait контракт, высокоуровневая архитектура (UI wireframes, DoD, risks, sequencing) — что будет в следующих разделах Phase 1 SPEC.
- **Программист** — driver implementation references по реальному коду v1.0.33 (ClaudeCliDriver, далее QwenHttpDriver implementation, ExternalGatewayDriver stub), PAL orchestrator имплементация, post_executor integration.
- **Cursor** — code investigation, schema dumps, review SPEC, review PR.
- **Владелец** — DEC решения, approve каждой итерации SPEC, бюджеты, scope decisions.

Это разделение зафиксировано после v2 review — детальные driver SPEC от меня создавали риск расхождения с реальным кодом. Контракт-уровень (trait, типы) — моя зона; реализация — программиста.

---

## 14. Source documents

- `Vault/decisions-log.md` — DEC-001, DEC-002, DEC-003
- `Vault/03-Phases/phase-1-current-db-schema.sql` — фактическая схема v1.0.33
- `Vault/03-Phases/phase-0-detailed-plan.md` — Phase 0 baseline
- `Vault/02-Patterns/документация-следует-за-работающим-кодом-а-не-наоборот.md` — принцип, применённый при v3 пересмотре
- `Vault/02-Patterns/rebuild-msi-playbook-v1.0.33.md` — release workflow используемый между фазами



---

# v3.1 Addendum (2026-05-28)

Четыре точечные naming/clarification-правки после Cursor review v3. Тело v3 не переписывается — все правки ниже **переопределяют** соответствующие места v3 и являются source of truth для имплементации.

## Changelog v3 → v3.1

- ПРАВКА 1: зафиксирована конвенция `agent_slug` (без префикса `mspro-`).
- ПРАВКА 2: явно указано где живёт `task_id` для связи с `dispatcher_logs`.
- ПРАВКА 3: решение по `raw_debug` (вынесен в БД, не в trait).
- ПРАВКА 4: footnote к §7 timeout reconciliation про live vs defaults.

Nice-to-have правка 5 (синхронизация error имён в IMPL-REFERENCE) — зона программиста, в trait не входит.

---

## ПРАВКА 1 — Конвенция `ProviderRequest.agent_slug`

**Переопределяет:** комментарий и пример к полю `agent_slug` в `ProviderRequest` (§3.1).

**Конвенция (single source of truth):**

> `agent_slug` содержит **sanitized slug поста БЕЗ префикса** (например `"office-manager"`, `"hco-head"`).
> Префикс `mspro-` формируется **внутри `ClaudeCliDriver`** при построении CLI-аргумента `--agent mspro-{agent_slug}`.
> Эта конвенция согласована с `ensure_post_agent_md` в реальном коде v1.0.33.

**Корректный пример:**

```rust
ProviderRequest {
    agent_slug: Some("office-manager".to_string()),  // БЕЗ mspro-
    // ...
}
```

**ClaudeCliDriver** при формировании команды:

```rust
let cli_agent_name = format!("mspro-{}", request.agent_slug.as_deref().unwrap_or("default"));
cmd.arg("--agent").arg(&cli_agent_name);
```

**Запрет:** передавать в `agent_slug` строку с уже добавленным префиксом (`"mspro-office-manager"`). Это будет считаться багом вызывающей стороны — driver НЕ нормализует префикс автоматически (иначе скрываем источник ошибки).

**Запрет для других драйверов:** `QwenHttpDriver` и `ExternalGatewayDriver` должны игнорировать `agent_slug` (поле `Option`, для них `None` — нормальное состояние). Если передан `Some(...)` — driver его молча игнорирует, не возвращает ошибку (защита от forward-compatibility ломок).

---

## ПРАВКА 2 — Где живёт `task_id` в `RequestTrace`

**Уточняет:** §3.1 `RequestTrace`. В v3 поля `task_id` / `correlation_id` были заменены на `attempt_id` + `attempt_number`. Cursor правильно поднял вопрос: где теперь связь с `dispatcher_logs`?

**Решение (single source of truth):**

> `task_id` (формат `task-<uuid>`, TEXT-PK в `dispatcher_logs` v1.0.33) хранится в `RequestTrace.dispatcher_log_id: Option<String>`.
> `attempt_id` — это **внутренний идентификатор попытки PAL invoke** (оди�� task_id может породить N attempts при fallback chain), НЕ связан с `dispatcher_logs.id`.

**Семантика:**

| Поле | Тип | Источник | Назначение |
|---|---|---|---|
| `dispatcher_log_id` | `Option<String>` | `dispatcher_logs.id` (v1.0.33 TEXT) | Связь PAL → dispatcher история |
| `attempt_id` | `String` (UUID) | сгенерирован PAL orchestrator | Уникальная попытка invoke внутри fallback chain |
| `attempt_number` | `u32` | счётчик попыток в chain | 1 = primary, 2+ = fallback |
| `post_slug` | `String` | вызывающий пост | Origin задачи (НЕ путать с `agent_slug`) |

**Запись в `run_logs`:** обе колонки — `task_id` (= `dispatcher_log_id`) и `attempt_id` — пишутся в отдельные поля для возможности `JOIN dispatcher_logs ON task_id` и при этом группировки попыток по `attempt_id`.

**Запрет:** не использовать `attempt_id` как FK к `dispatcher_logs`. Это разные сущности.

---

## ПРАВКА 3 — `raw_debug` решение

**Уточняет:** §3.2 `ProviderResponse`. В v2 было поле `raw_debug: Option<String>`, в v3 убрано без объяснения.

**Решение (single source of truth):**

> Raw debug (полный stdout subprocess, raw HTTP response body, JSONL stream) **не возвращается через trait**. Он пишется напрямую в БД `run_logs.raw_output` (TEXT, nullable, лимит 64 KB с truncation maker `...[truncated N bytes]`).
>
> Driver получает `&RunLogger` (или эквивалент) и сам решает что и сколько писать. Trait остаётся чистым data-контрактом.

**Обоснование:**

1. **Размер.** Raw debug может быть мегабайты (полный JSONL stream). Тянуть это через `ProviderResponse` означает удержание в RAM до завершения вызова. БД-запись по chunk-у дешевле.
2. **Безопасность.** Raw debug может содержать sensitive data (API keys в stderr, content пользовательских файлов). БД-слой проще обмазать redaction и audit, чем trait-возврат.
3. **Async write.** Driver может писать raw debug по мере поступления (streaming), не дожидаясь финального `ProviderResponse`. Trait-возврат был бы блокирующим.

**Импликация для драйверов:**

- `ClaudeCliDriver` пишет `run_logs.raw_output` = последние ≤64 KB stdout + stderr (с truncation marker если больше).
- `QwenHttpDriver` пишет `raw_output` = HTTP response body (или его часть до 64 KB).
- `ExternalGatewayDriver` (stub) — пишет `raw_output = "NotImplemented"`.

**Запрет:** добавлять `raw_debug` в `ProviderResponse` обратно без отдельного DEC решения. Если в Phase 2 stream-json driver потребует raw chunks в-memory — это новое DEC, не возврат старой архитектуры.

---

## ПРАВКА 4 — Footnote к §7 Timeout reconciliation

**Уточняет:** заголовок таблицы §7.1.

**Текст footnote (добавить под таблицу §7.1):**

> **Footnote ⁽¹⁾ к таблице §7.1:**
> Значения в колонке «Live (v1.0.33)» отражают **фактический deployed `settings.json`** Владельца на момент 2026-05-28 (после rebuild 1.0.33 с поднятым `claude_cli_timeout_sec` 180→360).
>
> **Defaults в коде** (`crates/core/src/settings/mod.rs`) — другие:
> - `claude_cli_timeout_sec` default = 180 (legacy)
> - `dispatcher_routing_timeout_sec` default = 60
> - `post_executor_timeout_sec` default = 600
>
> **Расхождение между live settings.json и code defaults — нормально**: пользователь переопределяет defaults под свои условия. Source of truth для PAL — **Tier presets** (§3.4), **не** значения из `settings`.
>
> Детальная сверка defaults vs live — в `IMPL-REFERENCE §5.1` (зона программиста).
>
> **Phase 2 migration:** после миграции CEO на PAL, `settings.claude_cli_timeout_sec` помечается deprecated и становится legacy alias на `Tier::T1.default_timeout()`. До этого момента — два независимых таймаута (CEO timeout vs PAL Tier presets).

---

## Сводка по версии

- **v1** — initial draft (5 nice-to-have правок).
- **v2** — 5 fixes Cursor (RequestTrace.dispatcher_log_id, ProviderConfig removed, Timeout serde, Timeout ownership, §7 reconciliation table).
- **v3** — 6 правок программиста (workspace_path / agent_slug / model_override в ProviderRequest, Tier T1=600s, artifacts=empty в Phase 1, §7 переписан, §13 roles).
- **v3.1** (этот addendum) — 4 naming/clarification после Cursor review v3 (конвенция agent_slug, task_id↔attempt_id, raw_debug в БД, §7 footnote).

**Status:** v3.1 — ✅ ready for driver implementation. Следующая инкрементальная версия — только при появлении новых требований от программиста или Cursor по результатам имплементации.
