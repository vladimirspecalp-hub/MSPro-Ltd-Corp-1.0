-- ============================================================================
-- MSPro-Ltd Corp 1.0 — Step 6: HMT-engine (Statistics + Conditions)
-- Hubbard Management Technology: каждый пост получает временной ряд значений,
-- по которому автоматически рассчитывается Состояние.
-- Append-only — точки и состояния никогда не удаляются.
-- ============================================================================

CREATE TABLE IF NOT EXISTS statistics (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL,
    value REAL NOT NULL,
    recorded_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id)
);
CREATE INDEX IF NOT EXISTS idx_statistics_post_time
    ON statistics(post_id, recorded_at DESC);

CREATE TABLE IF NOT EXISTS condition_logs (
    id TEXT PRIMARY KEY,
    post_id TEXT NOT NULL,
    condition TEXT NOT NULL,
    assigned_at DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (post_id) REFERENCES posts(id),
    CHECK (condition IN
        ('NonExistence','Danger','Emergency','Normal','Affluence','Power'))
);
CREATE INDEX IF NOT EXISTS idx_condition_logs_post_time
    ON condition_logs(post_id, assigned_at DESC);
