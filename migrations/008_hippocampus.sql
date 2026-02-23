-- Hippocampus: self-describing schema map for agent introspection
CREATE TABLE IF NOT EXISTS hippocampus (
    table_name TEXT PRIMARY KEY,
    description TEXT NOT NULL,
    columns_json TEXT NOT NULL,
    created_by TEXT NOT NULL DEFAULT 'system',
    agent_owned INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_hippocampus_agent ON hippocampus(created_by, agent_owned);
