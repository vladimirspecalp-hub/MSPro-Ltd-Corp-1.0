-- Этап 1 (Заход 1) — динамическая оргструктура конструктора «Оргсхема».
-- ОТДЕЛЬНЫЕ таблицы: НЕ трогают departments/posts (от них зависят мозг Гендира
-- build_ceo_system_prompt и Диспетчер). 3 уровня:
--   org_divisions (Отделение) → org_departments (Отдел) → org_agents (Агент).
-- org_agents здесь — ТОЛЬКО структурные поля. Поля карточки (claude_md,
-- brain_choice, mcp_servers, ckp, checklist) добавит Заход 2 отдельной миграцией
-- ПОСЛЕ дизайна секретов (DPAPI). Секретов/токенов в БД нет и не будет.
-- R-T-006: self-healing CREATE TABLE IF NOT EXISTS дублируется в lib.rs::setup
-- на случай недоезда миграции (tauri-plugin-sql не ретраит). Без partial index
-- и без ALTER в этой миграции (грабли 08-tribal #2/#3).

CREATE TABLE IF NOT EXISTS org_divisions (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS org_departments (
    id          TEXT PRIMARY KEY,
    division_id TEXT NOT NULL,
    name        TEXT NOT NULL,
    description TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (division_id) REFERENCES org_divisions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS org_agents (
    id            TEXT PRIMARY KEY,
    department_id TEXT NOT NULL,
    name          TEXT NOT NULL,
    slug          TEXT NOT NULL,
    role_label    TEXT NOT NULL DEFAULT 'member',
    status        TEXT NOT NULL DEFAULT 'active',
    folder_path   TEXT,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at    TEXT DEFAULT NULL,
    FOREIGN KEY (department_id) REFERENCES org_departments(id) ON DELETE CASCADE,
    CHECK (role_label IN ('head','member')),
    CHECK (status IN ('active','paused','off'))
);
