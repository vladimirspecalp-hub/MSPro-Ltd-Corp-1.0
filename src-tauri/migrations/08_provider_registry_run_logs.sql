-- Migration 08 — Phase 1 (Iteration B): PAL foundation.
-- Только схема (CREATE TABLE + индексы). Сид провайдеров живёт ОДНИМ источником
-- в Rust (lib.rs::PROVIDER_SEED_SQL) и применяется в self-healing setup() —
-- не дублируем INSERT в .sql и в Rust (DRY).
-- Forward-only. Все DDL идемпотентны (IF NOT EXISTS). Без partial index / ALTER.

CREATE TABLE IF NOT EXISTS provider_registry (
    id            TEXT PRIMARY KEY,
    kind          TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    endpoint      TEXT,
    default_model TEXT,
    secret_ref    TEXT,
    status        TEXT NOT NULL DEFAULT 'enabled',
    created_at    TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at    TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS run_logs (
    id             TEXT PRIMARY KEY,
    task_id        TEXT,
    post_slug      TEXT,
    provider_id    TEXT NOT NULL,
    model_used     TEXT,
    tier           TEXT,
    tokens_in      INTEGER NOT NULL DEFAULT 0,
    tokens_out     INTEGER NOT NULL DEFAULT 0,
    latency_ms     INTEGER,
    cost_usd       REAL NOT NULL DEFAULT 0,
    success        INTEGER NOT NULL DEFAULT 0,
    fallback_used  INTEGER NOT NULL DEFAULT 0,
    attempt_number INTEGER NOT NULL DEFAULT 0,
    error_kind     TEXT,
    raw_output     TEXT,
    created_at     TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_run_logs_task     ON run_logs(task_id);
CREATE INDEX IF NOT EXISTS idx_run_logs_provider ON run_logs(provider_id);
CREATE INDEX IF NOT EXISTS idx_run_logs_created  ON run_logs(created_at);
