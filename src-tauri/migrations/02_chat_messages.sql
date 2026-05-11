-- ============================================================================
-- MSPro-Ltd Corp 1.0 — Step 3 migration
-- Chat history table for the CEO conversation pane.
-- Decoupled from dispatcher_logs because chat = role/content dialogue,
-- whereas dispatcher_logs models structured cross-post task hand-offs.
-- ============================================================================

CREATE TABLE IF NOT EXISTS chat_messages (
    id          TEXT PRIMARY KEY,
    role        TEXT NOT NULL,
    content     TEXT NOT NULL,
    created_at  DATETIME DEFAULT CURRENT_TIMESTAMP,
    CHECK (role IN ('owner', 'ceo'))
);

CREATE INDEX IF NOT EXISTS idx_chat_created ON chat_messages(created_at DESC);
