# Phase 1 — Backlog

Источник: вскрыто при Day 2-3 Phase 0 AgentPod (2026-05-23). Формат: ID / описание / приоритет / оценка / пререквизиты.

## BL-P1-001 — UI: Dispatcher Processing фильтр дублирует Awaiting
- **Описание:** фильтр вкладки Processing (`src/components/views/Dispatcher.tsx`) включает задачи с `outbox_path != null`, которые уже показаны во вкладке Awaiting → одна задача висит в двух вкладках, сбивает UX. Фикс: Processing-фильтр должен исключать `outbox_path != null` (Awaiting и Processing — взаимоисключающие).
- **Приоритет:** Low (косметика, не блокер).
- **Оценка:** ~0.5 ч (1 условие фильтра + проверка в UI).
- **Пререквизиты:** нет. Подходит для Phase 0 final cleanup или Phase 1.

## BL-P1-002 — Operational: нет auto-cleanup зависших awaiting-задач >7 дней
- **Описание:** `task-5ca1df7f` висит с 21.05 (escalation на docx при недоступном MCP). Нет процесса очистки awaiting-задач старше 7 дней → operational gap, очередь захламляется stuck-задачами.
- **Приоритет:** Medium (operational, эффект растёт со временем).
- **Оценка:** ~3-5 ч (startup-sweep или cron: awaiting >7 дней → auto-fail/archive + запись в decisions/log; индикация в UI).
- **Пререквизиты:** решить политику (auto-fail vs notify-only); логично объединить с Health Monitor / Heartbeat (DEC-002 #7/#8, Phase 2).

## BL-P1-003 — resolve_target_slug: substring false match для коротких слагов
- **Описание:** функция использует наивный substring-поиск (`haystack.contains(&slug)`); короткий slug (например "a") может ложно совпасть в любом слове, содержащем эту букву (например "matches"). Вскрыто на Day 5 при написании теста `resolve_target_slug_missing`.
- **Приоритет:** Low (срабатывает только при очень коротких слагах; в production маловероятно).
- **Оценка:** ~2 ч (word-boundary / exact-token match + тесты + проверка всех call-sites).
- **Пререквизиты:** согласовать политику matching (substring vs word vs exact); возможно объединить с relevance-scoring при нескольких кандидатах.

## BL-P1-004 — Workflow Гендира: incremental save в Vault
- **Описание:** Гендир думает 5+ часов → сохраняет результат в КОНЦЕ → при `claude CLI timeout` теряет всю работу сессии (подтверждено 2026-05-24: за 5+ ч сессии vault_ops_log = 0 записей Гендира). Решение: сохранять прогресс пошагово через `write_vault_file` после каждого крупного раздела (drivers / risks / DoD). Гарантия: даже при timeout Claude в середине итерации — частичный результат на диске.
- **Приоритет:** Medium.
- **Оценка:** ~1-2 ч (правка системного промпта/инструкции Гендира + проверка).
- **Пререквизиты:** нет (фикс таймаута 360 уже снижает частоту timeout, но не устраняет потерю при срабатывании).

## BL-P1-005 — Дробить задачи Гендира на 1-2 раздела
- **Описание:** крупная итерация (напр. 8 разделов SPEC) велика для одной Claude-сессии при текущих таймаутах → выше шанс timeout до сохранения. Стратегия: давать Гендиру 1-2 раздела + явная команда сохранить после каждого («пиши drivers+traits, сохрани; потом UI wireframes, сохрани»).
- **Приоритет:** Low.
- **Оценка:** процессная практика (правка workflow, не код).
- **Пререквизиты:** синергия с BL-P1-004 (incremental save).

## BL-P1-006 — Repo-wide clippy + fmt cleanup (`cargo clippy --all-targets -- -D warnings`, `cargo fmt --check`)
- **Описание:** вскрыто на Iteration B Срез 1 (2026-05-31). Гейт `cargo clippy --all-targets -- -D warnings` **никогда раньше не гонялся** в проекте → накопилось **42 предсуществующих legacy-warning** в ~12 файлах: `commands/{chat,tool_calls,dispatcher,dispatcher_brain,artifacts,claude_bridge,ping,posts}.rs`, `external_agent/{gateway,handlers}.rs`, `com_server/dispatch.rs`, `vault_ops.rs`, `vault.rs`, `updater/check.rs`, `context_assembler.rs`, `post_executor.rs:686` (`cleanup_orphan_post_processes` stub 11B-1). Категории: unused imports (~5), doc-list-overindent (~12), too-many-args 8-12/7 (~6), never-used/never-read fields & fns (~10), useless format!/conversion/cast (~5), manual split_once, sort_by_key, large-Err-variant, complex-type.
- **Важно:** код Iteration B (`pal/`, `run_logger.rs`, `run_via_pal`) — **clippy-чистый** (verified `clippy_mine.txt` = 0 hits). Все 42 — legacy, не из этого среза.
- **Приоритет:** Medium (нужно для CI-гейта `-D warnings`; не блокер runtime — build/test зелёные).
- **Оценка:** ~3-5 ч (быстрые wins — unused imports/doc-overindent ~20 шт за час; `#[allow(clippy::too_many_arguments)]` на legacy-функции; never-used → `#[allow(dead_code)]` или удаление; рефакторинг complex-type/large-Err осторожно).
- **Пререквизиты:** отдельная chore-задача, НЕ смешивать с feature-срезами (риск регрессии в legacy). После cleanup — добавить `cargo clippy --all-targets -- -D warnings` в CI/pre-flight gate.
- **Дополнение (Срез 2, 2026-06-01):** `cargo fmt --check` тоже **никогда не гонялся** — флагует ~150 мест по всему крейту (build.rs, chat.rs, dispatcher*, vault*, tool_calls, posts, hmt, updater, context_assembler, external_agent...). Это тот же systemic gap. cleanup-задача должна включать `cargo fmt` всего крейта (один `cargo fmt` без `--check` → авто-формат) + добавить `cargo fmt --check` в gate. Файлы Среза 1-2 (pal/*, run_logger) уже fmt-clean. Делать ОДНИМ chore-коммитом на весь крейт, не точечно.

## BL-P1-007 — `run_logs.raw_output` redaction (секреты/PII) ⚠️ приоритет к Срезу 2
- **Описание:** замечание Cursor (review Срез 1). `run_logger` пишет `raw_output` = полный stdout/ответ модели (truncate ≤64KB уже есть, R-T-015). Но **контент не редактируется** — если в ответе окажется API-ключ, токен, путь с именем пользователя или PII, оно осядет в БД в открытом виде. На Срезе 1 (ClaudeCli) риск ниже (usage=0, ответ = текст агента), но на **Срезе 2 (Qwen HTTP)** через `raw_output` могут пройти echo промпта/системного контекста → выше шанс утечки.
- **Приоритет:** **Medium-High** (поднят Владельцем; реализовать ДО включения Qwen на Срезе 2).
- **Оценка:** ~2-3 ч. Redaction-функция в `run_logger` перед INSERT: regex-маски (`sk-[A-Za-z0-9]+`, `ghp_…`, `Bearer\s+\S+`, `MSPRO_TOKEN=\S+`, абсолютные пути `C:\Users\<name>` → `C:\Users\<redacted>`), либо опция `raw_output_enabled: bool` (default off в prod) + хранить только при явном debug-флаге.
- **Пререквизиты:** согласовать политику (маскировать vs не писать raw в prod). Связь с R-T-015 (truncation) и R-T-012 (секреты только DPAPI, не в логах).

## BL-P1-008 — `post_executor.rs:89` `s.data.lock().unwrap()` → graceful
- **Описание:** замечание Cursor. `settings_snapshot = s.data.lock().unwrap().clone()` (`post_executor.rs:89`) — `unwrap()` на `Mutex` panic-нет при poisoned lock (если другой поток паниковал держа этот lock). В fire-and-forget task это уронит async-task (не весь app, но задача молча умрёт). Заменить на `match lock() { Ok(g)=>g.clone(), Err(poisoned)=>poisoned.into_inner().clone() }` или `unwrap_or_else(|e| e.into_inner())` — graceful recovery poisoned-lock.
- **Приоритет:** Low (poisoned mutex редок; SettingsStore lock короткий, без await внутри). Но инвариант «ноль unwrap в командах/драйверах» — это **legacy-код 11B-1**, не из моего среза; чистка уместна вместе с BL-P1-006 или точечно.
- **Оценка:** ~15 мин.
- **Пререквизиты:** нет.

## BL-P1-009 — Legacy timeout re-wait в `run_via_pal` / post_executor
- **Описание:** замечание Cursor. На PAL-пути outer timeout владеет `orchestrator::pal_invoke` (`tokio::time::timeout`, Tier::T1=600). Legacy-путь оборачивает `child.wait()` в свой `timeout(post_executor_timeout_sec)`. Сейчас оба пути взаимоисключающие через `pal_enabled` (нет двойного timeout на одной задаче). Замечание — на будущее: при срезах 2-3 (fallback chain — несколько invoke на задачу) убедиться что нет вложенного/повторного timeout-wait (outer от orchestrator на каждый attempt, без дополнительного wrap в post_executor). Сейчас НЕ баг (single invoke), но зафиксировать чтобы не появился double-wait на fallback.
- **Приоритет:** Low (не активен на Срезе 1; watch на Срезе 2).
- **Оценка:** ~30 мин проверка + тест на Срезе 2.
- **Пререквизиты:** Срез 2 (fallback chain).

## BL-P1-010 — Синхронизировать default Claude-модель (settings vs seed) ⚠️
- **Описание:** вскрыто на runtime-smoke Захода B (2026-06-01). В `run_logs` `model_used=claude-opus-4-7` (из `settings.claude_cli_model`), а сид `provider_registry.claude_cli.default_model='opus'` (алиас always-latest). Расхождение: пост-агенты сейчас гоняются на **захардкоженной устаревшей** строке `claude-opus-4-7` (Opus 4.7), а не на актуальном Opus через алиас `opus` (4.8). Причина — `run_via_pal` берёт `model` из `post_executor` lookup (`preferred_model || settings.claude_cli_model`), а не из `provider_registry.default_model`. На Срезе 3 (post_runtime) модель должна резолвиться из registry/post_runtime, а не из legacy `settings.claude_cli_model`.
- **Приоритет:** Medium (агенты работают на устаревшей модели; не блокер — 4.7 рабочая, но не последняя).
- **Оценка:** ~1-2 ч. Варианты: (а) обновить `settings.claude_cli_model` → `opus` (быстрый фикс, везде где legacy читает); (б) на Срезе 3 резолвить модель из `provider_registry`/`post_runtime` (правильный путь — единый источник). Также `dispatcher_claude_model=claude-opus-4-7` и `default_claude_cli_model()` в settings/mod.rs — те же кандидаты на `opus`.
- **Пререквизиты:** согласовать политику «алиас vs полное имя» (алиас = always-latest но непредсказуемо при смене Anthropic дефолта; полное имя = воспроизводимо но устаревает). Связь со Срезом 3 (post_runtime model resolution).

---

## Срез 2 backlog (Cursor APPROVE-WITH-FIXES, 2026-06-01)

## BL-P1-011 — Glued-secret >200 символов: теоретическая утечка хвоста
- **Описание:** redaction token-маски имеют кэп длины `{20,200}` (`run_logger.rs` REDACTIONS). Если секрет слипся с текстом БЕЗ разделителя И длиннее 200 символов — regex замаскирует первые 200, хвост секрета >200 утечёт в `run_logs.raw_output`. Реальные ключи короче (Anthropic ~108, OpenAI ~164, GitHub ≤93) → не срабатывает на настоящих ключах; краевой случай (искусственно длинный токен без пробелов). `\b`/lookahead невозможны (regex crate без lookahead).
- **Приоритет:** **Низкий** (краевой случай; реальные ключи < 200, в проде окружены разделителями).
- **Оценка:** ~1 ч. Варианты: поднять кэп до 500 (покрыть экзотику), ИЛИ двухпроходный redact (split по non-alphanumeric перед маской), ИЛИ принять как documented limitation.
- **Пререквизиты:** нет. Документировано здесь — не теряем.

## BL-P1-012 — model_used при Err цепочки = settings, не фактический провайдер
- **Описание:** Cursor (Срез 2). В `run_via_pal` (`post_executor.rs:~612`) при ПРОВАЛЕ всей chain `model_used` пишется = `model` (из settings/lookup), а `provider_id` берётся правильно (id последнего пробованного провайдера через `outcome.attempt_idx`). На Err-ветке `model_used` не отражает фактическую модель провайдера, на котором случился последний провал (если fallback был на qwen — модель в логе всё равно claude-модель). На success-ветке всё верно (из `resp.model_used`). Косметика аудита run_logs при полном провале.
- **Приоритет:** Низкий (только Err-ветка полного провала chain; success корректен).
- **Оценка:** ~30 мин. Резолвить `model_used` из `chain[attempt_idx]` default_model на Err-ветке (нужен геттер default_model у драйвера, сейчас приватный).
- **Пререквизиты:** нет. Усилится на Срезе 3 (post_runtime — фактические модели на attempt).

## BL-P1-013 — Per-chunk idle timeout для Qwen-стрима
- **Описание:** Cursor (Срез 2). `QwenHttpDriver` имеет только общий `reqwest` timeout 600с (internal safety) + outer orchestrator timeout (Tier). НЕТ per-chunk idle timeout: если Ollama завис между SSE-чанками (отдал часть, потом тишина) — стрим висит до общего 600с, а не реагирует на «нет данных N секунд». Боевой `qwen_bridge.rs` тоже без idle-timeout, но там есть AtomicBool cancel от UI. В PAL cancel = drop future (orchestrator timeout). Idle timeout сделал бы реакцию быстрее при зависшем чанке.
- **Приоритет:** Низкий-Средний (зависший Ollama редок; общий timeout страхует, но грубо).
- **Оценка:** ~1-2 ч. `tokio::time::timeout` на каждый `byte_stream.next()` (напр. 30с idle) → при превышении `ProviderError::Timeout`/`Network`. Тест на mock-Ollama с паузой между чанками.
- **Пререквизиты:** связь с wiremock integration (BL-P1-014) — idle-timeout тестируется тем же mock-сервером.

## BL-P1-014 — wiremock integration для QwenHttpDriver (сетевой контракт НЕ закрыт)
- **Описание:** **вердикт Cursor (Срез 2): сетевой слой Qwen НЕ считается проверенным unit-тестами.** unit покрывают pure-функции (`parse_sse_buffer`/`map_http_error`/`build_request_body`), но реальный reqwest-flow (POST, SSE bytes_stream, header, JSON serialize, HTTP error path) — НЕ протестирован. wiremock integration ОБЯЗАТЕЛЕН перед опорой на Qwen-fallback в бою.
- **Приоритет:** **Средний** (сетевой контракт открыт; до live-smoke / wiremock Qwen-fallback нельзя считать боевым).
- **Оценка:** ~3-4 ч. `wiremock = "0.6"` в `[dev-dependencies]`; тесты: happy SSE-stream, HTTP 404/500/429 → правильный ProviderError, drop-при-timeout, idle между чанками (с BL-P1-013). По Qwen IMPL-REF §8.2.
- **Пререквизиты:** нет. **Альтернатива/дополнение — live-smoke Qwen** (реальный Ollama + ребилд 1.0.35): закрывает сеть end-to-end, но wiremock даёт воспроизводимость в CI. Оба желательны; live-smoke — минимум перед prod-опорой на fallback.

---

## Срез 1.5 backlog

## BL-P1-015 — Реальная изоляция пост-агентов (Phase 2) ⚠️
- **Описание:** заведено при Срезе 1.5 (flag harden). Phase 1 заменил `--dangerously-skip-permissions` (bypass) на `acceptEdits` + python-whitelist + deny (`pal::permission_flags()`, оба пути). Это **harden, НЕ полная изоляция**: (1) whitelist режет *какие бинари* запускаются (только python), но НЕ *что python делает внутри* — скрипт сам может `os.remove`/сетевой запрос/чтение чужих файлов; (2) cwd ≠ sandbox — Bash может `cd ..` за пределы `Outbox/<task_id>/`; (3) python-процесс наследует права Владельца. Спайк S5 подтвердил что whitelist истинный (не-python не запускается), но это не FS/network-jail.
- **Приоритет:** Средний (Phase 2; Phase 1 harden закрыл очевидный вектор — произвольные shell-команды/деструктив).
- **Оценка:** R&D. Кандидаты: (а) запуск пост-агента в отдельном Windows-процессе с ограниченным токеном (job object + restricted SID); (б) контейнер/WSL без сети + bind-mount только Outbox; (в) python-sandbox (RestrictedPython / seccomp-аналог) — слабее; (г) FS-jail через junction + ACL на Outbox. Выбор — отдельное investigation Phase 2.
- **Пререквизиты:** Phase 2. Связь с R-T-002/011 (Job Object 11B-bis — частично пересекается).

---

## 1.0.35 backlog

## BL-P1-016 — Cancel-кнопка: краевой случай cancel Ok + fail_task Err (UX)
- **Описание:** заведено при verify Cursor cancel-кнопки (1.0.35, `TaskRow.tsx::quickCancel`). Связка: `cancel_post_executor` (убивает PID) → `fail_task` (уводит из Processing). Если `cancel_post_executor` вернул Ok (процесс убит), но последующий `fail_task` бросил Err — `catch` делает только `alert(String(e))`, и задача **остаётся in_progress в UI**, хотя процесс уже мёртв (рассинхрон: процесса нет, статус «выполняется»). Не блокер (fail_task — локальный SQL UPDATE, Err редок), но UX-дыра.
- **Приоритет:** Low (UX-полировка; краевой случай — fail_task роняется редко).
- **Оценка:** ~30 мин. Варианты: (а) при Err после успешного kill — retry `fail_task` 1 раз / показать явное «процесс убит, но статус не обновлён — обнови вручную»; (б) перенести связку cancel+fail в один Rust-метод `cancel_and_fail` (атомарно, БД-транзакция) — чище, но Rust-правка; (в) оптимистично обновить локальный статус на failed до подтверждения. Предпочтение — (б) при следующем касании post_executor.
- **Live-находка (1.0.35, Заход C):** ⏹ висит на ОБЕИХ in_progress-задачах. PID пост-агента зарегистрирован только под **executing**-задачей (`→office-manager`). Отмена **routing**-задачи (`ceo→dispatcher`) → `cancel_post_executor`=false → «process **not running**» (бесполезно + путает). Отмена executing-задачи → «process **killed**» (I1 работает). **Фикс:** показывать ⏹ только где есть зарегистрированный PID, ИЛИ каскадить kill на дочерние executing-задачи.
- **Пререквизиты:** нет.

## BL-P1-017 — Устойчивость рефайнинга Диспетчера к elaborate-промптам ⚠️
- **Описание:** воспроизведено дважды live (1.0.35): задачи `task-29526d9e` (тяжёлые маркеры `«»`/`::`/диктовка) и `task-8ab5c4a4` («КРИТИЧНО: не оборачивать в кавычки», `(python-docx / Word COM / иное)`) → `ceo→dispatcher` **failed** с `dispatcher rejected: forward_to_post: refined_prompt missing` (`dispatcher_brain.rs:528`). Мозг Диспетчера выдал tool_call `forward_to_post`, но парсер не извлёк `refined_prompt` — спецсимволы/кавычки/мета-инструкции в elaborate-промпте ломают JSON tool_call'а. Простые промпты (Заход A retry, C) проходят. Задача умирает ДО office-manager, пост-агент не спавнится.
- **Приоритет:** **Средний-высокий** (любой «дотошный» промпт Гендира с кавычками/маркерами роняет диспетч — реальный блокер боевого использования).
- **Оценка:** ~2-4 ч. Кандидаты: (а) укрепить извлечение `refined_prompt` / парсинг tool_call к спецсимволам (salvage частичного объекта); (б) в системном промпте Диспетчера явно требовать экранирование/упрощение `refined_prompt`; (в) при `refined_prompt missing` — retry рефайнинга с упрощённым промптом вместо fail; (г) fallback: если рефайнинг не дал refined_prompt — форвардить raw_prompt как есть.
- **Пререквизиты:** нет. Источник истины ошибки — `dispatcher_brain.rs::execute_forward` (нет live app-лога с сырым выводом мозга — желательно добавить логирование сырого tool_call при этой ошибке).

## BL-P1-018 — Доставка результата: Скачать + артефакт в чате Гендира ⭐
- **Описание:** product-замечание Владельца (1.0.35). Сейчас результат пост-агента (.docx) лежит в `Outbox/<task_id>/` и доступен ТОЛЬКО через Диспетчер → панель Артефактов → «📂 Открыть» (открывает в Word). Нет (а) кнопки **⬇ Скачать** (save-as), (б) **доставки в чат Гендира** — Владелец дал задачу Гендиру, логично ждать результат ОТ Гендира в том же чате («готово, вот файл»), а не идти в отдельную вкладку.
- **Приоритет:** **Высокий** (без доставки в чат цепочка «полуфабрикат» для реального юзера).
- **Оценка:** (а) ⬇ Скачать в `ArtifactsPanel.tsx` + backend save-as команда — ~1 ч. (б) Артефакт в чате CEO: связка `chat → dispatch task → artifact` + рендер карточки результата (имя + Открыть/Скачать) в ленте `CeoChat.tsx`, событие при approve/completion — ~полдня.
- **Пререквизиты:** нет. Переиспользовать `open_artifact_in_default_app`, `list_task_artifacts`, `task_artifacts` (task↔artifact уже связаны через `task_id`/`parent_task_id`).

## BL-P1-019 — Усилить source-guard теста raw_brain_response (🟡 Cursor, BL-P1-017 follow-up)
- **Описание:** тест `raw_brain_response_routes_through_redaction` сейчас проверяет ОТСУТСТВИЕ старого голого char-капа (`chars().take(64…`). Этого мало: ловит только известный обход, а не любой. Усилить — проверять НАЛИЧИЕ вызова `prepare_sensitive_log` в `save_raw_brain_response` (позитивный guard), чтобы любой новый способ записать сырой ответ мимо redact ломал тест.
- **Приоритет:** Low (защита уже работает — Cursor APPROVE; это усиление регресс-сетки).
- **Оценка:** ~15 мин. ⚠️ Аккуратно с self-match (тест и проверяемый код в одном файле через `include_str!`) — позитивный assert на `prepare_sensitive_log` сматчит сам себя. Варианты: (а) вынести `save_raw_brain_response` в отдельный модуль/файл и грепать его; (б) грепать срез исходника между маркер-комментариями функции; (в) собрать искомую строку из частей и считать, что ≥2 вхождения (тело + тест).
- **Пререквизиты:** нет. Сделать **заодно со следующей правкой** `dispatcher_brain.rs`/`run_logger.rs` (директива Владельца — не отдельным заходом).
