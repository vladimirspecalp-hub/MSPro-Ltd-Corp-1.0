-- TICKET-001: audit log for CEO vault file tools (write/patch/delete).

CREATE TABLE IF NOT EXISTS vault_ops_log (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  timestamp TEXT NOT NULL,
  source_post TEXT NOT NULL,
  tool TEXT NOT NULL,
  path TEXT NOT NULL,
  mode TEXT,
  anchor TEXT,
  bytes_before INTEGER,
  bytes_after INTEGER,
  success INTEGER NOT NULL,
  error_code TEXT,
  archive_path TEXT,
  reason TEXT
);

CREATE INDEX IF NOT EXISTS idx_vault_ops_path ON vault_ops_log(path);
CREATE INDEX IF NOT EXISTS idx_vault_ops_timestamp ON vault_ops_log(timestamp);
