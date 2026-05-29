# Phase 1 — ExternalGatewayDriver IMPLEMENTATION REFERENCE (⚠ STUB)

- **Version:** v1 (2026-05-28)
- **Author:** Claude Code (skill `mspro-programmer`)
- **Status:** Approved Владельцем — источник истины для реализации `pal/external_gateway_driver.rs` (Phase 1 = **stub**, реализация Phase 2).
- **⚠ STUB:** в отличие от `ClaudeCliDriver` и `QwenHttpDriver`, **реального кода драйвера НЕТ**. Это контракт-заглушка: все методы возвращают `NotImplemented` / `Unknown`. Провайдер зарегистрирован в `provider_registry` (виден в UI Service Bureau), но не исполняет задачи в Phase 1.
- **Relation to other docs:**
  - Контракт trait — `phase-1-pal-trait-spec.md` v3 (`ProviderKind::ExternalGateway`, `ProviderError::NotImplemented`, `HealthStatus::Unknown`).
  - Сестринские driver-reference — `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1, `phase-1-qwen-http-driver-IMPL-REFERENCE.md` v1.1 (паттерн структуры; этот короче — stub).
  - Реальный код для Phase 2 reuse — `src-tauri/src/external_agent/{mod,gateway,handlers,auth}.rs` (WS gateway 8899).

**Назначение документа:** зафиксировать честную stub-имплементацию `PostRuntimeProvider` для `ProviderKind::ExternalGateway` в Phase 1 + контракт на Phase 2 (опираясь на **реально существующий** WS gateway `external_agent/`, не на выдумку).

---

## §1. Назначение и scope

### Что это
`ExternalGatewayDriver` — задел на провайдеров, доступных **через шлюз**, а не локально:
- удалённый Claude / GPT / любой облачный LLM через прокси-gateway;
- внешний AI-сервис компании (например, специализированный отраслевой ассистент);
- **внешний агент, подключённый к MSPro по WebSocket** (наиболее вероятный первый кейс — см. §5).

### Phase 1 scope — ТОЛЬКО stub
- Провайдер **зарегистрирован** в `provider_registry` (DEC-001 acceptance «3 провайдера по умолчанию»: `claude_cli`, `qwen_http`, `external_gateway`).
- Виден в UI Service Bureau (Tab «Провайдеры») как карточка.
- `invoke()` возвращает `ProviderError::NotImplemented(...)`.
- `health_check()` возвращает `HealthStatus::Unknown`.
- Реализация исполнения — **Phase 2 R&D**.

### Зачем регистрировать stub в Phase 1
1. **DEC-001 acceptance** — UI Service Bureau показывает N провайдеров; external_gateway демонстрирует, что система vendor-agnostic и расширяемая.
2. **Контракт зафиксирован** — `ProviderKind::ExternalGateway` уже в enum trait v3 §3.4; stub материализует его в реальный (пусть и пустой) driver-инстанс.
3. **UI расширяемость** — Владелец видит «сюда можно подключить внешний шлюз», понятен roadmap.
4. **Forward-compat тест** — orchestrator + registry + UI должны корректно обрабатывать провайдера, который ничего не умеет (защита от падений на «пустом» провайдере).

---

## §2. Реализация trait v3 как stub (6 методов)

```rust
use async_trait::async_trait;
use crate::pal::{
    PostRuntimeProvider, ProviderRequest, ProviderResponse, ProviderError,
    ProviderKind, Capabilities, HealthStatus,
};

/// Stub-драйвер для ProviderKind::ExternalGateway.
/// Phase 1: ничего не исполняет. Phase 2: WS gateway 8899 (см. §5).
pub struct ExternalGatewayDriver {
    id: String,  // "external_gateway" из provider_registry row
}

impl ExternalGatewayDriver {
    pub fn new(id: String) -> Self { Self { id } }
}

#[async_trait]
impl PostRuntimeProvider for ExternalGatewayDriver {
    fn provider_kind(&self) -> ProviderKind {
        ProviderKind::ExternalGateway
    }

    fn provider_id(&self) -> String {
        self.id.clone()  // "external_gateway"
    }

    fn capabilities(&self) -> Capabilities {
        // Всё false/0 — честный stub «ничего не умеет пока».
        // Когда реальный провайдер появится (Phase 2) — capabilities
        // станут зависеть от конкретного backend за шлюзом.
        Capabilities {
            supports_tools: false,
            supports_mcp: false,
            supports_streaming: false,
            supports_prompt_caching: false,
            max_context_tokens: 0,    // unknown до выбора реального провайдера
            max_output_tokens: 0,
            supports_vision: false,
        }
    }

    fn cost_per_1k_tokens(&self) -> (f64, f64) {
        (0.0, 0.0)  // unknown — зависит от backend за шлюзом (Phase 2)
    }

    async fn invoke(&self, _request: ProviderRequest)
        -> Result<ProviderResponse, ProviderError>
    {
        // Возвращаем ДО чтения каких-либо полей request (§4).
        Err(ProviderError::NotImplemented(
            "ExternalGateway driver — Phase 2 R&D (reuse external_agent WS gateway 8899)".to_string()
        ))
    }

    async fn health_check(&self) -> HealthStatus {
        // Phase 1: всегда Unknown — нет endpoint для проверки.
        // Phase 2: WS ping к ws://127.0.0.1:8899 (trait v3 §4 / §430).
        HealthStatus::Unknown
    }
}
```

**Замечания:**
- Driver **полностью stateless** и **immutable** — только `id` для `provider_id()`.
- `invoke()` мгновенный (нет IO) — возвращает `NotImplemented` без сети/subprocess.
- `health_check()` мгновенный — `Unknown` без ping (в Phase 1 нет реального endpoint).

---

## §3. Поведение в UI Service Bureau (wireframes v1.1)

Sync с `phase-1-ui-wireframes-spec.md` v1.1 UX-6 («external_gateway Test button — disabled серая»).

| Элемент UI | Phase 1 поведение |
|---|---|
| ProviderCard в Tab «Провайдеры» | **Видна** (registry row exists) |
| ProviderHealthBadge | `Unknown` — серый, UI label «неизвестно» (маппинг trait v3 HealthStatus) |
| Подпись карточки | «Stub (NotImplemented в Phase 1)» / endpoint `ws://127.0.0.1:8899` |
| `[Test connection]` | **DISABLED** (серая кнопка) — UX-6; нечего проверять в Phase 1 |
| `[Edit]` / `[Delete]` | Разрешены (registry CRUD работает), но смысла мало в Phase 1 |
| TierBadge | не назначается (external_gateway не привязан к Tier по умолчанию) |

UI **не должен** падать или показывать ошибку при рендере stub-провайдера — он отображается как «зарегистрирован, но не активен».

---

## §4. ProviderRequest — все поля игнорируются (no-op)

`invoke()` возвращает `NotImplemented` **до чтения** любых полей `ProviderRequest`:

| Поле | stub поведение |
|---|---|
| `system_prompt` | не читается |
| `user_message` | не читается |
| `tier` | не читается |
| `timeout` | не читается |
| `max_turns` | не читается |
| `model_override` | не читается |
| `workspace_path: Option<PathBuf>` | игнорируется (даже `Some(_)` — без ошибки) |
| `agent_slug: Option<String>` | игнорируется (даже `Some(_)` — без ошибки; trait v3 §700 forward-compat) |
| `mcp_bindings: Vec<McpBinding>` | игнорируется (без warning — stub вообще ничего не делает) |
| `trace: RequestTrace` | прокидывается orchestrator-ом в `run_logs` (post_slug, task_id) для аудита провалившейся попытки |

---

## §5. Phase 2 контракт (РЕАЛЬНЫЙ reuse path — не выдумка)

### ⚠ Главное различие inbound vs outbound (чтобы не путать)

В проекте **уже существует** `src-tauri/src/external_agent/` (WS gateway, порт 8899) — но это **другое направление**:

| | `external_agent/` (СУЩЕСТВУЕТ v1.0.33) | ExternalGatewayDriver (PAL, Phase 2) |
|---|---|---|
| Направление | **Inbound** control-plane | **Outbound** provider |
| Кто инициатор | Внешний агент подключается К MSPro | MSPro вызывает внешнего исполнителя |
| Сценарий | `brain_mode=claude_external`: MSPro broadcast-ит ceo-question по WS, внешний агент отвечает `ceo/respond` (внешний агент = мозг CEO) | пост-задача исполняется внешним агентом: `invoke(request)` → WS → reply |
| Код | `gateway.rs`, `handlers.rs`, `auth.rs`, `GatewayState`, `PendingResponses` | НЕ существует (Phase 2) |

**Ключ:** ExternalGatewayDriver в Phase 2 = **формализация того же сокета** для исполнения пост-задач, **переиспользуя** инфраструктуру `external_agent/`.

### Phase 2 implementation path (опираясь на реальный `external_agent/`)

Существующий механизм `brain_mode=claude_external` (verified в `external_agent/mod.rs`):
- `GatewayState { cancel_tx, current_port, events: broadcast::Sender<String> }`.
- `PendingResponses` — `oneshot` map keyed by message id.
- `send_chat_message` регистрирует `oneshot::Sender` → broadcast вопрос по `events` → await reply; RPC handler `ceo/respond` резолвит sender.

ExternalGatewayDriver Phase 2 повторит этот паттерн для пост-задач:
1. Driver получает `Arc<GatewayState>` (broadcast events + доступ к PendingResponses).
2. `invoke(request)` сериализует request в JSON-RPC: `{ "method": "pod/invoke", "params": {system_prompt, user_message, ...}, "id": <uuid> }`.
3. broadcast по `GatewayState.events` (как существующий ceo-question).
4. регистрирует `oneshot` в PendingResponses по `id`, await reply.
5. **новый** RPC handler `pod/respond` в `handlers.rs` (аналог существующего `ceo/respond`) резолвит oneshot → `ProviderResponse`.
6. timeout — orchestrator через `tokio::time::timeout` (trait v3 §6), как у всех driver; при drop — oneshot отменяется.

### Параметры Phase 2 (зафиксированы, реализуются позже)
- **Протокол:** JSON-RPC over WebSocket (как существующий gateway).
- **Порт:** 8899 (`GatewayState.current_port`).
- **Auth:** через существующий `external_agent/auth.rs` (не изобретать новый).
- **Инфраструктура:** reuse `gateway.rs` — **новый сокет не нужен**.
- **Capabilities Phase 2:** зависят от backend за шлюзом (если за gateway удалённый Claude — supports_mcp/tools/vision = true; если простой echo-агент — false). Определяются handshake-ом при подключении внешнего агента.

---

## §6. Согласованность с trait v3 + важный flag

### NotImplemented → should_fallback = false (КРИТИЧНО)
`ProviderError::NotImplemented` имеет `should_fallback() == false` (trait v3 §3.3, line 297 — «логические ошибки: fallback не поможет, проблема в запросе/инфре»).

**Следствие:** если пост назначить `ExternalGateway` как **primary** провайдер + Claude как fallback — задача **упадёт сразу с NotImplemented, БЕЗ перехода на fallback**. Это by-design (NotImplemented = config-ошибка, не transient сбой).

### Рекомендация Phase 1 (UI prevention) ⚠
**Service Bureau показывает карточку `external_gateway` (registry demo), но dropdown назначения `post_runtime.primary_provider_id` / `fallback_chain` ИСКЛЮЧАЕТ `external_gateway` до Phase 2.**

Обоснование: если разрешить выбор stub-провайдера как primary — пост будет fail-fast на каждой задаче с NotImplemented (т.к. fallback не сработает). Чище **не дать** назначить его, чем ловить провалы.

Альтернатива (если нужна гибкость): разрешить выбор, но UI показывает inline warning «⚠ ExternalGateway — заглушка Phase 1; задачи упадут с NotImplemented». **Рекомендую исключение из dropdown** (предотвращение лучше предупреждения).

### Честный stub в run_logs (trait v3 §748)
При попытке invoke (если кто-то всё же назначил):
- `provider_used = "external_gateway"`.
- `success = false`.
- `error_kind = "not_implemented"`.
- `raw_output = "NotImplemented"` (trait v3 §748).
- `tokens_in = tokens_out = 0`, `cost_usd = 0`.

### НЕ переопределять should_fallback ради stub
Соблазн: сделать `NotImplemented.should_fallback() = true` чтобы stub-primary мягко падал на fallback. **НЕ делать** — это испортит контракт для всех других логических ошибок. Решать config-проблему на UI-уровне (§6 prevention), не врать про fallbackability в trait.

---

## §7. Verification (минимум — это stub)

Запускать ПОСЛЕ команды на имплементацию + `cargo build` без ошибок.

**Unit-тесты:**
1. **`invoke` → NotImplemented:** mock любой `ProviderRequest` (включая `Some(workspace_path)`, `Some(agent_slug)`, непустые `mcp_bindings`) → `Err(ProviderError::NotImplemented(msg))`; `assert!(msg.contains("Phase 2"))`. Без panic, без чтения полей.
2. **`health_check` → Unknown:** возвращает `HealthStatus::Unknown` мгновенно (без сети) — `assert_eq!(driver.health_check().await, HealthStatus::Unknown)`.
3. **`capabilities` all-false:** все 5 bool == false, `max_context_tokens == 0`, `max_output_tokens == 0`.
4. **`should_fallback` = false:** `ProviderError::NotImplemented("x".into()).should_fallback() == false` — защита от случайного fallback на stub.
5. **Integration registry:** seed migration 08 регистрирует `external_gateway` row → `list_providers` (Tauri command) возвращает его с `status='enabled'`, последний `health = Unknown` → UI ProviderCard видна, `[Test connection]` DISABLED.

**НЕ нужно:** integration-тестов с реальным WS / mock-gateway (нет реализации в Phase 1). Они появятся в Phase 2 verification вместе с `pod/invoke` RPC.

---

## §8. Связанные документы

- **trait контракт** — `phase-1-pal-trait-spec.md` v3 (`ProviderKind::ExternalGateway` §3.4, `ProviderError::NotImplemented` §3.3 с `should_fallback=false`, `HealthStatus::Unknown` §3.4, диаграмма §2 `MVP=stub`).
- **Сестринские driver-reference** — `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1, `phase-1-qwen-http-driver-IMPL-REFERENCE.md` v1.1 (паттерн структуры; этот короче — stub).
- **DoD** — `phase-1-definition-of-done.md` v1.1 (AC-002.4 «ExternalGatewayDriver stub возвращает NotImplemented без panic»; AC-001.3 «3 провайдера зарегистрированы»).
- **Risk register** — `phase-1-risk-register.md` v1.1 (R-T-009 fallback chain — stub-primary fail-fast связан с этим).
- **Wireframes** — `phase-1-ui-wireframes-spec.md` v1.1 (UX-6 Test connection DISABLED для external_gateway).
- **Реальный код для Phase 2 reuse** — `src-tauri/src/external_agent/{mod,gateway,handlers,auth,sql_validator}.rs` (WS gateway 8899, `GatewayState`, `PendingResponses`, `ceo/respond` RPC паттерн). **НЕ путать:** существующий = inbound control-plane; ExternalGatewayDriver = outbound provider (Phase 2 формализация того же сокета).

---

## §9. Changelog

- **v1 (2026-05-28):** первая редакция — stub reference. Опирается на trait v3 (NotImplemented / ProviderKind::ExternalGateway / HealthStatus::Unknown / Capabilities all-false) + read реального `external_agent/mod.rs` (WS 8899 inbound control-plane) для честного Phase 2 reuse path. 9 разделов (компактнее real-драйверов). Flag: NotImplemented should_fallback=false → рекомендация исключить external_gateway из post_runtime dropdown Phase 1. Это последний driver-reference Итерации A Phase 1 SPEC.

*End of ExternalGatewayDriver IMPL REFERENCE (stub).*
