# Промпт для Гендира — программа AgentPod (DEC-001…004)

> Скопируйте содержимое блока ниже **одним сообщением** в чат Гендира в MSPro-Ltd Corp.

---

```
Владелец. Утверждаю четыре фундаментальных решения по программе AgentPod.
Зафиксируй в Vault/decisions-log.md (создай файл, если нет).
Формат каждого DEC: ID, дата 2026-05-20, статус Accepted, контекст, решение,
acceptance criteria, out of scope.

После фиксации DEC — подготовь одну программу AgentPod на review (структура ниже).
Один план, без альтернатив и без Path B.

═══════════════════════════════════════════════════════════════
DEC-001 — Vendor-agnostic runtime: Service Bureau (СБ) + PAL
═══════════════════════════════════════════════════════════════

КОНТЕКСТ
Сейчас post_executor завязан на claude.exe; tiered-models — паттерн в Vault,
не единый runtime. Нужен единый механизм моделей для всех Pod без привязки
к одному вендору.

РЕШЕНИЕ — три слоя (снизу вверх):

┌─────────────────────────────────────────────────────────────┐
│ СЛОЙ 3 — UI Pod Runtime                                     │
│ Dropdown модели, tier T1/T2/T3, fallback chain, hot-swap   │
└──────────────────────────────┬──────────────────────────────┘
┌──────────────────────────────▼──────────────────────────────┐
│ СЛОЙ 2 — PAL (Provider Abstraction Layer)                   │
│ Единый протокол вызова: prompt + tools policy + timeout     │
│ → response + usage metrics → Run Logger                     │
└──────────────────────────────┬──────────────────────────────┘
┌──────────────────────────────▼──────────────────────────────┐
│ СЛОЙ 1 — Service Bureau (СБ)                                │
│ Credential broker + health + fallback chain registry        │
└─────────────────────────────────────────────────────────────┘

СЛОЙ 1 — Service Bureau (СБ):
• Технический модуль MSPro, НЕ новый орг-пост в Фазе 1 программы.
• НЕ путать с «Финансовым отделением» (dept 3) в оргсхеме — это разные сущности.
• Единственная точка credentials для всей системы.
• Хранение: существующий DPAPI/Tauri secrets (secret_set/get) +
  новые таблицы provider_registry (см. SQL-spec в ответе).
• Health per provider (обновление ≤30 сек в UI):
  - alive
  - quota_exceeded
  - rate_limited
  - token_expired
  - unreachable
• Auto-fallback: при сбое primary → следующий в fallback_chain[] (логируется).
• Регистрация нового API/local провайдера через UI СБ без правки Rust-кода.

MVP СБ (входит в acceptance DEC-001):
1. Claude CLI — executable path + default model id
2. Qwen/Ollama — OpenAI-compatible base URL + model id
3. Contract-stub: external_agent_gateway
   — MSPro WebSocket ws://127.0.0.1:8899, JSON-RPC (ping, state, sql/query,
     dispatcher/submit, ceo/respond)
   — В PAL: интерфейс ExternalAgentProvider зарегистрирован
   — Полная реализация вызова через gateway — Phase 2 программы

ВНЕ MVP DEC-001 (Phase 2 R&D, явно не в acceptance):
• OAuth / subscription profiles (Claude Max, ChatGPT Plus, Gemini Advanced)
• Авто-refresh браузерных сессий подписок
• Claude Agent SDK как замена CLI — только если CLI не закрывает MCP/артефакты

СЛОЙ 2 — PAL:
• Trait/протокол PostRuntimeProvider:
  invoke(request) → ProviderResponse { text, usage, error, latency_ms }
• Цепочка: Pod → PAL → СБ.get_channel(provider_id) → driver → ответ → run_logs
• Эволюция текущего post_executor.rs — НЕ второй параллельный раннер с нуля.
• Drivers MVP: ClaudeCliDriver, QwenHttpDriver
• Driver stub: ExternalGatewayDriver (возвращает NotImplemented в MVP — OK)

СЛОЙ 3 — UI Pod Runtime:
• Экран «Runtime поста» (расширение «Знания поста»):
  - primary_provider_id (FK → provider_registry)
  - primary_model
  - fallback_chain_json (упорядоченный список provider_id)
  - tier: T1 | T2 | T3
• Сохранение SQLite; PAL читает перед каждым run
• Hot-swap: без рестарта MSPro, без деплоя, без ручного .env
• Dropdown «Модель» = все провайдеры со status=alive или degraded (не dead)

ACCEPTANCE CRITERIA DEC-001 (MVP):
□ Регистрация API-провайдера в UI СБ ≤10 мин → появляется в dropdown всех Pod
□ Health-статус каждого провайдера виден в UI СБ, обновление ≤30 сек
□ Сбой primary → auto-fallback, запись run_logs.fallback_used=true + reason
□ Смена модели Pod в UI ≤5 мин → следующий run на новой модели
□ PAL unit-тесты: mock driver, fallback chain, error mapping — проходят в cargo test

═══════════════════════════════════════════════════════════════
DEC-002 — Pod Template = цель программы (универсальная болванка)
═══════════════════════════════════════════════════════════════

КОНТЕКСТ
Цель программы — НЕ «доделать office-manager». Цель — Pod Template: технический
каркас, из которого любой новый пост поднимается через UI за минуты, а не пишется
с нуля в чате.

РЕШЕНИЕ — обязательные компоненты Template (все в MVP программы):

| # | Компонент | Назначение |
|---|-----------|------------|
| 1 | СБ + PAL integration | Готовое подключение к моделям |
| 2 | System Prompt Store | ≤130 строк, валидатор, post_prompt_history |
| 3 | Vault Scaffolder | Авто-дерево Vault/posts/<slug>/ по шаблону |
| 4 | MCP Loader | post_mcp_bindings в БД; MVP: list + stub spawn |
| 5 | Task Queue | post_slug, priority, status, payload, created_at |
| 6 | Run Logger | tokens, latency, cost_estimate, success, artifact_path, model, provider, fallback |
| 7 | Heartbeat MVP | Event-driven (задача в queue → wake Pod) + 1 cron preset (low). High-freq/conditional — Phase 2 |
| 8 | Health Monitor | 5× same error → escalate в очередь Гендира |
| 9 | Debug Console | Чат с постом: контекст posts/<slug> + Outbox only |

VALIDATION CLIENTS (не цель программы):
• office-manager = client #1, проверка Template end-to-end
• Второй пост (slug на выбор Владельца) = client #2, проверка повторяемости фабрики

ACCEPTANCE CRITERIA DEC-002:
□ office-manager полностью настроен через UI Template ≤1 час
□ Второй пост создан и настроен через тот же UI ≤30 минут
□ Если второй пост не создаётся тем же путём — Template не готов, программа НЕ закрыта
□ Smoke gate Фазы 4: оба поста выполнили ≥1 задачу → артефакт в Outbox → approve в UI

OUT OF SCOPE DEC-002:
• Department orchestrator (маршрут A→B внутри отдела) — отдельная программа
• Полный MCP docx pipeline — Phase 2 (после stub)

═══════════════════════════════════════════════════════════════
DEC-003 — Model & tier switching (персистенция)
═══════════════════════════════════════════════════════════════

РЕШЕНИЕ
Все настройки runtime поста в SQLite (таблица post_runtime или расширение posts):
• primary_provider_id, primary_model
• fallback_chain_json
• tier: T1 | T2 | T3
• max_turns (опционально, default из tier preset)

Поведение:
• Изменение в UI → сохранение → следующий run Pod использует новые значения
• Без рестарта приложения, без правки кода, без ручного .env
• Tier presets (документированы в Vault): T1=Opus-class, T2=Sonnet-class, T3=Qwen-local

ACCEPTANCE DEC-003:
□ Смена tier в UI меняет default model suggestion в dropdown
□ Запись в run_logs содержит фактические provider_id + model_used

═══════════════════════════════════════════════════════════════
DEC-004 — Нейтральная терминология (ТОЛЬКО UI + CEO-навигатор)
═══════════════════════════════════════════════════════════════

КОНТЕКСТ
Убираем авторские бренды из пользовательских поверхностей. Методология остаётся,
технический legacy в коде — не трогаем в этой программе.

РЕШЕНИЕ — публичные замены:
• «Хаббард», «Высоцкий» → не упоминать в UI и CEO-навигаторе
• «HMT» в UI → «Knowledge» / «База знаний»
• «Хаббард-каноны» → «Правила работы» / Operational Rules

НЕ МЕНЯТЬ в этой программе (отдельная техпрограмма позже):
• Rust-модули, SQLite-таблицы, префиксы hmt_
• Tool read_hmt_topic (имя tool без переименования)
• Папка Vault/01-HMT-Knowledge/ (переименование — отдельный DEC)
• Термины: «отделение», «ЦКП», «состояние поста», 8 отделений MSPro

ACCEPTANCE DEC-004:
□ В CEO_CORE_PROMPT нет слов Hubbard/Vysotsky/HMT в пользовательском тексте
□ В UI нет «HMT» в видимых лейблах (допустимо в dev-only debug)
□ cargo test + migration_tests проходят без переименования hmt_ таблиц

═══════════════════════════════════════════════════════════════
АРХИТЕКТУРНЫЕ ОГРАНИЧЕНИЯ (не обсуждаются в программе)
═══════════════════════════════════════════════════════════════

• Один Hub-and-Spoke Диспетчер на всю корпорацию. Без «диспетчера отделения».
• Department orchestrator — отдельная программа после Pod Template MVP.
• Path B запрещён: ты не «играешь» роли постов вместо post_executor/PAL.
• create_post — только по явной команде Владельца ИЛИ кнопке UI «Создать Pod».
  Самовольное создание постов из контекста разговора — запрещено.
• Composer/Cursor — со-исполнитель кода в репозитории MSPro-Ltd Corp;
  ты — стратегия, DEC, спеки, SQL-spec, review, Vault. Не пиши Rust из чата
  как замена разработчику — пиши SPEC.
• Переиспользуй v1.0.32: post_executor, dispatcher_brain, DPAPI secrets,
  departments/posts, external gateway :8899, Outbox, artifacts approve flow.

ROLLBACK / INSURANCE (обязательно в программе):
• Перед миграцией каждой фазы: git tag agentpod-phase-N-start +
  копия app.db → %APPDATA%/ru.msproltd.corp/backups/pre-agentpod-phase-N.db
• Миграции MVP: forward-only; откат фазы = restore backup + checkout tag
• В DoD каждой фазы: «rollback procedure documented and tested once»

═══════════════════════════════════════════════════════════════
ЗАПРОС: ПРОГРАММА AgentPod (единственный deliverable на review)
═══════════════════════════════════════════════════════════════

Подготовь программу AgentPod с учётом DEC-001…004 и ограничений выше.
Термин везде: «программа» (не «спринт»).

СРОК ПРОГРАММЫ:
• Реалистичная оценка: 8–12 недель (календарных).
• Если суммарно >12 недель — ОБЯЗАТЕЛЬНО:
  (1) таблица: компонент → недели → почему;
  (2) явный список переноса в Phase 2 с обоснованием.
  Без этого программа не принимается.

СТРУКТУРА ПРОГРАММЫ (все разделы обязательны):

───────────────────────────────────────────────────────────────
ФАЗА 0 — Smoke: доказать текущий executor (3–5 дней)
───────────────────────────────────────────────────────────────
Scope: post_executor БЕЗ PAL/СБ. Диагностика цепочки dispatcher → Outbox.

Definition of Done (все пункты):
□ office-manager имеет system_prompt_md > 0 (или test-pod создан)
□ send_to_dispatcher → forward_to_post → post_executor → test.txt в Outbox
□ Артефакт виден в UI Awaiting → approve
□ Запись в dispatcher_decisions + лог post_executor без ERROR

Testing:
□ Manual smoke script (шаги 1-10, ожидаемый результат каждого шага)
□ При наличии — дополнить cargo test для dispatcher forward path

Risks: ≥3 с риском (вероятность/impact) + митигация

Rollback: tag agentpod-phase-0-start + backup app.db

───────────────────────────────────────────────────────────────
ФАЗА 1 — Фундамент: СБ MVP + PAL v0 + schema БД (оцени недели)
───────────────────────────────────────────────────────────────
Scope:
• Таблицы: provider_registry, provider_health_log, post_runtime,
  post_prompt_history, run_logs (минимум — полный список в SQL-spec)
• СБ UI: список провайдеров, add provider, health dashboard
• PAL: ClaudeCliDriver + QwenHttpDriver + ExternalGateway stub
• Интеграция: post_executor вызывает PAL вместо прямого claude.exe

Definition of Done:
□ 2 рабочих провайдера зарегистрированы и alive в UI СБ
□ Один run через PAL → ответ + run_logs заполнен
□ cargo test: PAL unit + fallback chain
□ SQL migration применена в репо (spec от тебя, кодит Cursor)

Risks: ≥3 + митигации
Rollback: tag + db backup

───────────────────────────────────────────────────────────────
ФАЗА 2 — Компоненты болванки (оцени недели)
───────────────────────────────────────────────────────────────
Scope по компонентам DEC-002 (#2-#8):
• Vault Scaffolder (шаблон из hco-head structure)
• System Prompt Store + валидатор ≤130 строк
• MCP Loader stub
• Task Queue
• Run Logger (полные поля)
• Heartbeat: event-driven wake + 1 cron preset
• Health Monitor + escalate

Definition of Done: измеримый чеклист ПО КАЖДОМУ компоненту (не общими словами)

Testing:
□ Unit-тесты на scaffolder, prompt validator, queue insert/wake
□ Integration: task insert → heartbeat wake → PAL mock run → run_logs

Smoke gate: Фаза 3 не начинается пока Фаза 2 smoke не зелёная

Risks: ≥3 + митигации
Rollback: tag + db backup

───────────────────────────────────────────────────────────────
ФАЗА 3 — Сборка Pod Template + UI + Debug Console (оцени недели)
───────────────────────────────────────────────────────────────
Scope:
• UI «Создать Pod» (форма: slug, dept, ЦКП, metric, tier, model, scaffold vault)
• Объединение компонентов в Pod Template v1
• Debug Console (изолированный чат поста)

Definition of Done:
□ Новый Pod создаётся одной формой + vault scaffold автоматически
□ Debug Console: отправка сообщения → run → ответ/лог виден
□ DEC-004: UI strings нейтральны (чеклист замен)

Testing:
□ E2E smoke: create pod from UI → debug chat → one run logged

Risks: ≥3 + митигации
Rollback: tag + db backup

───────────────────────────────────────────────────────────────
ФАЗА 4 — Validation (оцени недели)
───────────────────────────────────────────────────────────────
Scope:
• office-manager через Template ≤1 ч
• Второй пост (Владелец выбирает slug/dept) ≤30 мин
• Оба: ≥1 боевая задача → артефакт → approve

Definition of Done:
□ ACCEPTANCE DEC-002 выполнен полностью
□ run_logs по обоим постам: ≥1 success each
□ Паттерн AgentPod-v1 сохранён в Vault/02-Patterns/ (≤80 строк)

Risks: ≥3 + митигации

───────────────────────────────────────────────────────────────
ОБЯЗАТЕЛЬНЫЕ ПРИЛОЖЕНИЯ К ПРОГРАММЕ
───────────────────────────────────────────────────────────────

A) SQL-SPEC (CREATE TABLE для всех новых таблиц):
   • Имена полей, типы SQLite, NOT NULL, DEFAULT, FK, индексы
   • Связь с существующими posts, departments
   • Это SPEC для Claude Code — миграции применяет разработчик в репо

B) МАТРИЦА КОМПОНЕНТОВ:
   | Компонент | Rust модуль/файл (предложение) | UI экран | Тесты |

C) ПЕРЕИСПОЛЬЗОВАНИЕ v1.0.32:
   • Что остаётся без изменений
   • Что рефакторится (post_executor → PAL)
   • Что deprecated (если есть)

D) TESTING STRATEGY (по всей программе):
   • Unit: cargo test — какие модули, какие фазы
   • Integration: какие пары компонентов
   • Smoke gates между фазами (таблица: фаза → smoke → критерий перехода)
   • Не требовать % coverage в MVP — требовать critical-path tests

E) PHASE 2 BACKLOG (явный список того, что НЕ в MVP):
   OAuth/subscription СБ, Claude SDK, full MCP docx, ExternalGateway driver,
   Department orchestrator, переименование hmt_* в коде

F) Паттерн Vault/02-Patterns/agentpod-v1.md (≤80 строк) — создай после программы

═══════════════════════════════════════════════════════════════
ПРОЦЕСС ОТВЕТА (сроки и качество)
═══════════════════════════════════════════════════════════════

Черновик + вопросы по существующей инфраструктуре MSPro v1.0.32:
• post_executor.rs, dispatcher_brain.rs, DPAPI secrets, gateway :8899,
  migrations/, Outbox, EditPostKnowledgeModal
→ задай в первые 12 часов СПИСКОМ, не строй догадки в SQL.

Финальная программа (все разделы Фаз 0–4 + приложения A–F):
→ до 60 часов с момента этого сообщения.

Если не успеваешь — в течение 12 часов: обоснование + новый дедлайн.

ЗАПРЕЩЕНО в ответе:
• Path B (ты играешь посты)
• Альтернативные архитектуры «или SDK или CLI» без выбора
• OAuth/subscription в MVP СБ
• Переименование hmt_* таблиц/tools
• Самовольный create_post

После публикации программы — сохрани краткое резюме в decisions-log.md
(ссылка: AgentPod Program v1, дата, статус Draft for Review).

Жду одну версию программы. Без create_post в этом сообщении.
```

---

## Как использовать

1. Откройте этот файл в редакторе.
2. Скопируйте всё **между** тройными обратными кавычками (блок ` ``` ` … ` ``` `).
3. Вставьте одним сообщением в чат Гендира.

## После ответа Гендира

Пришлите полный текст программы в Cursor — разберём фазы, SQL и тикеты Фазы 0.
