-- ============================================================================
-- BL-P1-016: Add 'cancelled' to dispatcher_logs status CHECK constraint.
--
-- SQLite does not support ALTER CHECK — full table rebuild required.
-- Forward-only: creates dispatcher_logs_new with updated CHECK, copies data,
-- drops original, renames. Indexes are recreated.
-- ============================================================================

CREATE TABLE dispatcher_logs_new (
    id TEXT PRIMARY KEY,
    from_entity TEXT NOT NULL,
    to_entity TEXT NOT NULL,
    task_payload TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'in_progress',
    execution_time_ms INTEGER,
    created_at DATETIME DEFAULT CURRENT_TIMESTAMP,
    parent_task_id TEXT DEFAULT NULL,
    completed_at DATETIME DEFAULT NULL,
    attempts_count INTEGER NOT NULL DEFAULT 1,
    hop_kind TEXT DEFAULT NULL,
    routed_by_model TEXT DEFAULT NULL,
    refined_prompt TEXT DEFAULT NULL,
    outbox_path TEXT DEFAULT NULL,
    raw_brain_response TEXT DEFAULT NULL,
    CHECK (status IN ('in_progress','completed','failed','cancelled'))
);

INSERT INTO dispatcher_logs_new
    SELECT id, from_entity, to_entity, task_payload, status, execution_time_ms,
           created_at, parent_task_id, completed_at, attempts_count, hop_kind,
           routed_by_model, refined_prompt, outbox_path, raw_brain_response
    FROM dispatcher_logs;

DROP TABLE dispatcher_logs;

ALTER TABLE dispatcher_logs_new RENAME TO dispatcher_logs;

-- Recreate all indexes (DROP TABLE removed them).
CREATE INDEX IF NOT EXISTS idx_dispatcher_status ON dispatcher_logs(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_dispatcher_to ON dispatcher_logs(to_entity, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_dispatcher_from ON dispatcher_logs(from_entity, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_dispatcher_parent ON dispatcher_logs(parent_task_id);
CREATE INDEX IF NOT EXISTS idx_dispatcher_hop ON dispatcher_logs(hop_kind);
