# Phase 1 — Definition of Done

**Версия:** v1.1  
**Создан:** 2026-05-28 (v1.0 Гендир)  
**Обновлён:** 2026-05-28 (v1.1 — программист, sync с реальным кодом v1.0.33 + Cursor правки)  
**Авторы:** Гендир (каркас 43 критерия, scope, Sign-off); программист (технический inventory: пути, cargo, argv, Tauri commands, deliverables, sequencing)  
**Статус:** Active checklist для закрытия Phase 1  
**Источники:** Vault/decisions-log.md (DEC-001/002/003/004), phase-1-pal-trait-spec.md v3, phase-1-ui-wireframes-spec.md v1.1, phase-1-claude-cli-driver-IMPL-REFERENCE.md v1.1, phase-1-current-db-schema.sql (v1.0.33)

## Changelog
- **v1.1 (2026-05-28)** — программист, sync с реальным кодом v1.0.33 + 9 правок Cursor + 1 trait v3 sync. Применены:
  1. Пути в Block 5: `crates/core/src/` → `src-tauri/src/`, `crates/core/migrations/` → `src-tauri/migrations/`, `crates/ui/src/` → `src/`.
  2. Cargo команды: `--package core` → `--lib`; `--workspace` → `--lib` (single crate `mspro-ltd-corp`).
  3. AC-001.3 разделил три понятия: registry status / HealthStatus / `ProviderError::NotImplemented`.
  4. Open Q JSONL → text stdout (по IMPL-REFERENCE §7.2; stream-json — Phase 2 backlog).
  5. RuntimeQuickBadge → `DepartmentCardRuntimeTooltip` (UX-2 wireframes v1.1).
  6. Block 5 дополнен подсекциями **Tauri commands** (5 модулей) + **Event emission** (provider_health_changed).
  7. Новый раздел «Phase 1 scope vs DEC-002/004 границы» (DEC-001/003 полностью, DEC-002 частично, DEC-004 → Phase 4).
  8. AC-002.6 Vault Scaffolder сужен до минимального `00-INDEX.md` (полный scaffolder → Phase 1.5 backlog).
  9. Новый раздел «Implementation sequencing» (6 этапов перед Sign-off).
  10. AC-002.2 — полный argv ClaudeCli; AC-002.3 + AC-002.8 — sync с trait v3 `ProviderError` names (`Auth`/`Server`/`Network`/`QuotaExceeded`/`BadRequest`, не HealthStatus-имена).
- **v1.0 (2026-05-28)** — Гендир, 43 проверяемых критерия в 5 блоках (Acceptance / Functional / Regression / Quality / Deliverables) + Sign-off процедура + Open questions.

## Принципы

- Каждый критерий **проверяемый** (есть конкретный тест/команда/наблюдение).
- Формулировка «X делает Y за Z секунд» вместо «работает хорошо».
- Phase 1 считается **CLOSED** только когда все критерии Блоков 1–4 ☑ + все Deliverables Блока 5 присутствуют.
- При расхождении DoD vs реальность работающего кода — применяется паттерн «документация следует за работающим кодом» (Vault/02-Patterns/), DoD корректируется через addendum.

---

## Phase 1 scope vs DEC-002 / DEC-004 границы

**Phase 1 ЗАКРЫВАЕТ:**
- ✅ **DEC-001** — Service Bureau + PAL **ПОЛНОСТЬЮ** (PAL trait + 3 драйвера + UI Service Bureau + secrets через DPAPI).
- ✅ **DEC-003** — Model/tier switching **ПОЛНОСТЬЮ** (UI EditPostKnowledgeModal Runtime секция + `post_runtime` таблица + hot-swap без рестарта).
- 🟡 **DEC-002** — Pod Template foundation **ЧАСТИЧНО**:
  - ✅ PAL слой (фундамент Pod Runtime).
  - ✅ 3 драйвера (ClaudeCli, QwenHttp, ExternalGateway stub).
  - ✅ `run_logs` + Run Logger (нормализованный учёт токенов/cost/fallback).
  - ❌ Полный Pod Template manifest + lifecycle + версии → **Phase 3**.

**Phase 1 НЕ ЗАКРЫВАЕТ (перенесено в Phase 2-4):**
- Регистрация второго поста ≤30 минут — **Phase 4 Validation**.
- office-manager закрывает деловую задачу ≤1ч — **Phase 4 Validation**.
- MCP Loader / Task Queue / Debug Console / Heartbeat — **Phase 2**.
- Полный Pod Template manifest + lifecycle + версии — **Phase 3**.
- DEC-004 терминология (Service Bureau / Pod / Template naming alignment в `decisions-log.md`) — **Phase 4 sweep**.

---

## Блок 1 — Acceptance Criteria по DEC

### DEC-001 — Service Bureau + PAL

- [ ] **AC-001.1** Регистрация нового API-провайдера ≤10 минут через UI Service Bureau Tab `Providers`.  
  Сценарий проверки: Владелец открывает Service Bureau → жмёт `+ Добавить провайдер` → заполняет 5 полей (kind, id, endpoint, model_default, secret_ref) → Test connection → Save → новый провайдер виден в списке с health badge.  
  Замер: секундомер от клика `+` до появления карточки. Цель: ≤ 600 секунд.

- [ ] **AC-001.2** Health-стату�� провайдера обновляется в UI ≤30 секунд от события (provider лёг / поднялся).  
  Сценарий проверки: останавливаем Ollama (`taskkill /F /IM ollama.exe`) → UI badge `qwen_http` меняется на `Unreachable` за ≤30 сек. Запускаем обратно → возвращается на `Alive` за ≤30 сек.  
  Механизм: Tauri event `provider_health_changed` + active poll каждые 5 мин + lazy re-check при ошибке.

- [ ] **AC-001.3** Три провайдера зарегистрированы по умолчанию (seed миграции 08):  
  - `claude_cli` (ClaudeCli kind, endpoint=path к claude.exe, default_model=claude-opus-4-7).  
  - `qwen_http` (QwenHttp kind, endpoint=http://localhost:11434, default_model=qwen3:14b).  
  - `external_gateway` (ExternalGateway kind, **stub-зарегистрирован** в `provider_registry` со статусом registry=`enabled`; `health_check()` возвращает `HealthStatus::Unknown` — провайдер не проверяется; при вызове `invoke()` driver возвращает `ProviderError::NotImplemented("external_gateway driver — Phase 2 R&D")`. UI: Test connection кнопка **disabled** — wireframes v1.1 UX-6).  

  **Разделение понятий (важно — три разных типа):**
  - `provider_registry.status` (`enabled` / `disabled`) — registry состояние, контролирует включён ли провайдер в роутинг.
  - `HealthStatus` (`Alive` / `QuotaExceeded` / `AuthFailed` / `Unreachable` / `ServerError` / `Unknown`) — наблюдаемое состояние при `health_check()`.
  - `ProviderError::NotImplemented(String)` — runtime ошибка при попытке `invoke()` нереализованного драйвера.

  Проверка: `SELECT id, kind, status FROM provider_registry;` возвращает 3 строки; для `external_gateway` `health_check()` → `Unknown`, `invoke(...)` → `Err(ProviderError::NotImplemented(...))`.

- [ ] **AC-001.4** Auto-fallback при сбое primary провайдера срабатывает автоматически.  
  Сценарий проверки: задача office-manager с tier=T1 (primary=claude_cli, fallback=qwen_http). Убиваем claude.exe в момент задачи → orchestrator ловит ProviderError, делает fallback на qwen_http, задача завершается успешно.  
  Проверка: `SELECT fallback_used, provider_used FROM run_logs WHERE id=<last>;` → `fallback_used=true`, `provider_used='qwen_http'`.

- [ ] **AC-001.5** PAL unit-тесты зелёные. Минимум:  
  - MockDriver implements PostRuntimeProvider trait  
  - Fallback chain: 3 провайдера, первый возвращает ServerError → второй Alive → задача выполнена  
  - Error mapping: subprocess exit codes → правильные ProviderError варианты  
  - Timeout wrapper: tokio::time::timeout срабатывает на hard cap=600s  
  Команда: `cargo test --lib pal::tests` (из `src-tauri/`; пакет `mspro-ltd-corp`, single crate — нет `--package core`). Цель: ≥15 тестов pass, 0 fail.

- [ ] **AC-001.6** Секреты хранятся в OS Keychain (Windows DPAPI). БД содержит только `secret_ref`.  
  Проверка: `SELECT secret_ref FROM provider_registry;` — никаких API-ключей в plain. Тест keyring: `keychain::get('claude_cli_oauth')` возвращает значение, BackuP app.db не содержит секретов (grep `sk-`).

### DEC-002 — Pod Template foundation

- [ ] **AC-002.1** PAL trait `PostRuntimeProvider` v3 реализован в `src-tauri/src/pal/mod.rs`.  
  Проверка: trait содержит 2 обязательных async-метода (`invoke`, `health_check`) + 4 sync (`provider_kind`, `provider_id`, `capabilities`, `cost_per_1k_tokens`). Совпадает с `phase-1-pal-trait-spec.md` v3 §4.

- [ ] **AC-002.2** `ClaudeCliDriver` реализован по `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1 в `src-tauri/src/pal/claude_cli_driver.rs`.  
  **Полный argv (IMPL-REFERENCE §1.1):** `claude.exe --print --output-format text --agent mspro-{safe_slug} --model {m} --dangerously-skip-permissions`.  
  **Окружение:** `current_dir = workspace_path` (Outbox sandbox; не `--workspace` флаг), `env(MSPRO_TASK_ID = task_id)`, `stdin = piped` (plain text `refined_prompt`), `kill_on_drop(true)`, `hide_console(&mut cmd)` (Windows `CREATE_NO_WINDOW`).  
  **Контракт:** агент сам создаёт файлы через native Write tool в cwd; filesystem scan артефактов через `diff_dir` делает caller (`post_executor`), не driver. `ProviderResponse.artifacts = vec![]` всегда; `usage = (0,0,0,0)` (`--output-format text` не содержит структурированных tokens — gap для cost-tracking, fix в Phase 2 через `stream-json`). См. IMPL-REFERENCE §3.1 / §7.2.

- [ ] **AC-002.3** `QwenHttpDriver` реализован в `src-tauri/src/pal/qwen_http_driver.rs`.  
  Проверка: OpenAI-compatible HTTP POST к `<endpoint>/v1/chat/completions`, `supports_mcp=false`, парсинг JSON response (`choices[0].message.content` + `usage.{prompt_tokens, completion_tokens}`).  
  **Error mapping (sync с trait v3 §3.3 — `ProviderError` варианты, НЕ `HealthStatus`):**
  - network IO / DNS / connection refused → `ProviderError::Network(reason)`.
  - HTTP 401 → `ProviderError::Auth(reason)`.
  - HTTP 5xx → `ProviderError::Server(reason)`.
  - HTTP 429 → `ProviderError::QuotaExceeded(reason)`.
  - HTTP 400 / unparseable body → `ProviderError::BadRequest(reason)`.
  - "model not found" / unknown model → `ProviderError::ModelUnavailable(reason)`.

- [ ] **AC-002.4** `ExternalGatewayDriver` stub в `src-tauri/src/pal/external_gateway_driver.rs`. `invoke()` возвращает `Err(ProviderError::NotImplemented("external_gateway driver — Phase 2 R&D"))`; `health_check()` возвращает `HealthStatus::Unknown`.  
  Проверка: оба вызова без panic; зарегистрирован в `provider_registry` со статусом registry=`enabled` (см. AC-001.3).

- [ ] **AC-002.5** System Prompt Store с вал��датором ≤130 строк.  
  Проверка: `UPDATE posts SET system_prompt_md = '<131 строка>'` через UI Edit Knowledge → ошибка валидации, save заблокирован.

- [ ] **AC-002.6** Vault Scaffolder MVP (Phase 1 сужение): при `create_post(slug, ...)` создаётся **только минимальный** `Vault/posts/<slug>/00-INDEX.md` (заголовок «Post: {slug}», описание поста, пустые разделы `## Knowledge` / `## Patterns` / `## Bugs`).  
  Подкаталоги `05-Patterns/`, `06-Bugs/` и `_README.md` стабы **НЕ создаются автоматически в Phase 1** — Владелец/пост создаёт по мере необходимости через `write_vault_file`.  
  Проверка: `create_post('test-post-001', ...)` → существует ровно один файл `Vault/posts/test-post-001/00-INDEX.md`.  
  **Полноценный Vault Scaffolder** (template tree, README-стабы, DEC-связь, knowledge migration) → **Phase 1.5 backlog** (отдельный тикет, не блокер Phase 1 CLOSED).

- [ ] **AC-002.7** Run Logger пишет в БД `run_logs` для каждого вызова PAL.  
  Поля минимум: id, post_slug, task_id (FK dispatcher_logs), provider_id, provider_used, model_used, fallback_used, attempt_number, tokens_in, tokens_out, latency_ms, cost_usd, success, error_kind, raw_output (truncated ≤64KB), created_at.

- [ ] **AC-002.8** Health Monitor MVP: 5× same error → escalate в очередь Гендира.  
  Сценарий: провайдер X выдаёт 5 раз подряд `ProviderError::Auth(_)` (или другой тип) → создаётся задача в `dispatcher_logs` с `post_slug='ceo'`, `priority='high'`, `payload='Health alert: provider X — Auth x5'`. **Имена variants — по trait v3 §3.3** (`Auth` / `Server` / `Network` / `QuotaExceeded` / `BadRequest`, НЕ `AuthFailed` / `ServerError` / `Unreachable` — последние три это `HealthStatus`).

### DEC-003 — Model/tier switching

- [ ] **AC-003.1** Смена модели поста ≤5 минут через UI EditPostKnowledgeModal → Runtime секция.  
  Сценарий: Владелец открывает post → Edit Knowledge → прокручивает до Runtime секции → меняет provider+model через dropdown → Save → закрывает modal.  
  Замер: ≤ 300 секунд от открытия до Save.

- [ ] **AC-003.2** Hot-swap без рестарта MSPro приложения. Следующий run поста использует новую модель.  
  Сценарий: меняем модель → НЕ перезапускаем MSPro → даём задачу посту → `run_logs.model_used` = новое значение.

- [ ] **AC-003.3** Tier presets T1/T2/T3 редактируются через Service Bureau Tab `Tier Presets`.  
  Сценарий: открыть TierPresetCard T2 → Edit → изменить timeout с 360 на 400 → Save → проверка: `SELECT default_timeout_sec FROM tier_presets WHERE tier='T2'` = 400.

- [ ] **AC-003.4** Изменение в UI → запись в `post_runtime` → следующий run читает свежий профиль.  
  Проверка: `UPDATE post_runtime SET primary_provider_id='qwen_http' WHERE post_slug='test-post'` → следующий run этого поста идёт через qwen_http.

- [ ] **AC-003.5** `post_runtime.model_override` per-request переопределяет default модель провайдера.  
  Проверка: в ProviderRequest указано `model_override=Some("claude-sonnet-4-6")` — даже если default=claude-opus-4-7, вызов идёт на sonnet.

- [ ] **AC-003.6** `run_logs` содержит фактические provider_id + model_used (не tier).  
  Проверка: SQL `SELECT DISTINCT provider_used, model_used FROM run_logs LIMIT 10` — конкретные строки типа `qwen_http / qwen3:14b`, не `T3`.

---

## Блок 2 — Функциональные проверки (E2E)

- [ ] **F-001** post_executor вызывает `pal.invoke()` при `settings.pal_enabled=true`.  
  Проверка: grep `pal.invoke` в post_executor.rs, feature flag читается из settings.

- [ ] **F-002** ClaudeCliDriver реально выполняет smoke-задачу office-manager → `result.txt` в Outbox.  
  Сценарий: повторяем Phase 0 Day 3 smoke, но через PAL. Артефакт создан, run_logs.success=true, run_logs.provider_used='claude_cli'.

- [ ] **F-003** QwenHttpDriver выполняет задачу через Ollama qwen3:14b.  
  Сценарий: создать тестовый пост `qwen-test-pod` с tier=T3, primary=qwen_http. Дать задачу «верни одно слово: ok». Артефакт создан, model_used='qwen3:14b'.

- [ ] **F-004** Fallback chain срабатывает end-to-end.  
  Сценарий: pod с chain `[claude_cli, qwen_http]`. Убиваем claude.exe → задача завершается через qwen_http. run_logs: 2 attempt-а (attempt 1 fail на claude_cli, attempt 2 success на qwen_http).

- [ ] **F-005** Service Bureau UI Flow «Добавить провайдер → health → tier».  
  Сценарий: добавить fake qwen-secondary (qwen_http на другом порту 11435) → запустить второй Ollama → Test connection OK → Save → health badge Alive за ≤30 сек → переключить tier T3 default model на новый провайдер.

- [ ] **F-006** Pod Runtime UI Flow «Смена модели → новый run на новой модели».  
  Сценарий: office-manager текущая модель=claude-opus-4-7 → Edit Knowledge → Runtime секция → сменить на claude-sonnet-4-6 → Save → дать задачу → run_logs.model_used='claude-sonnet-4-6'.

- [ ] **F-007** Test connection в AddProviderModal blocking (Save disabled до Test OK).  
  Сценарий: открыть AddProviderModal → заполнить поля с заведомо неверным endpoint → жмём Save → кнопка disabled. Запускаем Test → fail → Save остаётся disabled. Исправляем endpoint → Test OK → Save активен.

- [ ] **F-008** DepartmentCard hover tooltip показывает текущий runtime поста.  
  Сценарий: навести мышь на карточку office-manager → за ≤500 мс появляется tooltip с `provider_id / model / tier`.

---

## Блок 3 — Регрессионные проверки

- [ ] **R-001** Существующие 8 отделов и посты работают как раньше. Документы .docx/.xlsx создаются.  
  Сценарий: дать office-manager задачу на letterhead DOCX → артефакт появляется в Outbox/<task_id>/.

- [ ] **R-002** Гендир CEO chat работает (legacy claude_bridge, НЕ PAL — Phase 2 migration).  
  Сценарий: открыть CEO chat → отправить сообщение → ответ от Claude без регрессии.

- [ ] **R-003** Диспетчер routing работает (dispatcher_brain.rs НЕ PAL в Phase 1).  
  Сценарий: send_to_dispatcher → refined hop появляется в dispatcher_logs.

- [ ] **R-004** Killswitch `pal_enabled=false` возвращает к старому post_executor поведению.  
  Сценарий: установить settings.pal_enabled=false → задача office-manager → выполняется через legacy `spawn claude.exe` без PAL участия.

- [ ] **R-005** `vault_ops_log` из TICKET-001 (Phase 0) пишется корректно для write/patch/delete_vault_file.  
  Проверка: `SELECT COUNT(*) FROM vault_ops_log WHERE created_at > '<phase-1-start>'` > 0.

- [ ] **R-006** Все cargo tests Phase 0 baseline (102 теста) проходят зелёными.  
  Команда: `cargo test --lib` (из `src-tauri/`; single crate `mspro-ltd-corp`) — 0 failures, новые тесты PAL добавлены к baseline.

- [ ] **R-007** SecurityVault → SecretsPanel({embedded:true}) встроен в Service Bureau без двойного header.  
  Визуальная проверка: открыть Service Bureau → Tab Secrets → один header `🔐 Отдел СБ`, таблица секретов на месте, никакого второго H1 внутри Tab.

---

## Блок 4 — Качественные критерии

- [ ] **Q-001** `cargo test --lib` проходит (из `src-tauri/`). Минимум +30 unit + 3 integration тестов на PAL (поверх Phase 0 baseline 102 теста).

- [ ] **Q-002** Нет orphan `claude.exe` процессов после задач (грабля 11B-bis — kill_on_drop работает).  
  Сценарий: прогнать 10 задач последовательно через ClaudeCliDriver → `tasklist | findstr claude` после возвращает 0 процессов (или только 1 если CEO chat активен).

- [ ] **Q-003** Миграции 08 и 09 применяются чисто на свежей и существующей БД. Self-healing блок в `lib.rs::setup()` для новых таблиц (грабля 08-tribal).  
  Сценарий: rollback миграции 08 → application starts → CREATE TABLE IF NOT EXISTS triggers → таблицы появляются.

- [ ] **Q-004** Backup `app.db` сделан ДО применения миграций (rebuild MSI playbook).  
  Файлы: `Vault/03-Phases/pre-agentpod-phase-1.db` (708KB+) и `post-agentpod-phase-1.db` (после миграций).

- [ ] **Q-005** Timeout policy enforced. Hard cap 600s соблюдается. Tier presets работают как source of truth.  
  Тест: создать MockDriver который sleep 700s → orchestrator killит на 600s, возвращает Timeout.

- [ ] **Q-006** `cargo clippy --all-targets -- -D warnings` без warnings (из `src-tauri/`; с возможным allow-list под inline CSSProperties structs если потребуется).

- [ ] **Q-007** `phase-1-current-db-schema.sql` обновлён после миграций (sync). Cursor делает `sqlite3 app.db .schema > phase-1-current-db-schema.sql` в конце фазы.

- [ ] **Q-008** lf() wrapper в migrations 08/09 — CRLF/checksum грабля снята архитектурно (паттерн rebuild-msi-playbook-v1.0.33).

---

## Блок 5 — Deliverables

### Код Rust (src-tauri/src/)

- [ ] `pal/mod.rs` — trait `PostRuntimeProvider` v3 + типы (`ProviderRequest`, `ProviderResponse`, `ProviderError`, `HealthStatus`, `Capabilities`, `Tier`, `RequestTrace`).
- [ ] `pal/orchestrator.rs` — fallback chain, outer `tokio::time::timeout` wrapper, attempt loop, Run Logger вызов.
- [ ] `pal/claude_cli_driver.rs` — `ClaudeCliDriver` по IMPL-REFERENCE v1.1 (argv §1.1, sandbox через `current_dir`, plain text stdin, `kill_on_drop`, `hide_console`).
- [ ] `pal/qwen_http_driver.rs` — `QwenHttpDriver` (OpenAI-compatible HTTP к Ollama; error mapping по trait v3 §3.3).
- [ ] `pal/external_gateway_driver.rs` — stub возвращает `ProviderError::NotImplemented(...)` (Phase 2 R&D).
- [ ] `pal/health_monitor.rs` — active poll 5 мин + lazy re-check при ошибке primary + escalation в очередь Гендира (AC-002.8).
- [ ] `pal/tests/mod.rs` — `MockDriver` + unit tests (≥15 тестов; fallback chain, error mapping, timeout drop).
- [ ] `commands/post_executor.rs` — модификация (см. IMPL-REFERENCE §2): feature flag `settings.pal_enabled`, вызов `pal.invoke()` вместо `spawn claude.exe`; сохраняется `pre_snapshot` + `diff_dir` + `register_artifact` (Net 140→90 строк, 35% reduction).
- [ ] `run_logger.rs` (корень `src-tauri/src/`) — модуль записи в `run_logs` (атомарная вставка, `raw_output` truncation ≤64KB).
- [ ] `keychain.rs` (корень `src-tauri/src/`) — Windows DPAPI wrapper через `keyring` crate v3 (уже в Cargo.toml: `target.'cfg(windows)'.dependencies.keyring`).
- [ ] `settings/mod.rs` — добавлены поля `pal_enabled: bool` (default `false` для killswitch R-004), legacy `claude_cli_timeout_sec` помечен deprecated (используется только CEO/`claude_bridge.rs`, не PAL).

### Tauri commands (src-tauri/src/commands/)

- [ ] `commands/providers.rs` — Tauri commands: `list_providers`, `upsert_provider`, `delete_provider`, `test_provider_connection`, `register_provider` (CRUD + connection probe для AddProviderModal).
- [ ] `commands/post_runtime.rs` — Tauri commands: `upsert_post_runtime(slug, profile)`, `get_post_runtime(slug)`, `list_post_runtimes` (hot-swap для DEC-003).
- [ ] `commands/tier_presets.rs` — Tauri commands: `list_tier_presets`, `upsert_tier_preset` (Service Bureau Tab `Tier Presets`).
- [ ] `commands/health_monitor.rs` — Tauri command: `health_check_provider(provider_id)` manual trigger; background poll worker через `tauri::async_runtime::spawn` на startup (lib.rs::setup).
- [ ] `commands/run_logs.rs` — Tauri commands: `list_run_logs(filters)`, `get_run_log(id)` (UI debug / cost dashboard).
- [ ] Регистрация всех новых commands в `lib.rs::invoke_handler!`.

### Event emission (Tauri)

- [ ] `app.emit("provider_health_changed", payload)` в `pal/health_monitor.rs` worker при смене `HealthStatus` провайдера. payload: `{ provider_id: String, new_status: HealthStatus, checked_at_unix_ms: u64 }`.
- [ ] React `useEffect` subscription на event в `ProviderHealthBadge` (live update UI ≤30 сек — AC-001.2).

### Миграции SQLite (src-tauri/migrations/)

- [ ] `08_provider_registry_post_runtime.sql` — таблицы:  
  - `provider_registry` (id, kind, endpoint, default_model, secret_ref, status, created_at, updated_at)  
  - `tier_presets` (tier TEXT PRIMARY KEY, default_timeout_sec, max_turns, default_model)  
  - `post_runtime` (post_slug PRIMARY KEY, primary_provider_id, primary_model, fallback_chain_json, tier, model_override, updated_at)  
  - Seed: 3 провайдера + 3 tier presets (T1=600s, T2=360s, T3=60s)

- [ ] `09_run_logs_health.sql` — таблицы:  
  - `run_logs` (полная схема согласно AC-002.7)  
  - `provider_health_log` (provider_id, status, observed_at, error_kind, latency_ms)  
  - Self-healing CREATE TABLE IF NOT EXISTS в lib.rs::setup() для обеих

### React UI (src/)

**Service Bureau (рефакторинг существующего SecurityVault):**
- [ ] `src/components/views/ServiceBureau.tsx` — рефакторинг текущего `views/SecurityVault.tsx` в страницу с Tab bar (Провайдеры / Tier Presets / Секреты) по паттерну `views/Dispatcher.tsx`. Sidebar label «🔐 Отдел СБ» **остаётся** (UX-1 wireframes v1.1); `App.tsx` view route менять **не нужно**.
- [ ] `src/components/serviceBureau/Providers.tsx` — список провайдеров с `ProviderCard`.
- [ ] `src/components/serviceBureau/TierPresets.tsx` — 3 `TierPresetCard` + `EditTierPresetModal`.
- [ ] `src/components/serviceBureau/SecretsPanel.tsx` — refactored `SecurityVault` контент с `{ embedded?: boolean }` prop (R-007).

**Runtime visualization (generic компоненты):**
- [ ] `src/components/runtime/ProviderHealthBadge.tsx` — generic в стиле `chat/BrainStatusBadges.tsx` (inline CSS, точка + label), для N провайдеров.
- [ ] `src/components/runtime/TierBadge.tsx` — pill в стиле `dispatcher/TaskRow.tsx::STATUS_STYLE`.
- [ ] `src/components/runtime/FallbackChainList.tsx` — `ArrowUp`/`ArrowDown` иконки lucide-react (НЕ drag-and-drop — UX-4).
- [ ] `src/components/runtime/RuntimeEffectPreview.tsx` — превью эффекта смены модели в EditPostKnowledgeModal Runtime секции.

**Modals:**
- [ ] `src/components/modals/AddProviderModal.tsx` — форма + Test connection **blocking** (Save disabled до OK — UX-5).
- [ ] `src/components/modals/EditProviderModal.tsx` — Edit existing provider.
- [ ] `src/components/modals/EditTierPresetModal.tsx` — редактирование T1/T2/T3 пресета.

**Существующие компоненты (модификация):**
- [ ] `src/components/home/EditPostKnowledgeModal.tsx` — добавлена Runtime секция **после `<hr>`** (НЕ tab — табов в modal нет; v1.1 правка #1 wireframes).
- [ ] `src/components/home/DepartmentCard.tsx` — hover tooltip через `DepartmentCardRuntimeTooltip` (UX-2: visible по hover, не always).
- [ ] `src/components/home/DepartmentCardRuntimeTooltip.tsx` — tooltip с runtime info (`provider_id / model / tier`); fetch на hover, render через portal или inline absolute. Цель: ≤500 мс до показа (F-008).
- [ ] `src/components/Sidebar.tsx` — label «🔐 Отдел СБ» **остаётся**; bump версии вручную на 1.0.34 (4-й файл версии — `sync-version.mjs` не обновляет автоматически).

### Документация Vault

- [ ] `Vault/03-Phases/phase-1-detailed-plan.md` — sequencing 7-10 дней с зависимостями
- [ ] `Vault/03-Phases/phase-1-risk-register.md` — ≥5 рисков с mitigation
- [ ] `Vault/03-Phases/phase-1-testing-strategy.md` — unit/integration/e2e + MockDriver план
- [ ] `Vault/03-Phases/phase-1-completion-report.md` — заполняется в конце фазы (что сделано, что отложено, lessons learned)
- [ ] `Vault/03-Phases/phase-1-current-db-schema.sql` — обновлён после миграций (post-phase-1)
- [ ] Обновление `Vault/decisions-log.md` если DEC корректировались по итогам Phase 1 (через addendum)

### Release artifacts

- [ ] `git tag agentpod-phase-1-start` (создан в Day 1)
- [ ] `git tag agentpod-phase-1-complete` (создан в момент закрытия DoD)
- [ ] `backup pre-agentpod-phase-1.db` (≥708KB, до миграций)
- [ ] `backup post-agentpod-phase-1.db` (после миграций)
- [ ] MSI rebuild v1.0.34 по rebuild-msi-playbook-v1.0.33.md (с pre-flight gate, watcher pattern, MSI до подписи)
- [ ] `Sidebar.tsx` version bump на 1.0.34 (4-й файл версии, sync-version.mjs не обновляет автоматически)
- [ ] `cargo check` для синхронизации Cargo.lock перед commit
- [ ] FileVersion installed exe = 1.0.34 (S6 verify rebuild playbook)

---

## Implementation sequencing (порядок зависимостей — 6 этапов)

Этапы упорядочены по зависимостям. Каждый последующий опирается на предыдущий; параллелить можно внутри этапа, не между.

**Этап 1 — Pre-flight + миграции БД**
- Backup `app.db` ×2 по `02-Patterns/rebuild-msi-playbook-v1.0.33.md`: `pre-agentpod-phase-1.db` ДО старта + `post-agentpod-phase-1.db` ПОСЛЕ миграций.
- Миграции `08_provider_registry_post_runtime.sql` + `09_run_logs_health.sql` (`src-tauri/migrations/`).
- Self-healing блок `CREATE TABLE IF NOT EXISTS` + raw `ALTER` через `PRAGMA table_info` в `lib.rs::setup()` для **всех 5 новых таблиц** (`provider_registry`, `tier_presets`, `post_runtime`, `run_logs`, `provider_health_log`) — грабля 08-tribal (Phase 11C-D).
- `lf()` wrapper (live с v1.0.32) для новых `.sql` работает автоматически.
- Pre-flight gate перед миграциями: `cargo test --lib` + `pnpm tsc --noEmit`.

**Этап 2 — PAL trait + драйверы + orchestrator**
- `src-tauri/src/pal/mod.rs` — trait + типы по `phase-1-pal-trait-spec.md` v3.
- `pal/claude_cli_driver.rs` по IMPL-REFERENCE v1.1 (argv §1.1, sandbox через `current_dir`, plain text stdin, MCP игнорируется с warning).
- `pal/qwen_http_driver.rs` (OpenAI-compatible HTTP к Ollama `qwen3:14b`).
- `pal/external_gateway_driver.rs` stub (NotImplemented).
- `pal/orchestrator.rs` — outer `tokio::time::timeout(effective, driver.invoke(...))` + fallback chain (по `ProviderError::should_fallback()`) + Run Logger вызов.
- Unit-тесты `MockDriver` + fallback + error mapping + timeout-drop (`kill_on_drop` verify): `cargo test --lib pal::tests` ≥15 green.

**Этап 3 — post_executor integration + Run Logger + pal_enabled flag**
- Модификация `commands/post_executor.rs` по IMPL-REFERENCE §2 (Net 140→90 строк, 35% reduction): spawn/wait/stderr_capture уходят в driver; pre_snapshot+diff_dir+register_artifact+bump_attempts/fail_task/emit остаются.
- `src-tauri/src/run_logger.rs` — атомарная запись `run_logs` (raw_output truncation ≤64KB).
- `settings/mod.rs` — поле `pal_enabled: bool = false` (killswitch R-004; включаем `true` ТОЛЬКО после Этапа 6 smoke).
- Health monitor background worker — `tauri::async_runtime::spawn` в `lib.rs::setup` (5 мин active poll + Tauri event emit).

**Этап 4 — Tauri commands для UI CRUD**
- `commands/{providers, post_runtime, tier_presets, health_monitor, run_logs}.rs` (см. Block 5 / Tauri commands).
- Регистрация в `lib.rs::invoke_handler!`.
- Сначала backend контракт — потом React. Без работающего CRUD не строим UI.

**Этап 5 — React UI: Service Bureau + Pod Runtime modal + DepartmentCard tooltip**
- Рефакторинг `views/SecurityVault.tsx` → `views/ServiceBureau.tsx` (Tab bar: Провайдеры / Tier Presets / Секреты).
- `serviceBureau/SecretsPanel.tsx` через `{ embedded: true }` (R-007 — нет двойного header).
- `ProviderCard`, `TierPresetCard`, `ProviderHealthBadge`, `TierBadge`, `FallbackChainList` (ArrowUp/Down — UX-4).
- `AddProviderModal` с Test connection **blocking** (UX-5).
- `home/EditPostKnowledgeModal.tsx` — Runtime секция после `<hr>`.
- `home/DepartmentCard.tsx` + `home/DepartmentCardRuntimeTooltip.tsx` — hover tooltip (UX-2).
- `Sidebar.tsx` — bump версии 1.0.34 руками (label «🔐 Отдел СБ» **не** менять — UX-1).

**Этап 6 — E2E + MSI rebuild 1.0.34 + sign-off**
- F-001…F-008 smoke (8 функциональных сценариев Block 2).
- R-001…R-007 regression suite Block 3.
- Q-001…Q-008 quality gates Block 4.
- MSI rebuild по `02-Patterns/rebuild-msi-playbook-v1.0.33.md` (pre-flight `cargo test --lib`+`tsc` gate, watcher pattern, MSI берётся ДО signing-зависания + kill signer, S3 close MSPro + S6 verify FileVersion installed exe = 1.0.34).
- `git tag agentpod-phase-1-complete`.
- Cursor independent review всех 7 категорий Deliverables Block 5.
- Включить `settings.pal_enabled = true` (после успеха smoke).

---

## Sign-off процедура (Phase 1 CLOSED)

Фаза считается завершённой когда:

1. Все ☐ Блоков 1-4 переведены в ☑ с краткой пометкой (commit hash / test run id / SQL query result).
2. Все Deliverables Блока 5 присутствуют (Test-Path / git tag verify / SQL verify).
3. Cursor делает independent review всех 7 категорий Deliverables против реального состояния репо.
4. Владелец делает final approve в UI Awaiting tab (как в Phase 0 Day 4).
5. `save_win` с target_post=ceo: «Phase 1 CLOSED, AgentPod MVP работает на 2 провайдерах + stub, hot-swap UI, fallback chain proven».
6. Создаётся `phase-1-completion-report.md` с lessons learned (вход в Phase 2 prep).

## Open questions / risks (будут детализированы в risk-register)

- Совместимость PAL с существующим MCP startup (per-post MCP config механизм — Claude CLI принимает MCP через `~/.claude/mcp.json` глобально, не per-agent). Потенциальный блокер supports_mcp=true для разных постов с разным набором MCP.
- Orphan claude.exe после Tauri-window crash (даже kill_on_drop не помогает если упало само приложение). Watchdog?
- Backup app.db перед каждой миграцией требует прогрева WAL — может занимать 5-10 сек на больших БД, UI блокируется.
- stdout от Claude CLI в Phase 1 — **plain text** (`--output-format text`), парсинг не требуется: артефакты обнаруживаются через `diff_dir` в `post_executor`. `usage_tokens = (0, 0, 0, 0)` для ClaudeCli Phase 1 — это **известный cost-tracking gap** (см. IMPL-REFERENCE §7.2). Fix в Phase 2 через миграцию на `--output-format stream-json` (даст точный `usage` из Claude API) или post-hoc `anthropic-tokenizer` для estimate. JSONL parsing — НЕ нужен в Phase 1.
- Phase 2 migration CEO claude_bridge → PAL: непрозрачные dependencies в текущем коде, нужен отдельный investigation Cursor перед началом Phase 2.

---

*DoD будет дополняться по результатам investigation Cursor и review Владельца. Изменения — через addendum (v1.1, v1.2, ...) с changelog, не overwrite.*
