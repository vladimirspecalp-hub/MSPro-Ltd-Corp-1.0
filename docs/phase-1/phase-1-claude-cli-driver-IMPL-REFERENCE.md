# Phase 1 — ClaudeCliDriver IMPLEMENTATION REFERENCE

- **Version:** v1 (2026-05-24)
- **Author:** Claude Code (skill `mspro-programmer`)
- **Status:** Approved Владельцем (Вариант B — заменяет детальный driver SPEC; Гендир не переписывает driver-спеку)
- **Role:** **основной reference** для будущей имплементации `ClaudeCliDriver` в Phase 1.
- **Relation to other docs:**
  - Контракт trait — `phase-1-pal-trait-spec.md` v2 + правки v3 (за Гендиром).
  - Skeleton от Гендира — `phase-1-claude-cli-driver-spec.md` v1 — **архивный**, к боевой реализации применяется этот файл.
  - Декларативный playbook сборки/релиза — `02-Patterns/rebuild-msi-playbook-v1.0.33.md`.

**Назначение документа:** один источник истины «как реально работает claude.exe spawn в v1.0.33 и как ClaudeCliDriver должен повторить это поведение 1-в-1». Все цифры строк указаны для `src-tauri/src/commands/post_executor.rs` boevoy v1.0.33 (verified read-only 2026-05-24).

---

## §1. Реальный flow v1.0.33 (источник истины)

Функция `run_claude_cli_for_post()` в `post_executor.rs` строки 147-357 (~211 строк с комментариями, ~140 строк чистого кода).

### §1.1. Фактический argv claude.exe (строки 214-232)
```
{settings.claude_cli_path}                          // C:\Users\1\.local\bin\claude.exe
  --print
  --output-format  text
  --agent          mspro-{safe_slug}                 // sanitize_post_slug(post.slug)
  --model          {model}                           // preferred_model | settings.claude_cli_model
  --dangerously-skip-permissions                     // КРИТИЧНО — без него Write/Edit/Bash молча отказывают
```
- `--dangerously-skip-permissions` безопасен потому что **cwd жёстко sandbox-нут** в `Outbox/<task_id>/` (`current_dir`).
- На Windows перед `cmd.arg(...)` цепочкой вызывается `hide_console(&mut cmd)` (см. `claude_bridge.rs:42`, `CREATE_NO_WINDOW = 0x08000000`). Без него `claude.exe` откроет видимое окно cmd.

### §1.2. Окружение subprocess и pipes (строки 227-232, 256-261)
| Параметр | Значение |
|---|---|
| `current_dir` | `Outbox/<task_id>/` (sandbox-механизм; **не** `--workspace` флаг) |
| `env(MSPRO_TASK_ID)` | task_id (для opt. cleanup orphan-ов) |
| `stdin` | `piped`, plain text **`refined_prompt`** (без JSON-обёртки) |
| `stdout` | `piped`, plain text «говорильня» агента — для логики **не парсится** |
| `stderr` | `piped`, читается только при `exit != 0` (последние 8 строк → reason) |
| `kill_on_drop` | `true` (anti-orphan защита при `drop(child)`) |

### §1.3. agent.md frontmatter (`ensure_post_agent_md`, строки 364-392)
Путь: `~/.claude/agents/mspro-{safe_slug}.md`. Идемпотент: пишется только если содержимое изменилось (избегаем mtime updates и race с CLI reload).

```yaml
---
name: mspro-{safe_slug}
description: MSPro-Ltd Corp пост-агент (slug={slug}). Получает task через stdin, создаёт артефакты в текущей рабочей директории (Outbox sandbox).
tools: [Read, Write, Edit, Bash]
model: {model}
---

{system_prompt_md из posts table}
```
- `tools: [Read, Write, Edit, Bash]` — **критично отличие** от CEO/Dispatcher (там `tools: []`). Пост-агент ДОЛЖЕН физически писать файлы.
- `system_prompt` владельца живёт в `agent.md.body`, **НЕ** в stdin payload.

### §1.4. Model resolution (строки 188-193)
```rust
let model = preferred_model_opt
    .as_deref()
    .map(str::trim)
    .filter(|s| !s.is_empty() && !s.to_lowercase().starts_with("qwen"))
    .map(|s| s.to_string())
    .unwrap_or_else(|| settings.claude_cli_model.clone());
```
- Если `posts.preferred_model = "qwen*"` → используется `settings.claude_cli_model` (по факту Qwen в post_executor v1.0.33 НЕ поддержан; полная поддержка — Phase 1 через PAL + `QwenHttpDriver`).
- Default: `settings.claude_cli_model` (= `claude-opus-4-7` по live settings.json).

### §1.5. Artifacts discovery (строки 211, 301-319) — единственный достоверный механизм
```
// ДО spawn:
let pre_snapshot = snapshot_dir(&task_dir);   // HashMap<rel_path, mtime_secs>

// ... spawn → stdin → wait ...

// ПОСЛЕ exit:
let new_files = diff_dir(&task_dir, &pre_snapshot);  // Vec<String> новых/изменённых rel_path

for rel in &new_files {
    let mime = guess_mime_from_ext(rel);
    artifacts::register_artifact(task_id, rel, mime.as_deref(), &slug, db, vault, app).await
}
```
- Агент **НЕ возвращает** artifacts в stdout. Артефакты — файлы которые он физически создал через Write tool в `cwd = Outbox/<task_id>/`.
- `--output-format text` не содержит структурированных полей `tool_use_results` — поэтому stdout-парсинг для artifacts невозможен.
- `guess_mime_from_ext` мэппит расширение → MIME (md/json/html/docx/xlsx/pptx/pdf/png/jpg/csv).

### §1.6. Timeout (строка 264, КРИТИЧНО для timeout reconciliation)
```rust
let timeout_secs = settings.post_executor_timeout_sec;   // = 600 (ЭТО НЕ claude_cli_timeout_sec=360!)
let result = timeout(Duration::from_secs(timeout_secs), child.wait()).await;
```
- На expiry: `child.kill()`, `exit_code = -2`, `reason = "timeout {timeout_secs}s"`.
- `child.wait()` — ожидаем exit процесса, **не** read stdout (отличие от CEO/Dispatcher flow в `claude_bridge.rs`).

### §1.7. Success / fail logic (строки 322-339)
```
if exit_code == 0 && registered > 0:
    task остаётся in_progress с outbox_path != null
    → UI показывает в «Awaiting» (Владелец approve / reject)
else:
    bump_attempts(task_id)
    fail_task_inner(task_id, reason)
    reason = match:
      exit_code == -2   → "timeout {timeout_secs}s"
      registered == 0 && exit_code == 0 → "claude finished but produced no artifacts"
      _ → "exit={code}; stderr: {stderr_tail}"   // stderr_tail = последние 8 строк
```
Emit `post-executor-finished` event с `{task_id, exit_code, artifacts, elapsed_ms}` (строки 341-349).

### §1.8. Boundary артефактов: orphan protection
- `PostExecutorRegistry { running: Arc<AsyncMutex<HashMap<String, u32>>> }` хранит `task_id → PID` (строка 47).
- Дубль-spawn защита: если task_id уже в running map — `return` тихо (строки 104-112).
- `cancel_post_executor(task_id)` Tauri command: lookup PID → `sysinfo::Process::kill()` (строки 492-519).
- `cleanup_orphan_post_processes()` — Phase 11B-1 stub (no-op); Job Object из `lib.rs::setup` гарантирует kill всех `claude.exe` при выходе MSPro (документация в `post_executor.rs:19-20`).

---

## §2. Integration boundary — что PAL забирает / что остаётся в post_executor

После имплементации `ClaudeCliDriver` функция `run_claude_cli_for_post` **продолжает существовать** и оркестрирует sandbox + artifacts + dispatcher_logs. Меняется только её центральный блок «как именно вызвать LLM».

### §2.1. Уходит в `ClaudeCliDriver::invoke`
| Блок | Строки post_executor v1.0.33 | Объём |
|---|---|---|
| `Command::new` + arg-цепочка (`--print --output-format --agent --model --dangerously-skip-permissions`) | 214-232 | ~18 строк |
| `cmd.spawn()` + `map_err` + spawn-fail recovery (fail_task на спавн-крэше) | 234-246 | ~12 строк |
| `stdin.write_all(refined_prompt)` + `drop(stdin)` | 256-261 | ~5 строк |
| `timeout(child.wait())` + match exit_code (-1/-2/code) | 264-281 | ~17 строк |
| stderr capture + tail (последние 8 строк) | 284-297 | ~13 строк |
| **Итого в driver** | | **~65 строк** |

### §2.2. Остаётся в `post_executor::run_claude_cli_for_post`
| Блок | Строки v1.0.33 | Объём |
|---|---|---|
| Tauri state lookup (`WritePool`, `VaultState`, `SettingsStore`, `PostExecutorRegistry`) | 73-101 | ~30 строк |
| Registry duplicate guard | 104-112 | ~8 строк |
| `posts` row lookup + `system_prompt` validation + model resolve | 162-193 | ~32 строки |
| `sanitize_post_slug` + `ensure_post_agent_md` (~3 строки вызов + 30 строк helper в этом же файле) | 196-198 + 364-392 | ~3 + 30 |
| `outbox::task_outbox_dir` (mkdir идемпотентно) + spawn log | 200-208 | ~9 строк |
| `snapshot_dir(&task_dir)` → `pre_snapshot` | 211 | ~1 строка |
| Registry PID insert / remove | 248-253, 128-131 | ~9 строк |
| **NEW (после PAL):** build `ProviderRequest` + `pal.invoke()` + match `Result<ProviderResponse, ProviderError>` | — | ~15 строк |
| `diff_dir(&task_dir, &pre_snapshot)` + `register_artifact` loop | 301-319 | ~19 строк |
| `bump_attempts` + `fail_task_inner` + success log | 322-339 | ~17 строк |
| Emit `post-executor-finished` + `Ok(PostExecResult {...})` wrap | 341-356 | ~14 строк |
| **Итого остаётся** | | **~145 строк** |

### §2.3. Net delta
`run_claude_cli_for_post` сегодня ≈ **140 строк бизнес-логики**. После PAL:
- Уйдёт ~65 строк (spawn/wait/stderr capture).
- Придёт ~15 строк (build `ProviderRequest` + `pal.invoke` + match).
- **Net: 140 − 65 + 15 ≈ 90 строк (35% reduction)**.

Это **НЕ** «80 → 5 строк». PAL перенесит spawn-mechanics, но НЕ заберёт sandbox-orchestration / artifacts-discovery / dispatcher_logs bookkeeping — это бизнес-логика post_executor.

---

## §3. ProviderResponse контракт для Phase 1

### §3.1. Поля + значения для ClaudeCliDriver Phase 1 MVP
*(сверено с trait v3 §3.2 `ProviderResponse` — 7 полей)*

| Поле | Phase 1 ClaudeCliDriver значение | Комментарий |
|---|---|---|
| `text: String` | plain text stdout (вся «говорильня» агента) | Логируется в `run_logs.raw_output_debug`. Для бизнес-логики post_executor не используется. |
| `usage: TokenUsage` | **`(0, 0, 0, 0)`** (всегда нули) | `--output-format text` не содержит `input_tokens` / `output_tokens`. **Gap для cost-tracking** — см. §7.2. |
| `latency_ms: u64` | `Instant::now() - started_at` (wall-clock от spawn до exit) | Включает sandbox setup, stdin write, exit. |
| `model_used: String` | `model` который передали в `--model` arg | Может отличаться от `default_model` если `request.model_override.is_some()`. |
| `provider_used: ProviderKind` | **`ProviderKind::ClaudeCli`** | Constant для этого драйвера (новое поле trait v3 — для аудита fallback chain в `run_logs`). |
| `stop_reason: String` | Phase 1: всегда `"end_turn"` (нет structured stop_reason в `--output-format text`) | trait v3 §3.2 явно фиксирует `String` для Phase 1; enum нормализация — Phase 2. |
| `artifacts: Vec<ArtifactRef>` | **`vec![]` (всегда пусто)** | Artifacts discovery — забота post_executor через `diff_dir`. ClaudeCli driver НЕ знает sandbox-семантики. trait v3 §3.2 явно зафиксировал «Phase 1 ВСЕГДА empty». |

### §3.2. ProviderRequest — расширения (УЖЕ В trait v3)
✅ trait v3 §3.1 ввёл все три поля как `Option<...>` с `#[serde(default, skip_serializing_if = "Option::is_none")]`. Никаких правок Гендиру не требуется. Здесь — справка о семантике для имплементации.

```rust
pub struct ProviderRequest {
    // ... базовые поля trait v3 ...

    /// Sandbox-папка для ClaudeCli (cwd subprocess + Outbox для artifacts).
    /// Some(path) — driver запускает в sandbox-режиме.
    /// None — driver работает stateless (Qwen HTTP, ExternalGateway).
    /// ClaudeCliDriver с None → `ProviderError::BadRequest("workspace_path required for ClaudeCli")`.
    pub workspace_path: Option<std::path::PathBuf>,

    /// Slug пост-агента для `--agent mspro-{slug}`.
    /// Some(slug) — driver резолвит agent.md и передаёт в argv.
    /// None — driver работает без --agent (для CEO/Dispatcher unification в Phase 2).
    /// ClaudeCliDriver с None в Phase 1 → `ProviderError::BadRequest("agent_slug required for ClaudeCli")`.
    pub agent_slug: Option<String>,

    /// Per-request override модели. Если None — driver использует свою default model
    /// (из provider_registry, в Phase 1 MVP — `default_model` поля драйвера).
    /// Источник: posts.preferred_model или явная команда оператора (DEC-002 hot-swap).
    pub model_override: Option<String>,
}
```

### §3.3. ProviderError маппинг — для Phase 1 MVP
*(сверено с trait v3 §3.3 — 11 вариантов; `RateLimit`/`AuthFailure`/`Internal` из v2 в v3 переименованы/удалены)*

| stderr содержит | exit_code | ProviderError variant (v3) | `should_fallback()` |
|---|---|---|---|
| `"rate limit"` или `"quota"` | любой | `QuotaExceeded(stderr_line)` *(в v3 `RateLimit` отдельным вариантом нет — оба сценария идут под `QuotaExceeded(String)`)* | true |
| `"not authenticated"` или `"login"` | любой | `Auth(reason)` *(в v2 был `AuthFailure { reason }`; в v3 — tuple `Auth(String)`)* | true |
| `"timed out"` | 124 или любой | `Timeout { timeout_secs: 0 }` *(struct variant, имя сохранено; 0 = внутренний CLI timeout, orchestrator знает свой outer)* | true |
| `"connection"` или `"network"` | любой | `Network(reason)` *(в v3 — tuple `Network(String)`, не struct)* | true |
| `"model not found"` / `"unknown model"` / `"unsupported model"` | любой | `ModelUnavailable(stderr_line)` *(новый вариант v3, нет в v2)* | true |
| `"invalid"` / `"bad request"` / `"unparseable"` | любой | `BadRequest(stderr_line)` *(новый вариант v3, нет в v2)* | **false** |
| прочее | 0 | OK *(если `registered_artifacts == 0` — fail-решение принимает post_executor, не driver: «claude finished but produced no artifacts» уходит в `dispatcher::fail_task_inner`, **не** в ProviderError)* | n/a |
| прочее | -2 (kill после timeout) | Driver НЕ возвращает `Timeout` сам — orchestrator формирует его из `tokio::time::timeout` elapsed *(см. ниже)* | n/a |
| прочее | else | `Server(stderr_tail)` *(серверная ошибка CLI; в v2 был `Internal { reason }`, в v3 такого варианта нет — заменён на `Server(String)` для серверных и `Other(String)` для catch-all)* | true *(`Server` имеет `should_fallback=true`)* |

**Конкретные конструкторы (для копирования в код драйвера):**
```rust
return ProviderError::QuotaExceeded(stderr_first_line.to_string());
return ProviderError::Auth(format!("Claude CLI not logged in: {stderr_first_line}"));
return ProviderError::Timeout { timeout_secs: 0 };  // только если СAM CLI сообщил "timed out"
return ProviderError::Network(stderr_first_line.to_string());
return ProviderError::ModelUnavailable(stderr_first_line.to_string());
return ProviderError::BadRequest(stderr_first_line.to_string());
return ProviderError::Server(stderr_tail.to_string());  // generic exit != 0
// ProviderError::Other(...) — НЕ использовать в драйвере (last-resort, должен быть редким)
```

**ВАЖНО:** При drop future (orchestrator outer timeout exceeded) driver **НЕ** возвращает `ProviderError::Timeout` — он просто умирает через `kill_on_drop`. Mapping `Err(_elapsed) → ProviderError::Timeout { timeout_secs: effective }` делает orchestrator после `tokio::time::timeout(...).await` (см. trait v3 §6.1 псевдокод).

**Verified против trait v3 §3.3 `should_fallback()` (строки 283-301):**
- `QuotaExceeded` / `Server` / `Network` / `Timeout` / `Auth` / `ModelUnavailable` → `true` → orchestrator пробует следующий провайдер в chain.
- `BadRequest` / `McpFailure` / `ToolLoopLimit` / `NotImplemented` / `Other` → `false` → fallback не запускается (логическая ошибка / проблема в запросе).

---

## §4. MCP политика Phase 1

### §4.1. Реальность v1.0.33
В боевом `post_executor::run_claude_cli_for_post` **MCP не используется ни в каком виде:**
- В argv нет `--mcp-config`.
- В env нет `CLAUDE_MCP_CONFIG_PATH`.
- В agent.md `tools: [Read, Write, Edit, Bash]` — только native, никаких `mcp_servers:`.
- Соответственно `mcp__*` инструменты пост-агенту физически недоступны.

### §4.2. Phase 1 решение
**ClaudeCliDriver игнорирует `request.mcp_bindings` с warning лог-сообщением:**
```
log::warn!(
    "ClaudeCliDriver Phase 1 MVP: dropping {} mcp_bindings (per-post MCP не поддерживается; см. backlog Phase 2)",
    request.mcp_bindings.len()
);
```
- **Это НЕ error.** Драйвер возвращает успех, агент просто отвечает текстом без вызова MCP-tools.
- В `run_logs` запись: `mcp_bindings_dropped: <count>` (если поле есть в схеме; иначе в `notes`).

### §4.3. Phase 2 R&D (backlog)
Три кандидата для per-post MCP, в порядке цены имплементации:
1. **`.mcp.json` в `current_dir(Outbox/<task_id>/)`** — самое дешёвое. Claude CLI v2.x читает `.mcp.json` из CWD автоматически (verified на `C:\CODE\.mcp.json`). Драйвер генерит файл в task_dir перед spawn → CLI подхватит → агент получит указанные MCP. **Требует проверки** на v2.1.140.
2. **`--mcp-config <path>` CLI флаг** — если CLI поддерживает (нужна верификация через `claude --help`).
3. **`mcp_servers:` в frontmatter `agent.md`** — потребует runtime regen agent.md при изменении bindings; race condition с CLI reload.

Рекомендация: пробовать в порядке 1 → 2 → 3.

---

## §5. Timeout — правильная reconciliation

### §5.1. Реальные источники в v1.0.33 (verified)
| Источник | Live значение | Назначение | Файл/строка | Потребитель |
|---|---|---|---|---|
| `settings.claude_cli_timeout_sec` | **360** | Outer `read_fut` (stdout до EOF) в CEO-runner | `claude_bridge.rs:434` | `run_claude_cli` (Гендир) |
| `settings.dispatcher_routing_timeout_sec` | **180** | Outer `read_fut` в Dispatcher router | `claude_bridge.rs:310` | `run_claude_cli_for_dispatcher` |
| `settings.post_executor_timeout_sec` | **600** | Outer `child.wait()` в post-agent spawn | `post_executor.rs:264` | `run_claude_cli_for_post` |
| `settings/mod.rs` default `claude_cli_timeout_sec` | 180 (legacy; рекомендую update до 360) | Если settings.json пустой | settings.rs | fallback |

### §5.2. Tier mapping для post_executor flow — КРИТИЧНАЯ ПОПРАВКА
`Tier::T1::default_timeout()` для **пост-агентов** = **600s, не 360s**.

Обоснование:
- 360s — это **CEO/Гендир timeout** (Claude thinking + tool calls в text-only ответе).
- Пост-агент пишет **реальный .docx/.xlsx** через Office COM или `python-docx` — занимает 5-8 минут наблюдательно (Phase 11A-D).
- Снижение T1 до 360s = **регрессия**: задачи `менеджер → договор + смета + протокол разногласий` будут резаться.
- Текущий boevoy `post_executor_timeout_sec = 600` — эмпирически выверенный max за 6 месяцев боевых сессий.

```rust
// Правильная Tier-таблица для Phase 1:
impl Tier {
    pub fn default_timeout(self) -> Duration {
        match self {
            Tier::T1 => Duration::from_secs(600),  // ← 600, НЕ 360
            Tier::T2 => Duration::from_secs(360),
            Tier::T3 => Duration::from_secs(60),
        }
    }
    pub const fn hard_cap_timeout() -> Duration {
        Duration::from_secs(600)
    }
}
```

`settings.claude_cli_timeout_sec = 360` — **отдельный live параметр** для CEO/Гендир, не связан с post_executor. Не путать. Не «alias на Tier::T1».

### §5.3. Timeout в обоих режимах (pal_enabled on/off)
| Режим | Кто оборачивает timeout | Значение |
|---|---|---|
| `pal_enabled=false` (legacy) | `post_executor.rs::run_claude_cli_for_post` через `tokio::time::timeout(settings.post_executor_timeout_sec, child.wait())` | 600s из settings |
| `pal_enabled=true` (Phase 1) | PAL orchestrator: `tokio::time::timeout(effective, driver.invoke(request))`, где `effective = (request.timeout || Tier::T1::default_timeout()).min(Tier::hard_cap_timeout())` | 600s по умолчанию |

При expiry в Phase 1 mode: orchestrator drop-ает driver future → `Drop for ClaudeCliDriver::invoke` срабатывает → child убит через `kill_on_drop` → child gone → orchestrator возвращает `ProviderError::Timeout { timeout_secs: effective }`.

### §5.4. Sanity-чек
В коде после имплементации:
```rust
debug_assert!(
    settings.post_executor_timeout_sec >= Tier::hard_cap_timeout().as_secs(),
    "post_executor_timeout_sec ({}) must be >= hard_cap ({})",
    settings.post_executor_timeout_sec,
    Tier::hard_cap_timeout().as_secs()
);
debug_assert_eq!(
    Tier::T1.default_timeout().as_secs(),
    settings.post_executor_timeout_sec,
    "Tier::T1 timeout мust match settings.post_executor_timeout_sec (single source of truth)"
);
```

---

## §6. Trait v3 рекомендации (для Гендира — отдельный sweep)

Резюме `Verdict` по предложениям Cursor:

| Поле / change | Verdict | Где |
|---|---|---|
| `workspace_path: Option<PathBuf>` в `ProviderRequest` | ✅ Добавить | См. §3.2 |
| `agent_slug: Option<String>` в `ProviderRequest` | ✅ Добавить | См. §3.2 |
| `model_override: Option<String>` в `ProviderRequest` | ✅ Добавить | См. §3.2 |
| `cli_profile/flags struct` | ❌ Не добавлять | Leaky abstraction; driver internals |
| `stop_reason enum` (вместо String) | ⚠️ Phase 2 | Phase 1 живёт со String; Phase 2 для multi-turn / tool-loop |
| `stdin_mode (PlainText \| JsonAgent)` | ❌ Не добавлять | Driver знает свой формат; альтернативный flow = новый driver |
| `ProviderResponse.artifacts: Vec<ArtifactRef>` | ⚠️ Оставить, но **always empty** для ClaudeCli/Qwen Phase 1 | Phase 2: для stream-json driver `tool_use_results` → агрегация сюда |
| `Tier::T1::default_timeout = 600s` (вместо 360s) | ✅ Исправить в trait v3 §3.4 / `impl Tier` | См. §5.2 |

---

## §7. Новые риски / open questions

### §7.1. `--print` deprecation
В выводе `claude --version` v2.1.x иногда замечено: «`--print` mode is deprecated, use `claude run` instead». Если CLI выкатят hard-removal — миграция станет вынужденной.

**Action:** при имплементации проверить актуальный `claude --version` + `claude --help` на v2.1.140 (когда Владелец разрешит запустить). Если есть warning — добавить в backlog Phase 1.5: «migration to `claude run`» (отдельный driver или argv-fork внутри ClaudeCliDriver по runtime detection версии).

### §7.2. stdout для usage / cost tracking — GAP
В `--output-format text` нет полей `input_tokens` / `output_tokens`. Последствия:
- `ProviderResponse.usage` в Phase 1 ClaudeCliDriver = **всегда `(0, 0, 0, 0)`**.
- `cost_per_1k_tokens(model) × usage = 0` всегда → cost dashboard для ClaudeCli постов в Phase 1 **не работает**.

**Опции (выбор за Гендиром в Phase 1+/2):**
1. **Принять gap:** cost-dashboard Phase 1 работает только для Qwen (где HTTP response содержит `usage`). ClaudeCli без cost-данных. Самый дешёвый путь.
2. **Post-hoc tokenizer:** приcoединить `anthropic-tokenizer` crate, считать `input_tokens` от собранного prompt, `output_tokens` от stdout text. Approximation ~95%. ~1 день работы.
3. **Migration на `--output-format stream-json`:** даст точный `usage` от Claude API. Но требует переписать `parse_stdout` под JSONL + проверку поддержки на CLI v2.1.140. См. §7.1.

### §7.3. Два пути вызова Claude после Phase 1
После Phase 1 в проекте будут **два разных pathway** для `claude.exe`:
1. **CEO/Dispatcher** → `claude_bridge::run_claude_cli` (legacy direct spawn, **НЕ** через PAL).
2. **Post-agents** → `pal.invoke(request)` → `ClaudeCliDriver::invoke` → spawn.

Последствия:
- Дублируется логика `ensure_*agent.md` (3 helpers сейчас: `ensure_mspro_ceo_agent`, `ensure_mspro_dispatcher_agent`, `ensure_post_agent_md`).
- Дублируется логика `hide_console + Command::new + arg-цепочка + kill_on_drop`.
- Дублируется timeout reconciliation (CEO `claude_cli_timeout_sec`, Dispatcher `dispatcher_routing_timeout_sec`, post_executor через PAL `Tier::T1`).

**Acceptable для Phase 1**, но в **backlog Phase 2** поставить высоким приоритетом: «миграция CEO + Dispatcher в PAL» — единая поверхность. После миграции `claude_bridge.rs` останется только как «MspareClaudeCliRunner» helper или удалится.

### §7.4. `hide_console` Windows-only
`claude_bridge.rs::hide_console` (строки 42-49) обёртывает `cmd.creation_flags(CREATE_NO_WINDOW)`. **Без него** `claude.exe` откроет видимое окно cmd на Windows.

**Action для имплементации:** `ClaudeCliDriver::build_command` обязан вызывать `hide_console(&mut cmd)` (или эквивалент `cmd.creation_flags(CREATE_NO_WINDOW)` через `os::windows::process::CommandExt`) до возврата Command. На *nix — no-op.

---

## §8. Verification чек-лист (для пост-имплементации)

Запускать ПОСЛЕ команды Владельца на имплементацию + после `cargo build` без ошибок.

### §8.1. Unit-тесты (mock-driver, без сети)
1. **argv assembly:** `ClaudeCliDriver::build_command(workspace, slug, model)` собирает argv со **всеми** 5 флагами в правильном порядке (`--print --output-format text --agent mspro-{slug} --model {m} --dangerously-skip-permissions`) + `current_dir = workspace` + `env(MSPRO_TASK_ID)` + `kill_on_drop(true)` + `hide_console` (на Windows; проверка через `cmd.get_creation_flags() & CREATE_NO_WINDOW != 0`).
2. **stdin format:** mock-spawn → `invoke` пишет `request.user_message` **plain text** в stdin без JSON wrapping (`assert!(!stdin_captured.starts_with("{"))`).
3. **`map_exit_to_error` стрелы:** прогнать 5 stderr-сценариев (quota / auth / timeout / network / generic) → правильный `ProviderError` variant.

### §8.2. Integration-тесты (mock claude.exe = batch-скрипт)
4. **feature-parity:** mock-claude.exe пишет `result.docx` в cwd → `pal.invoke(...)` → post_executor `diff_dir` → `register_artifact` → `dispatcher_logs.outbox_path` совпадает с pre-PAL flow. Smoke на той же задаче что Day 3 Phase 0.
5. **drop = kill:** mock-claude sleep 30s → orchestrator timeout 1s → `tokio::time::timeout` дропает future → проверить через `tasklist /fi "imagename eq mock-claude.exe"` — никаких orphan-ов через 2 сек после drop.
6. **timeout sanity:** request `timeout=2s`, mock-claude sleep 10s → orchestrator вернёт `Err(ProviderError::Timeout { timeout_secs: 2 })`. Никакого внутреннего outer timeout в driver (проверка: убрать orchestrator-timeout → invoke не возвращает `Timeout` сам).

### §8.3. E2E (на реальном claude.exe — последний шаг)
- Smoke на той же задаче что Day 3 Phase 0 Гендира (`task-91e3d598` эквивалент):
  - `pal_enabled=true` + post-агент `manager`.
  - Ожидание: `result.txt` / `result.docx` в `Outbox/<task_id>/`, `register_artifact` записал MIME правильно, `dispatcher_logs.status = completed` после approve.
  - Сверить с pre-PAL trail (DB snapshot до и после) — schema идентична.

---

## §9. Связанные документы

- **trait контракт** — `phase-1-pal-trait-spec.md` **v3 (approved 2026-05-26)** — actual source of truth для типов и сигнатур.
- **driver SPEC от Гендира** — `phase-1-claude-cli-driver-spec.md` v1 — **архивный**, к боевой реализации применяется ЭТОТ файл.
- **БД схема** — `phase-1-current-db-schema.sql` (v1.0.33, 13 таблиц).
- **DEC source of truth** — `decisions-log.md` (DEC-001…004).
- **Release/rebuild discipline** — `02-Patterns/rebuild-msi-playbook-v1.0.33.md`.
- **Boevoy код для сверки при имплементации:**
  - `src-tauri/src/commands/post_executor.rs` (вся функция `run_claude_cli_for_post`).
  - `src-tauri/src/commands/claude_bridge.rs` (для `hide_console`, для timeout reconciliation, для CEO/Dispatcher path — НЕ копировать через PAL в Phase 1).

---

## §10. Changelog

- **v1.1 (2026-05-24):** sync error names with trait v3 (по запросу Cursor). Правки:
  - §3.1 — добавлены `provider_used: ProviderKind::ClaudeCli` и `stop_reason: "end_turn"` (Phase 1 константа); удалено несуществующее в v3 поле `raw_debug`.
  - §3.2 — статус «УЖЕ В trait v3» (Гендир добавил все 3 поля как `Option<...>` с serde defaults); коммент `ProviderError::Internal` заменён на `BadRequest` (валидный variant в v3).
  - §3.3 — error mapping переписан под актуальные variants v3: `RateLimit` → `QuotaExceeded(String)`, `AuthFailure { reason }` → `Auth(String)`, `Network { reason }` → `Network(String)` (tuple), `Internal { reason }` → `Server(String)` для exit≠0; добавлены variants `ModelUnavailable(String)` и `BadRequest(String)` (новые в v3); добавлена колонка `should_fallback()` для каждой строки + verified против trait v3 §3.3 строки 283-301.
  - §9 — ссылка на trait актуализирована (v3 approved).
- **v1 (2026-05-24):** первая редакция. Read-only анализ post_executor.rs v1.0.33 + claude_bridge.rs + skeleton Гендира v1 + trait v2. По решению Владельца этот документ заменяет детальный driver SPEC — Гендир его не переписывает.

*End of IMPL REFERENCE.*
