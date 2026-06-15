-- Заход 2: Карточка агента — 8 новых колонок org_agents + таблица связей.
--
-- brain_mode валидируется на Rust (enum match), НЕ через CHECK (table rebuild).
-- Секреты агентов — через СУЩЕСТВУЮЩИЕ vault_add_secret / vault_reveal,
-- конвенция key_name: agent-{short_id}-{type}-{name}.

ALTER TABLE org_agents ADD COLUMN role_prompt_md TEXT DEFAULT NULL;
ALTER TABLE org_agents ADD COLUMN brain_mode TEXT NOT NULL DEFAULT 'disabled';
ALTER TABLE org_agents ADD COLUMN brain_model TEXT DEFAULT NULL;
ALTER TABLE org_agents ADD COLUMN brain_endpoint TEXT DEFAULT NULL;
ALTER TABLE org_agents ADD COLUMN mcp_servers_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE org_agents ADD COLUMN ckp_text TEXT DEFAULT NULL;
ALTER TABLE org_agents ADD COLUMN checklist_json TEXT NOT NULL DEFAULT '[]';
ALTER TABLE org_agents ADD COLUMN memory_md TEXT DEFAULT NULL;

CREATE TABLE IF NOT EXISTS org_agent_links (
    id            TEXT PRIMARY KEY,
    from_agent_id TEXT NOT NULL,
    to_agent_id   TEXT NOT NULL,
    link_type     TEXT NOT NULL CHECK (link_type IN ('next','verifier','input_from')),
    description   TEXT,
    sort_order    INTEGER NOT NULL DEFAULT 0,
    created_at    DATETIME DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (from_agent_id) REFERENCES org_agents(id) ON DELETE CASCADE,
    FOREIGN KEY (to_agent_id)   REFERENCES org_agents(id) ON DELETE CASCADE,
    UNIQUE(from_agent_id, to_agent_id, link_type),
    CHECK(from_agent_id != to_agent_id)
);

CREATE INDEX IF NOT EXISTS idx_agent_links_to ON org_agent_links(to_agent_id);
