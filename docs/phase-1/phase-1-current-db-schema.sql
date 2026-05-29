-- MSPro-Ltd Corp app.db schema dump (live v1.0.33, 2026-05-24)
-- Applied migrations: [1, 2, 3, 4, 5, 6, 7]
-- Objects: 13 tables, 19 indexes
-- Источник для Phase 1 SPEC (Service Bureau + PAL): провайдерские таблицы расширяют эту схему.

CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);

CREATE TABLE agents (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL,
    model TEXT NOT NULL,
    skill_path TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id),
    CHECK (status IN ('active','paused','archived'))
);

CREATE TABLE chat_messages (
    id          TEXT PRIMARY KEY,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (role IN ('owner', 'ceo'))
);

CREATE TABLE condition_logs ( id TEXT PRIMARY KEY, post_id TEXT NOT NULL, condition TEXT NOT NULL, assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, FOREIGN KEY (post_id) REFERENCES posts(id), CHECK (condition IN ('NonExistence','Danger','Emergency','Normal','Affluence','Power')) );

CREATE TABLE departments (
    id TEXT PRIMARY KEY,
    dept_number INTEGER NOT NULL UNIQUE,
    name TEXT NOT NULL,
    description TEXT,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (dept_number BETWEEN 0 AND 7)
);

CREATE TABLE dispatcher_decisions ( id TEXT PRIMARY KEY, source_task_id TEXT NOT NULL REFERENCES dispatcher_logs(id), result_task_id TEXT REFERENCES dispatcher_logs(id), decision_kind TEXT NOT NULL CHECK (decision_kind IN ('forward','decompose','escalate','reject','clarify','retry')), reasoning TEXT, model_used TEXT NOT NULL, routing_complexity TEXT CHECK (routing_complexity IS NULL OR routing_complexity IN ('simple','complex')), elapsed_ms INTEGER, created_at DATETIME DEFAULT CURRENT_TIMESTAMP );

CREATE TABLE dispatcher_logs (
    id TEXT PRIMARY KEY,
    from_entity TEXT NOT NULL,
    to_entity TEXT NOT NULL,
    task_payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'in_progress',
    execution_time_ms INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP, parent_task_id TEXT DEFAULT NULL, completed_at DATETIME DEFAULT NULL, attempts_count INTEGER NOT NULL DEFAULT 1, hop_kind TEXT DEFAULT NULL, routed_by_model TEXT DEFAULT NULL, refined_prompt TEXT DEFAULT NULL, outbox_path TEXT DEFAULT NULL,
    CHECK (status IN ('in_progress','completed','failed'))
);

CREATE TABLE owner_history_log (
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

CREATE TABLE posts (
    id TEXT PRIMARY KEY,
    department_id TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    title TEXT NOT NULL,
    central_product TEXT NOT NULL,
    main_statistic_metric TEXT,
    status TEXT NOT NULL DEFAULT 'active',
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP, system_prompt_md TEXT DEFAULT NULL, vault_subdir TEXT DEFAULT NULL, claude_agent_name TEXT DEFAULT NULL, preferred_model TEXT DEFAULT NULL, updated_at TEXT DEFAULT NULL,
    FOREIGN KEY (department_id) REFERENCES departments(id),
    CHECK (status IN ('active','paused','archived'))
);

CREATE TABLE security_vault (
    id TEXT PRIMARY KEY,
    key_name TEXT NOT NULL UNIQUE,
    description TEXT,
    access_level INTEGER NOT NULL DEFAULT 0,
    credential_target TEXT NOT NULL,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    updated_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (access_level >= 0 AND access_level <= 3)
);

CREATE TABLE statistics ( id TEXT PRIMARY KEY, post_id TEXT NOT NULL, value REAL NOT NULL, recorded_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP, FOREIGN KEY (post_id) REFERENCES posts(id) );

CREATE TABLE task_artifacts ( id TEXT PRIMARY KEY, task_id TEXT NOT NULL REFERENCES dispatcher_logs(id), rel_path TEXT NOT NULL, mime_type TEXT, size_bytes INTEGER, created_by TEXT NOT NULL, created_at DATETIME DEFAULT CURRENT_TIMESTAMP, approved_at DATETIME, rejected_at DATETIME, reject_reason TEXT, UNIQUE(task_id, rel_path) );

CREATE TABLE vault_ops_log ( id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL, source_post TEXT NOT NULL, tool TEXT NOT NULL, path TEXT NOT NULL, mode TEXT, anchor TEXT, bytes_before INTEGER, bytes_after INTEGER, success INTEGER NOT NULL, error_code TEXT, archive_path TEXT, reason TEXT );

CREATE INDEX idx_agents_post ON agents(post_id, status);

CREATE INDEX idx_artifacts_task ON task_artifacts(task_id);

CREATE INDEX idx_chat_created ON chat_messages(created_at DESC);

CREATE INDEX idx_condition_logs_post_time ON condition_logs(post_id, assigned_at DESC);

CREATE INDEX idx_decisions_model ON dispatcher_decisions(model_used, created_at DESC);

CREATE INDEX idx_decisions_source ON dispatcher_decisions(source_task_id);

CREATE INDEX idx_dispatcher_from ON dispatcher_logs(from_entity, created_at DESC);

CREATE INDEX idx_dispatcher_hop ON dispatcher_logs(hop_kind);

CREATE INDEX idx_dispatcher_parent ON dispatcher_logs(parent_task_id);

CREATE INDEX idx_dispatcher_status ON dispatcher_logs(status, created_at DESC);

CREATE INDEX idx_dispatcher_to ON dispatcher_logs(to_entity, created_at DESC);

CREATE INDEX idx_owner_log_event_type ON owner_history_log(event_type, recorded_at DESC);

CREATE INDEX idx_owner_log_recorded ON owner_history_log(recorded_at DESC);

CREATE INDEX idx_owner_log_status ON owner_history_log(status, recorded_at DESC);

CREATE INDEX idx_posts_department ON posts(department_id, status);

CREATE INDEX idx_security_access ON security_vault(access_level);

CREATE INDEX idx_statistics_post_time ON statistics(post_id, recorded_at DESC);

CREATE INDEX idx_vault_ops_path ON vault_ops_log(path);

CREATE INDEX idx_vault_ops_timestamp ON vault_ops_log(timestamp);
