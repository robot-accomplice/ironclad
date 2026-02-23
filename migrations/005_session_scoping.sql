-- Session scoping: add scope_key and status to sessions
ALTER TABLE sessions ADD COLUMN scope_key TEXT;
ALTER TABLE sessions ADD COLUMN status TEXT NOT NULL DEFAULT 'active';
CREATE INDEX IF NOT EXISTS idx_sessions_scope ON sessions(agent_id, scope_key, status);
