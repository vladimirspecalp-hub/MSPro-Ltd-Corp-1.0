# MSPro CEO Brain v1.0.30 — единый план (слияние)

**Источники:**
- Старый план Claude: `C:\Users\1\.claude\plans\wild-launching-nygaard.md` (v1.0.29, HMT knowledge)
- Новый план Board + ревью: память в БД, навигатор, tools = UI

**Версия релиза:** v1.0.30 (не v1.0.29 — scope расширен)

**Репозиторий:** `c:\CODE\MSPro-Ltd Corp 1.0\`

---

## A. Ревью старого плана Claude (wild-launching-nygaard)

### Что в старом плане верно — оставить

| Пункт | Статус |
|-------|--------|
| Заменить `HMT_PREAMBLE` на компактное ядро | ✅ Да |
| `Vault/01-HMT-Knowledge/` с topic-файлами | ✅ Да |
| Seed при первом запуске, не перезаписывать если Владелец редактировал | ✅ Да |
| `02-Patterns`, `04-Wins` — уже есть | ✅ Без изменений логики |
| Live оргструктура + HMT-engine из SQLite в prompt | ✅ Да |
| MCP sequential-thinking — **не в этом релизе** (Phase 12) | ✅ Честно отложить |
| Vault только в `%APPDATA%\ru.msproltd.corp\Vault\` | ✅ Да |

### Что в старом плане ошибочно — исправить в v1.0.30

| Проблема | Было в wild-launching | Стало в v1.0.30 |
|----------|----------------------|-----------------|
| **Смешение навигатора и учебника** | `CEO_CORE_PROMPT` ~130 строк **с таблицей 8 отделений, формулами всех состояний, СФП, стратпланом** (блоки 2–3, 7–8) | `CEO_CORE` **≤130 строк БЕЗ формул и таблиц** — только роль, правила, **маршрутизация** |
| **Дублирование** | То же в prompt + 9 файлов + `read_vault_context` грузит все 9 в каждый turn | В prompt: INDEX (≤2KB) + 02/04/05; полные темы — **tool `read_hmt_topic`** |
| **Vault 32KB все знания** | `VAULT_BLOCK_BYTES` 16→32K, **все** `01-HMT-Knowledge` в system prompt | Динамический бюджет vault: **остаток после fixed**; 01-HMT **не** целиком каждый turn |
| **Stub seed 200–400 слов** | 9 generic stub-файлов | Seed: stub + **`import_hmt_knowledge_from_dir`** из `Анализ/` (или ручное наполнение) |
| **Память чата** | Не описана | **Фаза 1** — главный UX-фикс |
| **Управление приложением** | Не описано | **Фаза 3** — расширение tools |
| **Опечатка** | «≤130 **слов**» в заголовке, в тексте «строк» | Везде: **≤130 строк** (~2–3K токенов), не слов |

### Итог ревью

Старый план **на 60% верный** (Vault 01-HMT, убрать HMT_PREAMBLE, MCP позже). **Критическая ошибка:** назван «навигатор», по факту — **второй учебник в prompt**. v1.0.30 исправляет это явно.

---

## B. Архитектура v1.0.30 (три столпа)

```
┌─────────────────────────────────────────────────────────────┐
│ SQLite                                                       │
│  chat_messages              — вся переписка (навсегда)       │
│  chat_session_summaries     — сжатие при overflow окна       │
│  departments, posts, hmt, dispatcher_tasks, …                │
└─────────────────────────────────────────────────────────────┘
         │ UI (Владелец)              │ context_assembler
         └──────────► Tauri commands ◄────────── tool_call
                              │
              ┌───────────────┴───────────────┐
              ▼                               ▼
         Claude CLI                      Qwen 32k
    (claude_context_tokens)          (жёсткий cap)
```

### Принципы (P1–P8)

1. **Переписка** — только `chat_messages`; Vault не дублирует чат.
2. **Одно окно на turn** = CEO_CORE + live БД + vault-срез + tools + история из БД + user message (**один лимит**).
3. **Claude:** максимум истории из БД, пока влезает в `claude_context_tokens` (default 200k, настраиваемо до 1M). Подписка ≠ экономия; лимит = физика окна.
4. **Qwen:** тот же assembler, cap 32_000.
5. **Overflow:** старое остаётся в БД; в prompt — хвост + `chat_session_summaries`; tool `search_chat_history`.
6. **Reboot:** продолжение того же чата из БД.
7. **Навигатор vs учебник:** CEO_CORE ≤130 строк; учебник в `01-HMT-Knowledge/` + `read_hmt_topic`.
8. **Действия:** tool_call = те же Rust-команды, что UI.

### Сценарий приёмки (promotion-отдел)

- Вчера ~900k токенов суммарно → всё в `chat_messages`.
- ПК выключен → данные на месте.
- Сегодня новое сообщение → assembler: fixed + history (max из БД) + user ≤ лимита Claude.
- Если > лимита → summary из `chat_session_summaries` + свежий хвост; начало вчера в БД, recall через `search_chat_history`.

---

## C. Фазы реализации

### Фаза 1 — Память диалога (НОВОЕ — добавить к старому плану)

**Файлы:** `context_assembler.rs` (новый), `chat.rs`, `qwen_bridge.rs`, `claude_bridge.rs`, `settings/mod.rs`, миграция SQLite.

**Settings:**
```text
claude_context_tokens: 200_000   // user may set 1_000_000
qwen_context_tokens:   32_000
chat_history_turns:    100       // fallback cap only
```

**`pack_history_from_db`:**
1. `fixed_len` = CEO_CORE + org + HMT live + vault slice + TOOLS
2. `budget_history = model_max - fixed_len - user - 5% reserve`
3. SELECT history DESC, filter ⚡/⚠️/⏹, pack from end
4. Overflow → update `chat_session_summaries` (SQLite, **не** Vault/sessions/)

**Tool `search_chat_history`:** `{ query, limit? }` → LIKE по content.

**Уже в коде (v1.0.28+):** `fetch_chat_history`, injection в CLI/Qwen, `list_chat_history` subquery — **не ломать**, заменить логику лимита на token pack.

---

### Фаза 2 — Навигатор + Vault HMT (ИСПРАВЛЕННАЯ версия wild-launching)

#### C.2.1 vault.rs

```text
Vault/
  00-INDEX.md                 ← НОВОЕ: карта тем (≤2KB в prompt)
  01-HMT-Knowledge/           ← из старого плана (9 файлов)
  02-Patterns/                ← было
  03-Bugs/                    ← НОВОЕ
  04-Wins/                    ← было
  05-Lessons/                 ← НОВОЕ
  posts/<slug>/...
```

- `KNOWLEDGE_DIR = "01-HMT-Knowledge"`
- `ensure_vault_dirs` создаёт 00, 01, 03, 05
- `read_vault_context`: **INDEX + 02 + 04 + 05** (НЕ все 9 файлов 01)
- `VAULT_BLOCK_BYTES`: `min(24_000, remaining_budget * 0.25)` — не фикс 32K всех знаний
- `seed_hmt_knowledge` — только если пусто; **не** перезаписывать
- `import_hmt_knowledge_from_dir(path)` — опционально v1.0.30

**9 файлов** (имена из старого плана, контент — из `Анализ/` или stub):

| Файл | Тема для read_hmt_topic |
|------|-------------------------|
| 01-оргсхема-8-отделений.md | departments |
| 02-цкп.md | ckp |
| 03-обязанности-владельца.md | owner-duties |
| 04-обязанности-руководителя.md | manager-duties |
| 05-статистики.md | statistics |
| 06-формулы-состояний.md | formulas-states |
| 07-координация.md | coordination |
| 08-стратегическое-планирование.md | strategic-plan |
| 09-финансовое-планирование.md | financial-plan |

#### C.2.2 CEO_CORE (ЗАМЕНА секции 3 старого плана)

**Удалить из prompt:** HMT_PREAMBLE + блоки 2–3, 7–8 старого CEO_CORE.

**CEO_CORE содержит ТОЛЬКО (~≤130 строк):**
- Блок 1: Роль, 8 отделений Hubbard, Hub-and-Spoke
- Блок 2: Таблица маршрутизации (вопрос → куда)
- Блок 3: Правила поведения (опираться на БД, не чини норму, 24–48ч)
- Блок 4: Tools — когда `read_hmt_topic`, `send_to_dispatcher`, `search_chat_history`

**НЕ содержит:** таблицу 8 отделений с ЦКП, формулы состояний, СФП, стратплан — это в Vault 01.

#### C.2.3 chat.rs

- `HMT_PREAMBLE` → `CEO_CORE` (include from `resources/ceo_core_prompt.md`)
- `build_ceo_system_prompt` → вызывает `context_assembler` для vault slice budget

#### C.2.4 Tool `read_hmt_topic`

```json
{"name":"read_hmt_topic","arguments":{"topic":"formulas-states|departments|..."}}
```
→ read one file ≤8KB, return as tool result / system msg.

---

### Фаза 3 — Гендир управляет изнутри (НОВОЕ — добавить к старому плану)

**Уже есть:** `send_to_dispatcher`, `create_post`, `update_post`, `archive_post`, `save_pattern`, `save_win`, `read_post_knowledge`.

**Добавить:**

| Tool | Backend |
|------|---------|
| `record_statistic` | `hmt::add_statistic_value` |
| `search_chat_history` | Фаза 1 |
| `read_hmt_topic` | Фаза 2 |
| `list_dispatcher_tasks` | existing list command |

---

## D. Что явно ДОБАВИТЬ в старый план Claude

Список для copy-paste в задачу Claude Code — «сверх wild-launching»:

1. **Фаза 1 целиком** — `context_assembler.rs`, `chat_session_summaries`, `pack_history_from_db`, settings `claude_context_tokens` / `qwen_context_tokens`, tool `search_chat_history`.
2. **Исправить секцию 3** — CEO_CORE = навигатор **без** формул/таблиц/СФП (убрать блоки 2–3, 7–8 старого плана).
3. **Исправить vault read** — не грузить все 9 файлов 01-HMT каждый turn; добавить `00-INDEX.md`, `03-Bugs`, `05-Lessons`.
4. **Не поднимать слепо VAULT до 32K** — динамический бюджет от остатка окна.
5. **Tool `read_hmt_topic`** вместо dump всех знаний в prompt.
6. **Фаза 3** — `record_statistic`, `list_dispatcher_tasks`.
7. **Версия v1.0.30**, не v1.0.29.
8. **Нет** `Vault/sessions/` для чата — только SQLite.

---

## E. Что УБРАТЬ из старого плана

- Секция «Блок 2: 8 отделений — таблица в промте»
- Секция «Блок 3: Формулы состояний в промте»
- Секция «Блок 7–8: стратплан и СФП в промте»
- `append_section` всех `01-HMT-Knowledge` в `read_vault_context` без лимита
- Фиксированный `VAULT_BLOCK_BYTES = 32_000` для всех знаний
- v1.0.29-only scope (заменить на v1.0.30)

---

## F. Порядок коммитов

1. `feat(context): DB-backed history pack + session summaries + search_chat_history`
2. `feat(vault): CEO_CORE navigator + 01-HMT + read_hmt_topic + INDEX`
3. `feat(tools): record_statistic + list_dispatcher_tasks`
4. version → **v1.0.30**, Sidebar «CEO Brain · DB memory · HMT navigator»

---

## G. Вне scope v1.0.30

- MCP bridge / sequential-thinking (Phase 12)
- Claude API persistent session (вместо `--print`)
- Симлинки gendir / YandexDisk в runtime
- Полная загрузка 8000 строк книг в каждый turn

---

## H. Чеклист приёмки (для ревью Board)

### Память
- [ ] Reboot: вчерашний чат promotion виден и учитывается
- [ ] Claude: log `total ≤ claude_context_tokens`
- [ ] Qwen fallback: no context overflow
- [ ] `search_chat_history("promotion")` находит старые реплики

### Навигатор + Vault
- [ ] System prompt без полных формул HMT
- [ ] `read_hmt_topic("formulas-states")` → шаги из 06-файла
- [ ] `00-INDEX.md` в prompt, не все 9 файлов
- [ ] Редактирование .md в проводнике → CEO видит на следующем turn

### Tools
- [ ] `record_statistic` → UI HMT обновился
- [ ] `create_post` только по явной просьбе (как было)

### Build
- [ ] `cargo test` green
- [ ] `pnpm tsc --noEmit` green

---

## I. Текущее состояние кодовой базы

| Готово | По плану |
|--------|----------|
| `HMT_PREAMBLE`, vault 02/04 | → CEO_CORE + 01-HMT + INDEX |
| `fetch_chat_history` (~20 turns) | → `pack_history_from_db` |
| Tools: posts, dispatcher, pattern/win | + statistic, search, read_hmt_topic |
| `list_chat_history` DESC subquery | keep |

---

*Документ для отправки на ревью. Старый план: `wild-launching-nygaard.md` — не удалять, использовать этот файл как актуальный.*
