-- ============================================================================
-- MSPro-Ltd Corp 1.0 — Initial schema
-- Hubbard 8 departments + Posts + Agents + Owner history + Dispatcher + СБ
-- All secrets live in Windows Credential Manager (DPAPI). SQLite holds metadata.
-- ============================================================================

-- 1. ОТДЕЛЕНИЯ (8 канонных по Хаббарду + Office of Owner)
CREATE TABLE IF NOT EXISTS departments (
    id TEXT PRIMARY KEY,
    dept_number INTEGER NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (dept_number BETWEEN 0 AND 7)
);

-- 2. ПОСТЫ (должности с ЦКП внутри отделений)
CREATE TABLE IF NOT EXISTS posts (
    id TEXT PRIMARY KEY,
    department_id TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    central_product TEXT NOT NULL,
    main_statistic_metric TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (department_id) REFERENCES departments(id),
    CHECK (status IN ('active','paused','archived'))
);

-- 3. АГЕНТЫ (AI-исполнители на постах)
CREATE TABLE IF NOT EXISTS agents (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL,
    model TEXT NOT NULL,
    skill_path TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id),
    CHECK (status IN ('active','paused','archived'))
);

-- 4. ОФИС ВЛАДЕЛЬЦА — стратегический исторический лог
CREATE TABLE IF NOT EXISTS owner_history_log (
    id TEXT PRIMARY KEY,
    event_type TEXT NOT NULL,
    description TEXT NOT NULL,
    expected_outcome TEXT,
    actual_outcome TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    recorded_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (event_type IN ('decision','success','failure','milestone')),
    CHECK (status IN ('pending','analyzed','archived'))
);

-- 5. ДИСПЕТЧЕР — шина межагентских / межотдельных взаимодействий
CREATE TABLE IF NOT EXISTS dispatcher_logs (
    id TEXT PRIMARY KEY,
    from_entity TEXT NOT NULL,
    to_entity TEXT NOT NULL,
    task_payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'in_progress',
    execution_time_ms INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (status IN ('in_progress','completed','failed'))
);

-- 6. СБ-ХРАНИЛИЩЕ — ТОЛЬКО МЕТАДАТА (значения через DPAPI в Credential Manager)
CREATE TABLE IF NOT EXISTS security_vault (
    id TEXT PRIMARY KEY,
    key_name TEXT NOT NULL UNIQUE,
    description TEXT,
    access_level INTEGER NOT NULL DEFAULT 0,
    credential_target TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (access_level >= 0 AND access_level <= 3)
);

-- ============================================================================
-- INDEXES (горячие пути)
-- ============================================================================

CREATE INDEX IF NOT EXISTS idx_posts_department ON posts(department_id, status);
CREATE INDEX IF NOT EXISTS idx_agents_post ON agents(post_id, status);

CREATE INDEX IF NOT EXISTS idx_owner_log_recorded ON owner_history_log(recorded_at DESC);
CREATE INDEX IF NOT EXISTS idx_owner_log_status ON owner_history_log(status, recorded_at DESC);
CREATE INDEX IF NOT EXISTS idx_owner_log_event_type ON owner_history_log(event_type, recorded_at DESC);

CREATE INDEX IF NOT EXISTS idx_dispatcher_status ON dispatcher_logs(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_dispatcher_to ON dispatcher_logs(to_entity, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_dispatcher_from ON dispatcher_logs(from_entity, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_security_access ON security_vault(access_level);

-- ============================================================================
-- SEED: 8 канонных отделений (idempotent: INSERT OR IGNORE по dept_number UNIQUE)
-- ============================================================================

INSERT OR IGNORE INTO departments (id, dept_number, name, description) VALUES
    ('dept-0-owner',   0, 'Офис Владельца',                'Стратегия, инвестиции, исторический контроль'),
    ('dept-1-hco',     1, 'Отделение Построения / HCO',    'Найм, коммуникации, Диспетчер'),
    ('dept-2-distrib', 2, 'Отделение Распространения',     'Маркетинг и продажи'),
    ('dept-3-finance', 3, 'Финансовое Отделение',          'Доходы, активы, СБ'),
    ('dept-4-tech',    4, 'Техническое Отделение',         'Производство — высотные работы'),
    ('dept-5-qual',    5, 'Отделение Квалификации',        'Контроль качества, обучение'),
    ('dept-6-pr',      6, 'Отделение по связям',           'PR, новые рынки'),
    ('dept-7-exec',    7, 'Исполнительное Отделение',      'Гендир, координация всех отделов');
