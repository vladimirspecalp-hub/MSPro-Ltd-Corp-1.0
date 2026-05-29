# Phase 1 — Risk Register

- **Версия:** v1.1
- **Дата:** 2026-05-28
- **Автор технической части:** программист (Claude Code, skill `mspro-programmer`)
- **Автор бизнес/процессной части:** Гендир (CEO, §5); cross-links R-T — Cursor (v1.1)
- **Scope:** Phase 1 (PAL + 3 драйвера + Service Bureau UI + post_executor integration + MSI 1.0.34).
- **Связанные документы:** `phase-1-definition-of-done.md` v1.1, `phase-1-pal-trait-spec.md` v3, `phase-1-claude-cli-driver-IMPL-REFERENCE.md` v1.1, `02-Patterns/rebuild-msi-playbook-v1.0.33.md`, `02-Patterns/документы-с-кодовыми-путями-из-реального-репозитория.md`.

---

## Принципы оценки

**Вероятность (P):**
- **Низкая** — наблюдалось ≤1 раз в прошлых фазах; требует совпадения обстоятельств.
- **Средняя** — наблюдалось 2-3 раза в прошлых фазах; срабатывает в нетривиальном сценарии.
- **Высокая** — известная грабля проекта v1.0.33 или регулярно наблюдается; срабатывает «само».

**Влияние (I):**
- **Низкое** — UX неудобство; обходимо одним кликом или явной инструкцией.
- **Среднее** — задача / провайдер недоступны, но восстанавливаются перезапуском или переключением.
- **Высокое** — БД повреждена / провайдер неработоспособен / fail задач у Владельца; восстановление через rollback / playbook.
- **Критическое** — fatal init / installer-upgrade сломан / data loss; **блокер релиза**.

**Топ-риск = High × Critical** (или High × High при необратимых эффектах). Приоритет mitigation в первых этапах sequencing DoD.

---

## Технические риски (15 шт., ведёт программист)

### Группа A — предзаготовлены в DoD Open Q (структурированы + добавлен mitigation)

| # | Риск | P | I | Mitigation (реализуемая в коде Phase 1) | Триггер обнаружения | DoD-связь |
|---|---|---|---|---|---|---|
| **R-T-001** | **MCP per-post config — миф.** Claude CLI читает MCP только глобально из `~/.claude/mcp.json` или per-project `.mcp.json` в CWD; нет per-agent injection в `--agent` frontmatter (verified IMPL-REFERENCE §4). Если в Phase 1 заявим `supports_mcp=true` с разными MCP per post — реально работать не будет. | Высокая | Среднее | `ClaudeCliDriver::capabilities().supports_mcp = false` в Phase 1. `request.mcp_bindings` игнорируются с warning лог `"mcp bindings dropped: ClaudeCli Phase 1 MVP не поддерживает per-post MCP"`. Per-post MCP → Phase 2 R&D (3 кандидата: `.mcp.json` в CWD выполнения / `--mcp-config` CLI флаг / `mcp_servers:` в frontmatter — проверять на актуальной CLI v2.1.140). | В логах warning `mcp bindings dropped` для пост-задач. Code review должен поймать любую попытку выставить `supports_mcp=true` без R&D. | AC-002.4 (capabilities честные), Q-005 (timeout/MCP policy enforced). |
| **R-T-002** | **Orphan `claude.exe` после Tauri crash.** `kill_on_drop(true)` срабатывает только при штатном drop future. При crash самого MSPro процесса — orphan'ы остаются. Win32 Job Object **отложен в 11B-bis** (`lib.rs:154` явно «отложен»). `cleanup_orphan_post_processes()` в `post_executor.rs:527` сейчас stub no-op (комментарий «на 11B-2 если Job Object не справится»). | Средняя | Среднее | Phase 1 принимает текущее `kill_on_drop` (~95% случаев). Дополнить **startup orphan sweep**: расширить stub `cleanup_orphan_post_processes` через `sysinfo` фильтр по env-var `MSPRO_TASK_ID` (пост-агенты помечают себя — чище чем по имени binary). Полная Win32 Job Object — Phase 2 backlog (тикет «закрыть 11B-bis»). UI: Settings → кнопка «Cleanup zombie processes» как ручной fallback. | После принудительного завершения MSPro (Task Manager): `tasklist /fi "imagename eq claude.exe"` показывает процессы с CWD внутри `Outbox/`. | Q-002 (нет orphan после 10 задач). |
| **R-T-003** | **Backup `app.db` WAL прогрев блокирует UI.** `PRAGMA wal_checkpoint(TRUNCATE)` или file copy при активной транзакции занимает 5-10 сек; на больших БД (>50MB) — до 30 сек. UI поток ждёт. | Средняя | Среднее | Backup выполняется в `tokio::task::spawn_blocking` (не на UI thread). Pre-backup invariant: дождаться завершения активных Claude-вызовов через `PostExecutorRegistry.running.lock().await.is_empty()` poll до начала backup. UI плашка «Сохраняем БД, ~30 сек, не выключайте приложение» с прогресс-индикатором. Backup делаем **только перед migrations 08-09** (Этап 1 sequencing), не каждый run. | UI замораживается >3 сек при «Backup app.db». В логах — `PRAGMA wal_checkpoint` > 5000ms. Владелец видит окно «Не отвечает». | Q-003 (миграции на свежей+существующей БД), Q-004 (backup pre/post phase-1). |
| **R-T-004** | **Большой stdout (>100KB) от Claude CLI buffer growth.** В `--output-format text` весь stdout копится в `output_buffer: String` внутри `ClaudeCliDriver::invoke`. На задачах генерации длинных документов (>50K output tokens) буфер достигает 200KB+. Память не критична, но при сохранении в `run_logs.raw_output` без truncation — БД row blow up. | Низкая | Низкое | `output_buffer.len()` cap = 256KB: `if buffer.len() > 256*1024 { log::warn!("output buffer cap reached, truncating"); break; }`. `run_logs.raw_output` пишется через `truncate_to_64kb(&buffer)` helper (AC-002.7 уже это требует). Test: mock-агент генерит 500KB output → driver не падает, `run_logs.raw_output` обрезан до 64KB. | OOM крах процесса или INSERT INTO `run_logs` с row > 100KB. | AC-002.7 (raw_output truncation ≤64KB). |
| **R-T-005** | **CEO migration Phase 2: непрозрачные deps в `claude_bridge.rs`.** ~430 строк содержат CEO chat lifecycle (`ChatLifecycle`), Dispatcher router (`DispatcherLifecycle`), `hide_console`, `ensure_mspro_ceo_agent`, `ensure_mspro_dispatcher_agent`, `detect_claude_cli`. Перенос всего на PAL = unentangle всех зависимостей одновременно — high regression risk. | Средняя | Среднее | **Phase 1 НЕ трогает `claude_bridge.rs`** — явно зафиксировано в DoD §«Phase 1 scope vs DEC». CEO остаётся на legacy. Подготовка: создать `Vault/03-Phases/phase-2-prep-investigation.md` (backlog тикет до старта Phase 2) — Cursor делает deps map. Принцип: ничего не мигрировать в Phase 1 кроме post_executor; CEO + Dispatcher routing — отдельная work item Phase 2 с собственным DoD. | Любая попытка `grep -r "pal::invoke" claude_bridge.rs` находит совпадения → значит кто-то начал миграцию вне scope. | R-002 (CEO chat работает legacy), R-003 (Dispatcher routing работает legacy). |

### Группа B — дополнительные риски из реального кода v1.0.33

| # | Риск | P | I | Mitigation (реализуемая в коде Phase 1) | Триггер обнаружения | DoD-связь |
|---|---|---|---|---|---|---|
| **R-T-006** | **Self-healing блок забыли для миграций 08-09.** Грабля 08-tribal: `tauri-plugin-sql` кэширует rolled-back миграции и не ретраит. `lib.rs::setup` имеет **6 ручных self-healing блоков** (HMT statistics, Step 9 full, v1.0.13 dept, v1.0.21 per-post, v1.0.22 dispatcher_hub, TICKET-001 vault_ops_log — Grep подтвердил). Если для **5 новых таблиц** Phase 1 (`provider_registry`, `tier_presets`, `post_runtime`, `run_logs`, `provider_health_log`) забыть self-healing — installer-upgrade сломается у beta-тестеров. | **Высокая** | **Критическое** | **Обязательный code-review checkpoint** перед commit миграций 08-09: checklist «для каждой новой таблицы — self-healing блок `CREATE TABLE IF NOT EXISTS` (+ нужные `ALTER` через `PRAGMA table_info`) через `WritePool` raw query в `lib.rs::setup()`». **Test:** rollback миграции 08 в `_sqlx_migrations` → app start → таблицы должны появиться без ручной правки БД. PR-template для Phase 1 содержит чек-бокс «self-healing для всех новых таблиц добавлен». | Installer upgrade с v1.0.33 → v1.0.34 у beta: `migration 8 modified` / `no such table: provider_registry` / `no such table: run_logs`. | Q-003 (миграции на свежей+существующей БД с self-healing). |
| **R-T-007** | **Миграция rollback не ретраит — partial index + ALTER в одной транзакции.** Известная грабля (08-tribal): если в одной миграции `ALTER TABLE ADD COLUMN` + `CREATE INDEX ... WHERE ...` (partial), sqlx откатит транзакцию при ошибке частичного индекса; плагин запомнит «применено», но в БД ничего нет. Defensive `CREATE TABLE IF NOT EXISTS` в setup() подхватит таблицы — но **не индексы**. | Средняя | Высокое | **Phase 1 правило:** разделить миграции — 08 = только `CREATE TABLE` + seed `provider_registry`/`tier_presets`; 09 = `run_logs` + `provider_health_log` + плоские `CREATE INDEX` (БЕЗ `WHERE`). Никаких `ALTER` + partial `CREATE INDEX` в одной миграции. Если partial index нужен — отдельная миграция 10. **Test:** rollback миграции 08 → start → `SELECT count(*) FROM provider_registry` работает (self-healing), индексы тоже есть (раз отдельная миграция — не зависит). | После rollback одной миграции: ловим расхождение «таблица есть, но индекса нет» через `PRAGMA index_list(...)`. | Q-003. |
| **R-T-008** | **`pal_enabled` mid-task переключение race.** `settings.pal_enabled` можно поменять live (без рестарта; setter в SettingsStore — runtime-mutable). Если task стартовала с `pal_enabled=true`, посередине Владелец/Cursor переключил `false` — current task в pal-режиме, новые в legacy. Confused state, ошибки невоспроизводимые. | Низкая | Среднее | **Read flag один раз в начале task'a**, сохранить в local `let pal_enabled_at_start = settings.pal_enabled;`. Внутри `post_executor::run_claude_cli_for_post` НЕ делать live re-read. **Test:** установить `pal_enabled=true` → стартовать long-running task → переключить на `false` через 10 сек → проверить что текущая задача завершилась через PAL (`run_logs` запись есть), следующая — через legacy (`run_logs` записи нет, прямой spawn). | run_logs показывает запись через PAL, но логи `post_executor` пишут `using legacy path` — расхождение. | R-004 (killswitch `pal_enabled=false` возвращает к legacy). |
| **R-T-009** | **Все провайдеры в fallback chain упали — нет осмысленного ответа.** Если у поста chain `[claude_cli, qwen_http]` и оба `Unreachable` (Anthropic outage + Ollama не запущен) — orchestrator может зациклиться или вернуть последнюю ошибку без накопления контекста. Владелец не понимает что упало. | Средняя | Среднее | Orchestrator после exhaustion chain возвращает **агрегированную ошибку**: `ProviderError::Other(format!("All {n} providers in chain failed: [{p1}: {e1}; {p2}: {e2}]"))`. `dispatcher_logs.fail_reason` содержит ВСЕ последовательные ошибки (для UI Awaiting). **Never retry chain бесконечно** — один проход по chain max. UI: красная плашка с раскрывающимся списком «попытки 1/N». | Логи показывают `pal_invoke: chain exhausted after {n} attempts, last_err=...`. UI Awaiting → tasks с reason `All providers failed`. | F-004 (fallback chain end-to-end). |
| **R-T-010** | **Timeout reconciliation: три источника + Tier presets могут разойтись.** Live в `settings/mod.rs`: `claude_cli_timeout_sec=360` (CEO), `dispatcher_routing_timeout_sec=180` (Dispatcher), `post_executor_timeout_sec=600` (post-agent). После Phase 1 — Tier presets из БД (T1=600, T2=360, T3=60). Если Владелец поменял T1 через UI на 400 — а `settings.post_executor_timeout_sec=600` остался — внешний kill timeout сработает позже PAL hard cap. Разнобой = unpredictable behaviour. | Средняя | Высокое | На startup в `lib.rs::setup`: `debug_assert!(settings.post_executor_timeout_sec >= max(Tier::T1::default_timeout(), Tier::T2::default_timeout(), Tier::T3::default_timeout()))`. Если расхождение в release-build — лог warning + UI badge «settings/Tier mismatch» в Service Bureau. **Tier presets** — source of truth для PAL Phase 1. `settings.claude_cli_timeout_sec` остаётся ТОЛЬКО для CEO (legacy не PAL). UI Tier Preset modal в Service Bureau показывает inline «T1 timeout = PAL hard cap = post_executor_timeout_sec» с возможностью sync «Применить к settings». | `run_logs.latency_ms` > `effective_timeout` (приходит из PAL hard cap), а post_executor выживает дольше → diagnostic: timeout не совпали. | Q-005 (timeout policy enforced; hard cap 600s). |
| **R-T-011** | **Job Object отложен (11B-bis) — orphan защита только `kill_on_drop`.** Структурно та же грабля что R-T-002, но **архитектурно**: в Phase 1 НЕ закрываем 11B-bis. Любой crash MSPro = orphan `claude.exe` возможен. Со временем (месяцы использования) orphan'ы накапливаются. | Низкая | Среднее | **Принять** в Phase 1 (не блокер). Добавить в Phase 1 backlog: «Phase 2 — реализовать Win32 Job Object 11B-bis». UI: Settings показывает badge «Orphan-detector: stub-mode (Phase 1)» — видно что защита не полная. Ручной cleanup кнопка (см. R-T-002 mitigation). Документировать в `phase-1-completion-report.md` lessons learned: «11B-bis открыт, перенос в Phase 2 ticket». | Регулярные сигналы от Q-002 после каждой ~10 задач или жалоба «накопилось claude.exe» от Владельца через несколько дней использования. | Q-002 + новый Phase 2 backlog ticket (Job Object 11B-bis). |
| **R-T-012** | **`keyring` DPAPI ошибки на bootstrap.** `keyring v3` с `windows-native` feature (Cargo.toml line 51). На старте если DPAPI инициализация падает (guest user без permissions, Windows 7, corrupt user profile) — секреты `claude_cli_oauth` не читаются → провайдер `claude_cli` не работает, но MSPro грузится и UI показывает «всё ок». Тихий fail. | Низкая | Высокое | `keyring::Entry::new(...).get_password()` → match на ошибку: лог warning + установить `provider_registry.status='disabled'` для соответствующего провайдера + UI плашка в Service Bureau «секрет `claude_cli_oauth` недоступен — нажмите Reconfigure». **НЕ** падать на startup — продолжить работу с пометкой провайдера disabled. **Test:** mock keyring failure (например, через override env-var `KEYRING_FORCE_FAIL=1` если поддерживается) → app starts → `claude_cli` помечен `disabled` в registry, UI показывает причину. | App start logs: `keyring read failed for claude_cli_oauth: <error>`. Service Bureau Providers tab: `claude_cli` бейдж `Unauthorized`. | AC-001.6 (секреты в OS Keychain). |
| **R-T-013** | **Hot-swap во время активного run — race condition.** Владелец в UI меняет `post_runtime.primary_provider_id` для поста, у которого **прямо сейчас** идёт task. Race: task читала old runtime в start, hot-swap записал new в БД, но текущий run уже на old. UI говорит «новый провайдер активен», по факту run всё ещё старый. | Низкая | Низкое | **Snapshot `post_runtime` в начале task'a** (как `pal_enabled` в R-T-008): локальная переменная для всего lifecycle. Hot-swap применяется к **следующему** run этого поста. UI Toast: «Изменение применится к следующей задаче поста (текущая выполняется на {old_model})». DEC-003 «hot-swap без рестарта» соблюдается — рестарта не требуется, новый run сразу использует new. | `run_logs.provider_used` != `post_runtime.primary_provider_id` в момент окончания (для свежезаписанной задачи). | AC-003.2 (hot-swap без рестарта — для следующего run). |
| **R-T-014** | **Test connection blocking — UI freeze.** В `AddProviderModal` Test connection делает blocking call (HTTP к Ollama, `claude --version` через spawn). Если endpoint медленный (Ollama стартует 10-15 сек на первом вызове, `claude.exe` load 3-5 сек на холодном старте) — Save button disabled, UI выглядит зависшим. | Средняя | Низкое | Test connection с **internal timeout 10 сек** (`tokio::time::timeout(Duration::from_secs(10), check_fut)`). UI показывает spinner + «Проверяю... до 10 сек». На timeout — Toast «Не удалось проверить за 10 сек; проверьте endpoint». Save разблокируется по успеху ИЛИ ручной override (галочка «Знаю, что endpoint медленный, сохранить без проверки» — для localhost edge case). | UI feedback от Владельца: «модалка не отвечает на Test». В логах: `test_provider_connection elapsed > 10s`. | F-007 (Test connection blocking — но не залипает дольше 10 сек). |
| **R-T-015** | **`run_logs` unbounded growth — БД blow up.** Каждый PAL invoke = одна запись (~16 metric columns + `raw_output` ≤64KB). Активное использование (100 задач/день × 64KB raw = 6.4MB/день; за год = 2.3GB+). Без cleanup БД vacuum растёт неконтролируемо, performance падает. | Средняя | Высокое | Phase 1 базовое: `run_logs.raw_output` сразу truncated ≤64KB (AC-002.7). Phase 1.5 backlog: **nightly cleanup task** — `raw_output` хранить только last 100 runs per provider ИЛИ TTL 30 дней (`DELETE FROM run_logs WHERE created_at < datetime('now', '-30 days')` для метрик; для `raw_output` — `UPDATE run_logs SET raw_output = NULL WHERE created_at < datetime('now', '-7 days')`). Vacuum раз в неделю (cron). UI dashboard «Run Logs disk usage: 240 MB / 2 GB» + кнопка «Cleanup older than 30 days». В Phase 1 — только monitoring (метрика disk usage), реализация cleanup — Phase 1.5. | `SELECT sum(length(raw_output)) FROM run_logs` > 500MB. UI показывает плашку «БД растёт быстро, рекомендуем настроить cleanup в Phase 1.5». | AC-002.7 (truncation ≤64KB per row — уже есть); новый Phase 1.5 backlog ticket «run_logs cleanup task + TTL». |

---

## ⚠️ Топ-3 риска Phase 1 (приоритет mitigation в начале фазы)

Эти риски имеют наибольший product `P × I` и должны быть закрыты mitigation **до старта Этапа 2** sequencing DoD (т.е. до начала реализации PAL trait + драйверов).

### 🔴 1. R-T-006 — Self-healing блок забыли для миграций 08-09 (High × Critical)
- **Почему топ:** грабля **уже сработала** в прошлых фазах 4 раза (HMT, Step 9, v1.0.21, v1.0.22, vault_ops_log — каждый раз reactivere fix). Без `CREATE TABLE IF NOT EXISTS` в `lib.rs::setup` upgrade с v1.0.33 → v1.0.34 у любого beta-тестера крашнет init на migration mismatch / no such table.
- **Mitigation reflex:** **Этап 1 sequencing DoD** — обязательный self-healing блок для всех 5 новых таблиц. PR-template checkbox. Test rollback миграции 08.
- **Кто следит:** программист пишет блоки, Cursor review перед merge в main.

### 🟠 2. R-T-001 — MCP per-post миф (High × Medium)
- **Почему топ:** **уже опровергнут** в IMPL-REFERENCE §4 — Claude CLI v2.1.140 поддерживает MCP только глобально. Если кто-то на этапе implementation поставит `supports_mcp=true` в `ClaudeCliDriver::capabilities` — Phase 1 deliverable формально пройдёт, но реальные MCP вызовы из пост-агентов работать не будут (silent fail).
- **Mitigation reflex:** **Этап 2 sequencing** — `capabilities.supports_mcp = false` + warning log на сброс `mcp_bindings`. Per-post MCP — Phase 2 R&D.
- **Кто следит:** программист при имплементации `ClaudeCliDriver::capabilities()`.

### 🟠 3. R-T-007 — Миграция rollback не ретраит при partial index + ALTER (Medium × High)
- **Почему топ:** грабля наблюдалась в v1.0.21 и v1.0.22 (см. `08-tribal-knowledge.md`). Если в миграциях 08-09 окажется `ALTER TABLE ... ADD COLUMN` + `CREATE INDEX ... WHERE ...` в одной транзакции — sqlx откатит, плагин запомнит «применено», БД полу-готова, self-healing блок не покрывает индексы.
- **Mitigation reflex:** **Этап 1 sequencing** — миграции 08 и 09 чистые `CREATE TABLE` + seed; никаких `ALTER` + partial `CREATE INDEX` в одной миграции. Если partial index нужен — отдельная миграция 10.
- **Кто следит:** программист пишет миграции, Cursor review SQL.

**Watch-list (P × I = Medium-High, мониторим, но не блокеры):** R-T-010 timeout reconciliation, R-T-015 `run_logs` growth, R-T-009 chain exhaustion.

---

## §5 — Бизнес/процессные риски (CEO зона)

> §5 заполнено Гендиром 2026-05-28. Префикс R-B-XXX (Business) отличает от R-T-XXX (Technical) в §3. Топ-3 выделены жирным. Cross-link R-T — Cursor v1.1 (R-B-006/007/008).

| ID | Риск | P | I | Mitigation | Cross-link R-T | Триггер |
|---|---|---|---|---|---|---|
| R-B-001 | **Phase 1 timeline slip (14 дней → 21+)** — Phase 0 заняла 6 дней вместо 4-5; Phase 1 шире (trait + 2 driver + UI + миграции + integration). | Высокая | Высокое | Daily sync Гендир↔Owner. Incremental save обязателен. Scope freeze после DoD approve. Milestones: Day 3 trait+IMPL-REF / Day 7 UI / Day 10 интеграция / Day 14 release. | — | Day 5 не закрыт ClaudeCliDriver; Day 10 нет E2E PAL→post_executor. |
| R-B-002 | Scope creep — попытки «а ещё бы…» (CEO migration, MCP per-post, dashboard). | Средняя | Среднее | DoD = контракт scope (43 критерия = граница). Новое → Phase 2 backlog. Только Owner расширяет через explicit DEC. | R-T-001, R-T-005 | Новые требования вне DoD >2 раз/неделю. |
| R-B-003 | Конфликты записи Vault (Гендир/программист/Cursor в один файл) — потеря работы, спорные правки. | Средняя | Среднее | Role division (trait v3.1 §13): один файл = один владелец. Гендир — trait/wireframes/DoD/risks/plan. Программист — IMPL-REFERENCE/код. Cursor — review через Owner. Changelog с автором/датой. | — | Один файл >3 правок разных авторов за день; merge-конфликт git на phase-1-*.md. |
| R-B-004 | Токен-бюджет на integration tests — реальные прогоны Opus съедают месячную квоту за неделю. | Высокая | Среднее | Integration с реальным Claude — только CI=full flag. На dev — MockProvider. T2/T3 (Sonnet/Qwen) для тестов (3-5× дешевле Opus). Usage dashboard >70% к 20-му → пауза. | R-T-004, R-T-015 | Usage >70% к 20-му числу; Anthropic email-warning по Opus quota. |
| R-B-005 | **UAT задержка от Владельца** — Phase 1 в Awaiting, Owner занят корп.договорами МСПро → sign-off простаивает. | Высокая | Высокое | UAT-чеклист за 2 дня до конца фазы. 30-мин UAT-окна 2-3×/неделю в календаре. Phase 2 prep параллельно. UAT >5 дней без действия → перепланирование. | — | DoD ☑ закрыт, но в Awaiting >3 дней без сигнала Owner. |
| R-B-006 | External vendor drift — Anthropic deprecation warning на CLI флаги; Ollama API breaking changes раз в 3-6 мес. | Средняя | Высокое | Weekly review Anthropic Changelog + Ollama notes. claude_cli_version_expected + ollama_api_version_expected в settings с warning при mismatch. Hotfix в IMPL-REFERENCE. | **R-T-001**, **R-T-005**, **R-T-009** | Anthropic breaking change < Phase 1 end; Ollama 404/400 на устоявшийся endpoint. |
| R-B-007 | **Региональные блокировки/санкции по API** — РФ + Anthropic = риск 403/AuthFailed на массиве постов. | Средняя | Критическое | Qwen local (Ollama) — primary fallback по DEC-001. VPN routing на уровне ОС. Резерв бюджета на резидентный прокси. Health Monitor ловит массовый AuthFailed (>3 постов) → авто-T3. | **R-T-009**, **R-T-012** (DoD AC-001.2) | Claude CLI AuthFailed массовый; Anthropic 403 на 3+ запросах подряд. |
| R-B-008 | Гендир hallucinations про реальный код — 3 кейса (workspace в SPEC, табы EditPostKnowledgeModal, фантомный run_logs). | Средняя | Среднее | Role division (trait v3.1 §13): Гендир не пишет про реальный код. Программист — IMPL-REFERENCE source of truth. Cursor проверяет SPEC до approve. Паттерны «документация-следует-за-кодом» + «документы-с-кодовыми-путями-из-репо». | *процессный, без R-T аналога* | Cursor находит ≥2 расхождения с реальным кодом в одном артефакте Гендира. |
| R-B-009 | Knowledge fragmentation между сессиями Гендира — stateless контекст, потеря договорённостей mid-phase. | Средняя | Среднее | Все решения → decisions-log.md. Паттерны → Vault. Incremental save. Phase 1 progress tracker для cross-session continuity. | — | Один вопрос обсуждается заново в 2+ сессиях Гендира. |
| R-B-010 | Phase 1 закрывается без полного Quality block (Q-001…Q-008) — баги в проде. | Средняя | Высокое | Q-блок = blocking. Перед approve: cargo test green, clippy, orphan check, backup verified, миграции lf(). PR в main без green CI отклоняется. | R-T-006, R-T-007 | PR в main без green CI; пропуск ≥1 пункта Q-001…Q-008 в sign-off. |

### Топ-3 бизнес-рисков (P × I)

1. **R-B-005** — UAT задержка (Высокая × Высокое) — главный business-блокер.
2. **R-B-001** — Timeline slip (Высокая × Высокое) — по историческому базису Phase 0.
3. **R-B-007** — Региональные блокировки (Средняя × Критическое) — низковероятный, фатальный без fallback.

---

## Связь с DoD v1.1 — какие критерии проверяют mitigation

| Риск | DoD-критерии для verification mitigation |
|---|---|
| R-T-001 | AC-002.4 (capabilities честные); Q-005 (timeout/MCP policy enforced). |
| R-T-002 | Q-002 (нет orphan `claude.exe` после 10 задач последовательно). |
| R-T-003 | Q-003 (миграции на свежей+существующей БД); Q-004 (backup pre/post phase-1). |
| R-T-004 | AC-002.7 (`raw_output` truncation ≤64KB). |
| R-T-005 | R-002 (CEO chat legacy работает); R-003 (Dispatcher routing legacy работает). |
| R-T-006 | Q-003 (self-healing для 08-09 присутствует и подхватывает rollback). |
| R-T-007 | Q-003 (rollback миграции 08 → app start → таблицы и индексы на месте). |
| R-T-008 | R-004 (killswitch `pal_enabled=false` возвращает к legacy без mid-task confusion). |
| R-T-009 | F-004 (fallback chain end-to-end — два attempt'а с агрегированной ошибкой). |
| R-T-010 | Q-005 (timeout policy enforced; hard cap 600s = `post_executor_timeout_sec`). |
| R-T-011 | Q-002 + новый Phase 2 backlog ticket (Job Object 11B-bis). |
| R-T-012 | AC-001.6 (секреты в OS Keychain — graceful failure при недоступности DPAPI). |
| R-T-013 | AC-003.2 (hot-swap без рестарта; для **следующего** run, не current). |
| R-T-014 | F-007 (Test connection blocking, но с internal timeout 10 сек). |
| R-T-015 | AC-002.7 (truncation already in place); новый Phase 1.5 backlog (cleanup + TTL). |

---

## Open questions / risk evolution

Эти вопросы остаются открытыми на момент v1.0 risk register. Обновлять через **addendum** (v1.1, v1.2, …), не overwrite.

1. **`--print` deprecation timing** (из IMPL-REFERENCE §7.1) — нужно ли в Phase 1 уже добавить «backup path» через `claude run` (новая команда), или Phase 1 остаётся на `--print`? Решение зависит от `claude --help` на v2.1.140 (программист проверит на implementation).
2. **`run_logs` schema:** нужно ли поле `mcp_bindings_dropped: i32` для аудита R-T-001? (Если да — в migration 09.)
3. **Tauri event delivery guarantees:** `provider_health_changed` event может «потеряться» если React subscription стартует ПОСЛЕ emit. Подписка должна быть на mount, эмит — после первого poll cycle через debounce. Уточнить при имплементации `health_monitor.rs`.
4. **Anthropic prompt caching gap:** `ProviderResponse.usage.cache_read_tokens`/`cache_write_tokens` есть в trait, но Claude CLI `--output-format text` не даёт это в stdout. Cost-tracking gap (R-T в IMPL-REFERENCE §7.2). Phase 1.5 R&D: anthropic-tokenizer post-hoc estimate.
5. **Phase 1.5 vs Phase 2 разделение:** некоторые риски имеют mitigation которая **частично в Phase 1**, **полностью в Phase 1.5** (R-T-015 cleanup, R-T-011 Job Object). Нужно ли явное Phase 1.5 этап в roadmap? Решение Владельца.

---

## Changelog

- **v1.1 (2026-05-28)** — §5 вставлена текстом Гендира (patch_vault AnchorNotFound); заглушка удалена; cross-links: R-B-006→R-T-001/005/009, R-B-007→R-T-009/012, R-B-008→процессный; R-B-002→R-T-001/005; R-B-004→R-T-004/015; R-B-010→R-T-006/007.
- **v1.0 (2026-05-28)** — программист, первая редакция: 15 технических рисков (5 из DoD Open Q + 10 из реального кода v1.0.33), Топ-3 выделены, заглушка для бизнес-рисков Гендира, связь с DoD. Источники: `lib.rs::setup` (6 self-healing блоков verified), `settings/mod.rs` (3 timeout-поля verified), `migrations/` (7 файлов через `lf()` verified), IMPL-REFERENCE §4/§7. Phase 0 lessons applied (играя 08-tribal — R-T-006/R-T-007; 11B-bis — R-T-002/R-T-011).

*End of risk register v1.1 — 15 R-T + 10 R-B = 25 рисков.*
