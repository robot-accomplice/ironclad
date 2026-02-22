CREATE TABLE IF NOT EXISTS sub_agents (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    display_name TEXT,
    model TEXT NOT NULL DEFAULT '',
    role TEXT NOT NULL DEFAULT 'specialist',
    description TEXT,
    skills_json TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    session_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
