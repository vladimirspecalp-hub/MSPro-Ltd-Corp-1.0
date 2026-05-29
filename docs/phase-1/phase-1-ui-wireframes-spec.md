# Phase 1 UI Wireframes — Service Bureau + Pod Runtime Model Switcher

**Версия:** v1.1
**Дата:** 2026-05-28
**Статус:** Approved with fixes applied (Cursor review)

## Changelog
- **v1.0** (2026-05-27) — initial draft (2 wireframes)
- **v1.1** (2026-05-28) — Cursor review против реального кода v1.0.33: 7 фактических правок + 6 UX-решений Владельца. Урок ClaudeCliDriver применён: опора на реальный код, не на память.

### v1.1 — Часть 1: фактические правки (Cursor против real code)
1. **EditPostKnowledgeModal** — табов нет, scrollable секции через `<hr>`. Runtime = новая секция, не вкладка.
2. **DepartmentCard** — `<li>` с slug/title/ЦКП/HMT/иконками 🧠📊. Кнопок Dispatch/Archive нет.
3. **SecurityVault** — нужен refactor → `SecretsPanel({ embedded?: boolean })`. Колонки реальные: Ключ, Доступ, Описание, Обновлён.
4. **HealthStatus** — строго из trait v3.1: `Alive | QuotaExceeded | AuthFailed | Unreachable | ServerError | Unknown`. Убрано `degraded`, `dead`, `stub`.
5. **ProviderKind** — строго из trait v3.1: `ClaudeCli | QwenHttp | ExternalGateway`. Убрано `qwen_local`.
6. **BrainStatusBadges** не drop-in (заточен под 2 мозга CEO) → новый `ProviderHealthBadge` в том же inline-стиле, generic для N провайдеров.
7. **ConditionBadge** не для Tier (для HMT Condition) → новый `TierBadge` по pill-паттерну `TaskRow STATUS_STYLE`.

### v1.1 — Часть 2: UX-решения Владельца
1. Sidebar label остаётся **"🔐 Отдел СБ"** (не переименовывать в Service Bureau).
2. Runtime indicator в DepartmentCard — **по наведению мышки** (tooltip), не всегда visible.
3. Default tier для новых постов — **T2** (360s, Standard).
4. Fallback chain reorder — **ArrowUp/ArrowDown** иконки lucide-react (не drag-and-drop).
5. Test connection в AddProviderModal — **blocking** (Save disabled до успешного теста).
6. external_gateway Test button — **disabled** серая (stub/NotImplemented видно, но не нажимается).

---

## WIREFRAME 1 — Service Bureau (расширение «Отдел СБ»)

### Sidebar (UX-1)
Label остаётся **«🔐 Отдел СБ»**. Клик → переход на страницу `ServiceBureau` (не переименовываем визуально, но внутри — полный credential broker по DEC-001).

### Layout страницы

```
┌──────────────────────────────────────────────────────────────┐
│ 🔐 Отдел СБ                                                   │ ← H1 (как в текущей SecurityVault)
├──────────────────────────────────────────────────────────────┤
│ [ Провайдеры ] [ Tier Presets ] [ Секреты ]                   │ ← Tab bar (паттерн Dispatcher tabs)
├──────────────────────────────────────────────────────────────┤
│                                                                │
│  (content активного tab)                                      │
│                                                                │
└──────────────────────────────────────────────────────────────┘
```

Padding: `32px 48px` (как существующая SecurityVault). Tab bar в стиле `Dispatcher.tsx`.

### Tab 1: Провайдеры

Header tab:
```
Зарегистрированные провайдеры              [+ Добавить провайдер]
```

Список карточек **ProviderCard** (новый компонент, не таблица):

```
┌──────────────────────────────────────────────────────────────┐
│ [TierBadge T1]  claude_cli — Claude CLI       [● Alive]  [⋮] │
│ Модель по умолчанию: claude-sonnet-4-6                        │
│ Путь: C:\Users\1\.local\bin\claude.exe                        │
│ Последняя проверка: 2 мин назад                               │
│ [Test connection]  [Edit]  [Delete]                           │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│ [TierBadge T3]  qwen_http — Qwen HTTP        [● Alive]   [⋮] │
│ Модель по умолчанию: qwen3:14b                                │
│ Endpoint: http://localhost:11434/v1                           │
│ Последняя проверка: 30 сек назад                              │
│ [Test connection]  [Edit]  [Delete]                           │
└──────────────────────────────────────────────────────────────┘
┌──────────────────────────────────────────────────────────────┐
│ external_gateway — External Gateway          [● Unknown] [⋮] │
│ Stub (NotImplemented в Phase 1)                               │
│ Endpoint: ws://127.0.0.1:8899                                 │
│ [Test connection — DISABLED]  [Edit]  [Delete]                │ ← UX-6: серая кнопка
└──────────────────────────────────────────────────────────────┘
```

**ProviderKind labels** (правка 5) — строго `claude_cli / qwen_http / external_gateway` (snake_case в UI/БД).

**ProviderHealthBadge** (правка 6) — новый компонент в стиле `BrainStatusBadges` (inline CSS, точка + label), generic для N провайдеров. Маппинг статусов trait v3.1 → UI-копии:
| HealthStatus (trait v3.1) | Цвет | UI label |
|---|---|---|
| `Alive` | зелёный | alive |
| `QuotaExceeded` | оранжевый | квота исчерпана |
| `AuthFailed` | красный | ошибка авторизации |
| `Unreachable` | серый | недоступен |
| `ServerError` | красный | ошибка сервера |
| `Unknown` | серый | неизвестно |

**Live update механика:**
- Active poll каждые 5 мин (cron worker в Rust)
- Lazy re-check при ошибке primary до fallback
- Tauri event `provider_health_changed` → `{ provider_id, new_status, checked_at }`
- React: `useEffect` подписка на event → setState → ре-рендер badge
- DEC-001 acceptance «Health обновление ≤30 сек» закрыт через event-emission (не таймер)

### AddProviderModal (UX-5: blocking)

Паттерн `AddSecretModal` (overlay + form), поля:
- **Kind** (radio: `claude_cli` / `qwen_http` / `external_gateway`) — required
- **Provider ID** (text, unique) — required, validation на duplicate
- **Display name** (text) — required
- **Endpoint / CLI path** (text) — required, label условный по Kind (Path для claude_cli, Endpoint для остальных)
- **Default model** (text) — required
- **Credentials** (dropdown `secret_ref` из SecretsPanel) — required для qwen_http/external_gateway, optional для claude_cli
- **Tier preset binding** (dropdown T1/T2/T3) — optional, для default привязки

Ниже формы — **TestConnectionInline** (новый компонент):
```
[Test connection]   Status: ⚪ Тест не пройден
                    ⏳ Тестирую...
                    ✓ Test OK (300 мс) — actual model returned: claude-sonnet-4-6
                    ✗ Test failed: <ProviderError variant + reason>
```

Кнопки:
- **[Отмена]** — закрыть без save
- **[Save]** — **disabled** пока `test_status !== 'ok'` (UX-5)

**Flow добавления qwen_http (DEC-001 ≤10 мин):**
1. Клик `[+ Добавить провайдер]` — модал открыт
2. Kind = qwen_http
3. Заполнить: provider_id=`qwen_secondary`, display=`Qwen Local`, endpoint=`http://localhost:11434/v1`, model=`qwen3:14b`
4. Clik **[Test connection]** → 0.5-2 сек → `✓ Test OK`
5. **[Save]** становится активной
6. Клик `[Save]` → Toast `«Провайдер qwen_http добавлен»`, модал закрыт, карточка в списке
7. Через ≤30 сек реальная alive-плашка по `provider_health_changed`

**Время:** 1.5-2 минуты (sub-DEC-001 ≤10 мин).

### EditProviderModal
Те же поля, **Kind disabled** (immutable). Test connection blocking при изменении endpoint/credentials/path — иначе можно сохранить нерабочую конфигурацию.

### Delete confirmation
Если провайдер используется в `post_runtime` хотя бы одним постом — modal warning: «Используется в N постах: <list>. Удаление переведёт их на fallback. Продолжить?». Если 0 — простое подтверждение.

### Tab 2: Tier Presets

3 карточки **TierPresetCard** (новый компонент):

```
┌────────────────────────────────────────┐
│ [TierBadge T1]  Premium                 │
│ Timeout: 600 сек                        │
│ Max turns: 80                           │
│ Default provider: claude_cli            │
│ Default model: claude-opus-4-7          │
│ [Edit]                                  │
└────────────────────────────────────────┘
┌────────────────────────────────────────┐
│ [TierBadge T2]  Standard ← default      │ ← UX-3: pill «default для новых»
│ Timeout: 360 сек                        │
│ Max turns: 40                           │
│ Default provider: claude_cli            │
│ Default model: claude-sonnet-4-6        │
│ [Edit]                                  │
└────────────────────────────────────────┘
┌────────────────────────────────────────┐
│ [TierBadge T3]  Local                   │
│ Timeout: 60 сек                         │
│ Max turns: 20                           │
│ Default provider: qwen_http             │
│ Default model: qwen3:14b                │
│ [Edit]                                  │
└────────────────────────────────────────┘
```

**TierBadge** (правка 7) — новый pill-компонент в стиле `TaskRow STATUS_STYLE`. Цвета по tier (T1=premium violet, T2=standard blue, T3=local gray).

**EditTierPresetModal:**
- Timeout (number, validation ≤600s = hard cap из trait v3.1 §7)
- Max turns (number, 1-200)
- Default provider (dropdown из Tab 1, фильтр по compatible kind)
- Default model (text, свободный)
- **[Save]** → запись в `tier_presets` таблицу → применяется ко всем постам этого tier при следующем run (hot-swap без рестарта)

### Tab 3: Секреты

**Refactor** (правка 3): `SecurityVault.tsx` → `SecretsPanel({ embedded?: boolean })`:
- `embedded=true` — убирает дублирующий H1 «🔐 Отдел СБ» (уже на странице ServiceBureau)
- `embedded=false` — backward-compat для старого роутинга если где-то ещё используется
- Остальная логика без изменений

Колонки таблицы (правка 3 — точно как в реальном коде):
| Ключ | Доступ | Описание | Обновлён |
|---|---|---|---|
| `qwen_local_token` | OS Keychain | Auth для Qwen HTTP | 2 ч назад |
| `openrouter_api_key` | OS Keychain | (если когда-то добавим) | … |

`AddSecretModal` — без изменений, переиспользуется как есть.

---

## WIREFRAME 2 — Pod Runtime Model Switcher

### Где живёт: EditPostKnowledgeModal — новая СЕКЦИЯ (правка 1)

**Реальная структура EditPostKnowledgeModal (v1.0.33)** — scrollable modal с секциями через `<hr>`:

```
[Header: «Знания поста: <slug>»]
  System prompt section (textarea ≤130 строк + counter)
<hr>
  Vault path section (read-only display + Open in Explorer)
<hr>
  Import section (drag-drop / file picker)
[Close button]
```

**Новая структура v1.1:**

```
[Header: «Знания поста: <slug>»]
  System prompt section
<hr>
  Vault path section
<hr>
  Import section
<hr>
  ⚙️ Runtime section ← НОВАЯ (правка 1)
[Close button]
```

**Опциональный prop** `initialSection?: 'prompt' | 'vault' | 'runtime'` — при открытии модала скроллит/фокусирует на нужную секцию. Используется при quick-edit из DepartmentCard tooltip → modal сразу на Runtime.

### Runtime section — структура

```
⚙️ Runtime
────────────────────────────────────────────────────────
Tier:
┌──────────────┐ ┌──────────────┐ ┌──────────────┐
│  ○ T1        │ │  ● T2        │ │  ○ T3        │
│  Premium     │ │  Standard    │ │  Local       │
│  600s / 80   │ │  360s / 40   │ │  60s / 20    │
└──────────────┘ └──────────────┘ └��─────────────┘
(новый TierRadioCard — radio в виде карточек, current = бордер violet/blue/gray)

Provider:    [claude_cli ▾]   ← dropdown из provider_registry
Model:       [claude-sonnet-4-6 ▾]  ← список моделей провайдера
Model override (опционально):
             [_______________________________]
             ↑ per-request override, пустое = использовать Model выше

Fallback chain:
┌─────────────────────────────────────────────────────┐
│ 1. claude_cli                       [↑] [↓] [✕]    │
│ 2. qwen_http                        [↑] [↓] [✕]    │
│ [+ Добавить провайдер в chain ▾]                    │
└─────────────────────────────────────────────────────┘
(новый FallbackChainList — стрелки (UX-4), не drag)

Preview (RuntimeEffectPreview):
┌─────────────────────────────────────────────────────┐
│ При следующем запуске поста:                        │
│  primary  = claude_cli / claude-sonnet-4-6          │
│  timeout  = 360s (из T2)                            │
│  fallback = qwen_http при QuotaExceeded/Unreachable │
└─────────────────────────────────────────────────────┘

[Save Runtime]    [Reset to tier defaults]
```

### Default для новых постов (UX-3)

При `create_post` (через UI или Гендира) новый пост получает `post_runtime` row:
- `tier = T2`
- `primary_provider = T2.default_provider` (`claude_cli`)
- `primary_model = T2.default_model` (`claude-sonnet-4-6`)
- `fallback_chain_json = ["qwen_http"]`
- `model_override = NULL`

Это означает: новый пост сразу работоспособен без ручной настройки. Tier T2 = разумный default по compute и timeout (5-8 мин для документов).

### Validation при Save Runtime

- **Primary provider** должен быть в `Alive` или `Unknown` (свежесозданный) — нельзя сохранить с `AuthFailed`/`Unreachable`/`ServerError` без warning override.
- **Fallback chain** — нет циклов (primary не в chain), нет удалённых/архивных провайдеров, нет дубликатов.
- **Model override** — свободный текст, не валидируется (доверяем — может быть beta-модель).
- **Tier mismatch warning** — если выбран provider/model не соответствующий tier defaults, показать info: «Не совпадает с tier preset. Точно сохранить?» — info, не блокер.

### Hot-swap механика (DEC-002/003)

1. `[Save Runtime]` → запись в `post_runtime` таблицу (UPSERT по `post_slug`)
2. **Без рестарта приложения** — последующие run-ы пост_executor читают свежие значения через PAL
3. Текущий run (если идёт) — НЕ прерывается, он на старой конфигурации
4. Toast: «Runtime поста `<slug>` обновлён. Изменения применятся при следующем run.»
5. Закрытие модала, обновление DepartmentCard tooltip

### Runtime indicator в DepartmentCard (UX-2 + правка 2)

**Реальная структура DepartmentCard** (правка 2) — `<li>` элемент списка постов:
```
<li>
  [slug + title]
  [ЦКП — 1-2 строки]
  [HMT row: ConditionBadge + Sparkline]
  [Иконки: 🧠 (knowledge) + 📊+ (stat)]
</li>
```

**Никаких** кнопок «Dispatch task» / «Archive» в текущем коде нет (правка 2).

**Runtime indicator — НЕ всегда visible** (UX-2):
- Mouse hover на `<li>` → small icon `Info` (lucide-react) появляется рядом с ConditionBadge
- Hover на icon → tooltip/popover:
  ```
  ┌──────────────────────────────────────┐
  │ Runtime:                              │
  │  Tier: T2 (Standard)                  │
  │  Provider: claude_cli                 │
  │  Model: claude-sonnet-4-6             │
  │  Fallback: qwen_http                  │
  │                                        │
  │  [Изменить →]                          │
  └──────────────────────────────────────┘
  ```
- Клик `[Изменить →]` → открывает `EditPostKnowledgeModal` с `initialSection='runtime'`
- Минимальный визуальный шум: на самой карточке runtime НЕ показан

**Альтернатива на MVP** (если popover сложен в реализации): просто маленькая `⚙️` иконка появляется в hover-state, клик → modal на Runtime секции. Tooltip с деталями — Phase 2 enhancement.

### User flow смены модели (DEC-003 ≤5 мин)

1. **Hover** на карточку поста → появляется `⚙️` или Info icon
2. **Hover на icon** → tooltip с текущим runtime
3. **Клик** «Изменить →» → modal открывается на Runtime section
4. **Изменить tier**: T2 → T3 (radio click)
5. **Provider/Model автообновляются** на T3 defaults (`qwen_http` / `qwen3:14b`)
6. (Опц.) Изменить fallback chain через `↑↓` arrows
7. **[Save Runtime]** → Toast, modal закрыт
8. Следующий run поста — на Qwen

**Время:** 20-30 секунд (sub-DEC-003 ≤5 мин).

---

## Existing/новые компоненты — финальная сверка

### Переиспользовано без изменений
- `Sidebar.tsx` — label остаётся «🔐 Отдел СБ» (UX-1)
- `Toast.tsx` — feedback на Save/Delete/Test
- `AddSecretModal` — внутри Tab 3 без изменений
- Dispatcher tab bar pattern — для tabs Service Bureau
- `TaskRow STATUS_STYLE` — заимствуется только стиль (inline CSSProperties) для нового `TierBadge`

### Refactor существующих
- **SecurityVault.tsx → SecretsPanel({ embedded?: boolean })** (правка 3) — лёгкий рефакторинг, добавление опционального prop
- **EditPostKnowledgeModal** — новая секция Runtime после `<hr>` + опциональный prop `initialSection` (правка 1)
- **DepartmentCard** — добавить hover state с Info icon + popover/tooltip (правка 2 + UX-2)

### Новые компоненты (минимально, без новых dep)
| Компонент | Назначение | Стиль-источник |
|---|---|---|
| `ProviderCard` | Карточка провайдера на Tab 1 | inline CSS, паттерн из SecurityVault row |
| `ProviderHealthBadge` | Generic health pill (правка 6) | стиль `BrainStatusBadges` |
| `TierBadge` | Pill для tier label (правка 7) | стиль `TaskRow STATUS_STYLE` |
| `TierPresetCard` | Карточка tier preset на Tab 2 | inline CSS, новый |
| `TierRadioCard` | Radio-карточка в Runtime section | inline CSS, новый |
| `TestConnectionInline` | Кнопка + статус блок (UX-5 blocking) | inline CSS, новый |
| `FallbackChainList` | Список с ↑↓ (UX-4) | inline CSS + lucide ArrowUp/Down |
| `RuntimeEffectPreview` | Текстовый preview | inline CSS, новый |

**Всё на:** React 19 + inline `CSSProperties` + `lucide-react`. **Никаких** новых dep (Tailwind, shadcn, Radix, dnd-kit — НЕ добавляются).

---

## DEC acceptance — закрыто wireframe v1.1

**DEC-001 (Service Bureau / Provider Registry):**
- ✅ «Регистрация API-провайдера ≤10 мин через UI» — AddProviderModal flow 1.5-2 мин
- ✅ «Health обновление ≤30 сек» — Tauri event `provider_health_changed` + active poll 5 мин + lazy re-check
- ✅ «Health в UI СБ» — Tab 1 с live `ProviderHealthBadge`
- ✅ «Sбой primary → auto-fallback» — fallback chain editable, audit в `run_logs.fallback_used`

**DEC-002 (Pod runtime persistence):**
- ✅ `post_runtime` таблица — primary, fallback_chain, tier, max_turns, model_override
- ✅ Hot-swap без рестарта — следующий run читает свежие значения
- ✅ Tier presets как source of truth, переопределение per post

**DEC-003 (Model & tier switching через UI):**
- ✅ «Смена модели Pod ≤5 мин в UI» — flow 20-30 секунд (sub-DEC)
- ✅ Tier T1/T2/T3 selector в Runtime section
- ✅ Model override per-request возможен
- ✅ Изменение tier меняет default model в dropdown

---

## Что НЕ выдумано (правка vs реальность v1.0.33)

| v1.0 wireframe говорил | Реальность (Cursor v1.0.33) | v1.1 wireframe фиксит |
|---|---|---|
| EditPostKnowledgeModal табы (System/Vault/Runtime) | Scrollable секции через `<hr>`, табов нет | Runtime = новая секция после `<hr>` |
| DepartmentCard кнопки [Dispatch] [Archive] | `<li>` со slug/HMT/иконками 🧠📊, кнопок нет | Hover tooltip + Info icon, без кнопок |
| SecurityVault как Tab «zero-change» | Полноценная страница с H1 + padding | Refactor → `SecretsPanel({ embedded })` |
| HealthStatus: `alive/degraded/quota_exceeded/auth_failed/dead/unknown/stub` | Trait v3.1: `Alive/QuotaExceeded/AuthFailed/Unreachable/ServerError/Unknown` | Только из trait v3.1 |
| ProviderKind: `qwen_local` | Trait v3.1: `QwenHttp` | `qwen_http` везде |
| BrainStatusBadges drop-in для N провайдеров | Заточен под 2 мозга CEO (props onClaude/onQwen) | Новый `ProviderHealthBadge` в том же стиле |
| ConditionBadge для Tier | Для HMT Condition (NonExistence…Power) | Новый `TierBadge` (стиль TaskRow STATUS_STYLE) |

---

## Sign-off
- ✅ Все 7 фактических правок Cursor применены (Часть 1)
- ✅ Все 6 UX-решений Владельца интегрированы (Часть 2)
- ✅ Воображаемых компонентов нет — опора только на реальный код v1.0.33
- ✅ Никаких новых dep — React 19 + inline CSS + lucide-react
- ✅ Закрыты DEC-001/002/003 acceptance criteria

**Ready for implementation:** программист может писать React-компоненты по этому wireframe + driver SPEC от программиста (IMPL-REFERENCE) — wireframe определяет UX/layout/flow, программист — props/state/hooks.

**Следующий раздел Итерации A:** Phase 1 DoD (детализация DEC acceptance в проверяемые тесты) или Risk register (≥5 рисков mitigation). Решение — Владелец.


---

## v1.1 Sign-off Confirmation (incremental save Часть 2)

Подтверждаю отдельным sweep что **6 UX-решений Владельца** интегрированы в тело документа v1.1, не только в changelog:

| # | UX-решение | Где в документе применено |
|---|---|---|
| 1 | Sidebar остаётся «🔐 Отдел СБ» | Wireframe 1 → Sidebar (UX-1) |
| 2 | Runtime indicator по наведению | Wireframe 2 → DepartmentCard hover tooltip (UX-2) |
| 3 | Default tier для новых постов = T2 | Wireframe 2 → Default для новых постов (UX-3) + Tab 2 «default для новых» pill на T2 карточке |
| 4 | Fallback reorder через ↑↓ | Wireframe 2 → FallbackChainList (UX-4) |
| 5 | Test connection blocking | Wireframe 1 → AddProviderModal + TestConnectionInline (UX-5) + Save disabled до Test OK |
| 6 | external_gateway Test = disabled | Wireframe 1 → ProviderCard карточка external_gateway (UX-6) |

Каждое решение помечено в тексте через `(UX-N)` маркер — Cursor может grep-нуть и проверить точечно.

**Incremental save workflow выполнен:**
- Save 1: `write_vault_file` overwrite — полный v1.1 контент с 7 фактическими правками (Часть 1) применёнными к телу wireframes
- Save 2: этот append — sign-off confirmation Части 2 + чеклист точек применения UX-решений

Wireframe v1.1 — **final ready for implementation review**.
