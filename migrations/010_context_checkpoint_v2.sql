-- Context checkpoint for instant boot
CREATE TABLE IF NOT EXISTS context_checkpoints (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    system_prompt_hash TEXT NOT NULL,
    memory_summary TEXT NOT NULL,
    active_tasks TEXT,
    conversation_digest TEXT,
    turn_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_checkpoints_session ON context_checkpoints(session_id, created_at DESC);
