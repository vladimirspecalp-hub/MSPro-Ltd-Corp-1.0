# Phase 1 — UI Design Language Reference (v1.0.33)

**Версия:** v1  
**Дата:** 2026-05-26  
**Статус:** Snapshot для wireframes Service Bureau + Pod Runtime Model Switcher  
**Источник:** read-only анализ `MSPro-Ltd Corp 1.0/src/` (React 19 + Tauri 2, без UI-фреймворков)  
**Аудитор:** Cursor (code investigation)

---

## §1 Навигация и layout

### 1.1 Routing — state-based, не React Router

| Факт | Путь |
|------|------|
| Единое Tauri-окно, один SPA root | `src/main.tsx` → `src/App.tsx` |
| Текущий экран = `useState<View>` | `App.tsx` L12 `const [view, setView] = useState<View>("home")` |
| Условный рендер view | `App.tsx` L28–32 |
| Типы view | `src/components/Sidebar.tsx` L3: `"home" \| "ceo" \| "vault" \| "dispatcher" \| "settings"` |

**Нет:** `react-router`, отдельных Tauri windows для навигации, URL-based routing.

### 1.2 Sidebar — подтверждённые пункты

| id | Label в UI | Icon | Компонент view |
|----|------------|------|----------------|
| `home` | Главная | 🏠 | `views/Home.tsx` |
| `ceo` | Гендир (CEO) | 💬 | `views/CeoChat.tsx` |
| `vault` | **Отдел СБ** | 🔐 | `views/SecurityVault.tsx` |
| `dispatcher` | Диспетчер | 📡 | `views/Dispatcher.tsx` |
| `settings` | Настройки | ⚙ | `views/Settings.tsx` |

Файл: `src/components/Sidebar.tsx` L10–16 (`ITEMS`), L18–29 (стили контейнера).

**Критично для Гендира:** пункт **«Отдел СБ»** (`vault`) — это **Security Vault** (API-ключи, DPAPI / Windows Credential Manager), **НЕ** Service Bureau (PAL / провайдеры). Экрана Service Bureau в навигации **нет**.

### 1.3 Layout shell

```
┌─────────────────────────────────────────────────────────────┐
│ flex row, height 100vh, overflow hidden                     │
│  ┌──────────┐  ┌──────────────────────────────────────────┐ │
│  │ Sidebar  │  │ <main> flex:1, column, overflow hidden   │ │
│  │ 220px    │  │   → активный view (свой scroll внутри)   │ │
│  │ #1a1a1a  │  │   padding обычно 32px 48px             │ │
│  └──────────┘  └──────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────────┘
```

| Зона | Размер / стиль | Файл |
|------|----------------|------|
| Root | `fontFamily: system-ui`, `background: #f9f9f9`, `color: #1a1a1a` | `App.tsx` L16–24 |
| Sidebar | `width: 220`, `flexShrink: 0`, тёмный `#1a1a1a` | `Sidebar.tsx` L18–29 |
| Active nav | `background: #2a2a2a`, `borderLeft: 3px solid #4caf50` | `Sidebar.tsx` L38–48 |
| Content pages | `padding: 32px 48px`, `maxWidth: 900–1400`, `overflowY: auto` | каждый `views/*.tsx` |

Версия в sidebar: hardcoded строка `v1.0.33 · TICKET-001 vault tools` (`Sidebar.tsx` L56).

---

## §2 Design tokens (цвета, шрифты, spacing, где живут)

### 2.1 Где живут стили

| Слой | Использование |
|------|----------------|
| **Inline `React.CSSProperties`** | Основной способ — почти все компоненты |
| **`App.css`** | Legacy Vite scaffold + `.msg-row` hover, `@keyframes spin`, `toast-slide-in`, `@media (prefers-color-scheme: dark)` на `:root` |
| **`src/types/hmt.ts`** | Токены HMT condition badges (fg/bg по состоянию поста) |
| **Локальные константы** | `th`, `td`, `overlayStyle`, `brainBtnStyle`, `STATUS_STYLE` в файлах компонентов |

**Нет:** Tailwind, CSS Modules, styled-components, shadcn/ui, Radix, Mantine, MUI, Chakra.  
**Единственная UI-зависимость:** `lucide-react` (иконки в Toast, BrainStatusBadges).

### 2.2 Цветовая палитра (фактическая, из кода)

| Роль | Hex | Где |
|------|-----|-----|
| **Primary / CTA** | `#1a1a1a` (чёрный) | Кнопки send, tab active, primary actions |
| **Accent (nav)** | `#4caf50` | Sidebar active border |
| **Owner chat bubble** | `#1a73e8` | `CeoChat.tsx` owner bubble |
| **Page background** | `#f9f9f9` | App shell, CeoChat area |
| **Surface / card** | `#fff` | Карточки, таблицы, modals |
| **Muted surface** | `#f5f5f5`, `#fafafa` | Table header, empty states |
| **Border** | `#ddd`, `#ccc`, `#eee` | Cards, inputs |
| **Text primary** | `#1a1a1a`, `#333`, `#222` | Headings, body |
| **Text muted** | `#666`, `#888`, `#999` | Subtitles, hints |
| **Error** | bg `#fee` / `#ffebee`, border `#c00` / `#ef5350`, fg `#b71c1c` | Error boxes, failed status |
| **Success** | bg `#d4edda` / `#e8f5e9`, fg `#155724` / `#1b5e20` | Task completed, Brain badge OK |
| **Warning / in_progress** | bg `#fff3cd`, fg `#856404` | `TaskRow` in_progress |
| **System success (CEO)** | bg `#fff8e1`, border `#ffb300` | ⚡ tool messages |
| **Sidebar** | bg `#1a1a1a`, text `#e0e0e0` / `#bbb` | Always dark |

**CSS variables:** не используются в компонентах (только `:root` в `App.css` для scaffold).

### 2.3 Тёмная / светлая тема

- **Фактический UI:** явно **светлая** тема в content area (белые карточки, `#f9f9f9` фон).
- **Sidebar:** всегда тёмный, независимо от OS.
- **`App.css` L131–149:** `@media (prefers-color-scheme: dark)` меняет `:root` и generic `input/button` — **не переопределяет** inline-стили экранов. Скриншоты «тёмные» у Владельца могут быть OS-level или старый билд; **код v1.0.33 = light content + dark sidebar**.

### 2.4 Типографика

| Уровень | Размер | Пример |
|---------|--------|--------|
| Page H1 | 28px | Home, Dispatcher, Settings, SecurityVault |
| Page H1 (CEO) | 22px | CeoChat header |
| Section H2 | 18–20px | Settings sections, Home «Оргструктура» |
| Section H3 | 14px | BrainSettings subsections |
| Body | 13–14px | Tables, forms, chat |
| Small / meta | 10–12px | Timestamps, hints |
| Monospace | `ui-monospace, monospace` | Keys, payloads, tokens |

**Шрифт:** `system-ui, -apple-system, sans-serif` (App + Sidebar); `App.css` также объявляет `Inter, Avenir, Helvetica` на `:root` — на компоненты почти не влияет.

### 2.5 Spacing (неформальная система)

Повторяющиеся значения (не design tokens file):

| Token | px | Использование |
|-------|-----|---------------|
| Page padding | 32 / 48 | `padding: "32px 48px"` |
| Section gap | 16–24 | marginBottom headers, grid gap |
| Card padding | 14–24 | modals, sections |
| Control padding | 8–10 vertical, 12–18 horizontal | buttons, inputs |
| Border radius | 4 / 6 / 8 / 12 | buttons 6, cards 8, pills 12 |
| Sidebar item | 12×20 | nav buttons |

---

## §3 Переиспользуемые компоненты

### 3.1 Layout / shell

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `Sidebar` | `src/components/Sidebar.tsx` | Навигация, export type `View` |
| `ToastProvider` / `useToast` | `src/components/common/Toast.tsx` | Bottom-right toast (success/error/info) |

### 3.2 Views (экраны)

| View | Путь |
|------|------|
| Home | `src/components/views/Home.tsx` |
| CeoChat | `src/components/views/CeoChat.tsx` |
| SecurityVault | `src/components/views/SecurityVault.tsx` |
| Dispatcher | `src/components/views/Dispatcher.tsx` |
| Settings | `src/components/views/Settings.tsx` |

### 3.3 Chat / brain

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `BrainStatusBadges` | `src/components/chat/BrainStatusBadges.tsx` | Плашки Claude CLI / Qwen local + poll 30s |
| `MessageActions` | `src/components/chat/MessageActions.tsx` | Save to Vault memory |
| `MessageHoverActions` | `src/components/chat/MessageHoverActions.tsx` | Hover bar CEO messages |
| `AttachmentButtons` / `AttachmentsArea` | `src/components/chat/AttachmentButtons.tsx`, `AttachmentsArea.tsx` | 📎 файлы |
| `VaultSaveModal` | `src/components/chat/VaultSaveModal.tsx` | Modal save message |

### 3.4 Dispatcher

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `TaskRow` | `src/components/dispatcher/TaskRow.tsx` | Строка таблицы + status pill + ✅/❌ |
| `PayloadViewer` | `src/components/dispatcher/PayloadViewer.tsx` | Modal детали задачи |
| `ChainView` | `src/components/dispatcher/ChainView.tsx` | Цепочка hops в modal |
| `ArtifactsPanel` | `src/components/dispatcher/ArtifactsPanel.tsx` | Approve/reject артефактов |

### 3.5 Home / org

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `DepartmentCard` | `src/components/home/DepartmentCard.tsx` | Accordion отдел + список постов |
| `ConditionBadge` | `src/components/home/ConditionBadge.tsx` | HMT condition pill |
| `Sparkline` | `src/components/home/Sparkline.tsx` | Мини-график метрики |
| `AddPostModal` | `src/components/home/AddPostModal.tsx` | Создание поста |
| `AddStatisticModal` | `src/components/home/AddStatisticModal.tsx` | HMT статистика |
| `EditPostKnowledgeModal` | `src/components/home/EditPostKnowledgeModal.tsx` | 🧠 system prompt + vault import |

### 3.6 Settings

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `BrainSettings` | `src/components/settings/BrainSettings.tsx` | Claude/Qwen paths + Field pattern |
| `ExternalAgentGateway` | `src/components/settings/ExternalAgentGateway.tsx` | WS gateway on/off, token |
| `UpdateRollback` | `src/components/settings/UpdateRollback.tsx` | MSI update/rollback |
| `VaultPreview` | `src/components/settings/VaultPreview.tsx` | Preview Vault memory |

### 3.7 Vault (secrets)

| Компонент | Путь | Назначение |
|-----------|------|------------|
| `AddSecretModal` | `src/components/vault/AddSecretModal.tsx` | Форма нового секрета |

### 3.8 Паттерны без отдельного shared library

| Паттерн | Реализация | Файлы-эталон |
|---------|------------|--------------|
| **Status pill** | `padding 2px 10px`, `borderRadius 12`, bg+fg map | `TaskRow.tsx` STATUS_STYLE; `SecurityVault` ACCESS_LABELS; `BrainStatusBadges` Badge |
| **Tab bar** | Toggle buttons: active `#1a1a1a` / inactive white | `Dispatcher.tsx` L115–141 |
| **Primary button** | `#1a1a1a` bg, white text, radius 6 | SecurityVault, Dispatcher |
| **Secondary button** | white bg, `#ccc` border | везде |
| **Error banner** | `#fee` + `#c00` border | Dispatcher, SecurityVault, Home |
| **Empty state** | dashed border, `#fafafa`, centered text | Dispatcher, SecurityVault |
| **Modal overlay** | `fixed inset 0`, `rgba(0,0,0,0.5–0.55)`, white panel | `AddSecretModal`, `EditPostKnowledgeModal`, `PayloadViewer` |
| **Data table** | `borderCollapse collapse`, `#f5f5f5` thead | Dispatcher, SecurityVault |
| **Section card** | white bg, `1px solid #ddd`, `borderRadius 8`, padding 20–24 | BrainSettings, ExternalAgentGateway |

---

## §4 Существующие экраны (UX-паттерны)

### 4.1 Главная (`Home.tsx`)

- **Header:** H1 «MSPro-Ltd Corp 1.0», subtitle с `app_info` + Rust `ping`.
- **Body:** grid карточек отделов `repeat(auto-fill, minmax(380px, 1fr))`.
- **Данные:** SQLite `departments` через `@tauri-apps/plugin-sql`.
- **Паттерн:** collapsible `DepartmentCard` → посты, HMT badges, sparklines, кнопки CRUD/modals.

### 4.2 Гендир CEO (`CeoChat.tsx`)

- **Layout:** column flex — header (fixed) / messages scroll / input bar (fixed).
- **Header:** subtitle по `brain_mode`; **segmented control** Claude Opus / Qwen 3 (`brainBtnStyle`); `BrainStatusBadges` ниже.
- **Chat:** owner bubbles справа синие; CEO слева серые; streaming dashed border; system ⚡/⚠️ full-width yellow/red panels.
- **Input:** auto-grow textarea, Send, Cancel (red outline), attachments drag-drop overlay blue.
- **Backend:** `list_chat_history`, `send_ceo_message`, events `ceo-start/chunk/done/tool-result`; `set_brain_mode`.

### 4.3 Отдел СБ (`SecurityVault.tsx`) — НЕ Service Bureau

- **Назначение:** метаданные секретов (`vault_list_secrets`), reveal/delete, DPAPI.
- **UX:** header + toolbar (+ Новый секрет, ↻) + table или empty state + modals.
- **Access level pills:** 0–3 с цветами (public/heads/ceo/owner).
- **Нет:** провайдеров LLM, health, Tier, PAL.

### 4.4 Диспетчер (`Dispatcher.tsx`)

- **Tabs:** Inbox / Processing / Awaiting / Completed / Failed / Все — client-side filter на `list_recent_tasks(500)`.
- **Toolbar:** text filter + refresh + count.
- **Table columns:** From → To, Status, Payload preview, ms, relative time, actions.
- **Row click:** `PayloadViewer` modal (chain, artifacts, payload monospace block `#1a1a1a` / `#9ef5a4`).
- **Live updates:** `dispatcher-task-changed` event → refresh.
- **Quick actions:** ✅ complete / ❌ fail на `in_progress`.

### 4.5 Настройки (`Settings.tsx`)

- Вертикальный stack секций (maxWidth 900):
  1. `BrainSettings` — формы path/model + checkbox auto-fallback
  2. `UpdateRollback` — updater
  3. `ExternalAgentGateway` — toggle WS :8899, token mask/reveal
  4. `VaultPreview` — read-only memory preview

**Паттерн формы:** label 12px bold → input monospace → hint 12px gray → «Сохранить» только при `edited` (`BrainSettings` Field).

---

## §5 Что переиспользовать для Service Bureau (DEC-001)

### 5.1 Рекомендуемая привязка UI

| Требование DEC-001 | Существующий паттерн | Файл-эталон |
|--------------------|----------------------|-------------|
| Список провайдеров | Data **table** или **section cards** | `SecurityVault.tsx` table; `ExternalAgentGateway` section |
| Health alive/degraded/quota/dead | **Status pill** + optional icon | `BrainStatusBadges.tsx` Badge; `TaskRow` STATUS_STYLE |
| Добавить/редактировать провайдер | **Modal form** + Field inputs | `AddSecretModal.tsx`, `BrainSettings` Field |
| Tier T1/T2/T3 presets | **Tab bar** или **3 toggle buttons** | `Dispatcher.tsx` tabs; `CeoChat` brainBtnStyle |
| Ошибки / save feedback | **Toast** | `Toast.tsx` |
| Пустой registry | **Empty state** dashed box | `SecurityVault`, `Dispatcher` |

### 5.2 НЕ путать с «Отдел СБ»

- Wireframes Service Bureau: **новый пункт sidebar** (например «Service Bureau» / «Провайдеры») или подсекция **Настройки** — сейчас `vault` занят секретами.
- Переименование `Отдел СБ` без миграции сломает mental model Владельца (SB = security в текущем UI).

### 5.3 Плашки провайдеров из Гендир-чата

`BrainStatusBadges` — **ближайший визуальный прототип** health row для Service Bureau:
- label + ok/not + detail string
- poll 30s через `detect_claude_cli` / `detect_qwen`
- Для PAL v3 health: расширить mapping `HealthStatus` → те же цвета (Alive=green, Unreachable=red, QuotaExceeded=orange, etc.)

### 5.4 Чего нет в UI для Tier / provider_registry

- Нет полей `preferred_model` в React (только backend `posts`).
- Нет `provider_registry`, `run_logs`, PAL health scheduler в frontend.
- Формы Tier timeout — проектировать по `BrainSettings` Field, значения из trait v3 (T1=600s…).

---

## §6 Model Switcher — прототип (текущая реализация)

### 6.1 Что есть сейчас — CEO Brain Mode Switcher (DEC-003 частично)

| Аспект | Реализация |
|--------|------------|
| UI | Две кнопки в header CeoChat: «⭐ Claude 4.7 Opus» / «🐉 Qwen 3 (Автономный)» |
| Стили | `brainBtnStyle(active)` — active filled `#1a1a1a`, inactive white outline |
| Persistence | `invoke("set_brain_mode", { mode })` → `settings.brain_mode` |
| Modes | `"claude_cli" \| "qwen_local" \| "claude_external"` (третий скрыт из UI, legacy) |
| Guards | Qwen disabled if `!qwenReady`; external needs `external_agent_enabled` |
| Status | `BrainStatusBadges` — не switcher, а health indicators |
| Scope | **Только CEO chat** — не post-level, не per-pod |

Файлы: `CeoChat.tsx` L253–274, L483–503; `BrainSettings.tsx` (paths/models, не switcher UI).

### 6.2 Это прототип Pod Runtime Model Switcher?

**Частично да** для DEC-003 «hot-swap модели»:
- ✅ Segmented control pattern
- ✅ Persist через settings + Tauri invoke
- ✅ Provider availability gating

**Нет** для полного DEC-003:
- ❌ Per-post / per-department switcher (должен жить в `DepartmentCard` / post editor)
- ❌ `posts.preferred_model` UI
- ❌ Tier T1/T2/T3 visibility
- ❌ Fallback chain UI
- ❌ Service Bureau link «какой provider_id обслуживает этот пост»

### 6.3 Переиспользуемость для Pod Runtime

1. Скопировать **`brainBtnStyle` + row of buttons** для 2–3 моделей на карточке поста.
2. Переиспользовать **`BrainStatusBadges`** рядом с switcher на post detail.
3. Сохранение: новый invoke `set_post_preferred_model` (backend есть в DB, UI нет) + toast success.
4. Не reuse CEO `set_brain_mode` напрямую — разная семантика (CEO brain vs post runtime).

---

## §7 Gaps (что проектировать с нуля)

| Gap | Важность для wireframes |
|-----|-------------------------|
| **Экран Service Bureau отсутствует** | Новый view + sidebar item или Settings subsection |
| **«Отдел СБ» ≠ Service Bureau** | Naming collision — явно развести в wireframes |
| **Нет design system package** | Все новые экраны — inline styles по таблице §2 |
| **Нет shared Button/Card/Table components** | Копировать константы из эталонных файлов |
| **Нет provider health grid** | Расширить BrainStatusBadges pattern на 3+ providers + ExternalGateway |
| **Нет CRUD provider form** | Новая modal по AddSecretModal + select enums ProviderKind |
| **Нет Tier editor UI** | Read-only badges в v1 wireframes; edit → Settings-style fields |
| **Нет run_logs / fallback chain viewer** | Можно второй tab в Service Bureau по аналогии Dispatcher |
| **Post preferred_model UI** | EditPostKnowledgeModal — логичное место добавить model dropdown |
| **Нет React Router deep links** | Service Bureau = ещё один `View` id в Sidebar + App.tsx branch |
| **Health enum mismatch** | Trait v3 `HealthStatus` enum ≠ Brain badge binary ok/not — mapping table в impl |
| **Dark theme inconsistency** | Wireframes лучше рисовать **light content + dark sidebar** как в коде |

---

## §8 Файловая карта (quick reference)

```
src/
  App.tsx                 # shell + view state
  App.css                 # minimal global + dark OS pref
  components/
    Sidebar.tsx           # nav
    views/                # 5 screens
    chat/                 # CEO chat + BrainStatusBadges
    dispatcher/           # task table + modals
    home/                 # org cards + modals
    settings/             # settings sections
    vault/                # secret modal
    common/Toast.tsx
  types/hmt.ts            # condition color tokens
```

**package.json dependencies:** `react`, `react-dom`, `@tauri-apps/*`, `lucide-react` only.

---

## §9 Changelog

- **v1 (2026-05-26):** Initial snapshot from `MSPro-Ltd Corp 1.0` frontend v1.0.33 read-only investigation.
