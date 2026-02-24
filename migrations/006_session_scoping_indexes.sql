CREATE INDEX IF NOT EXISTS idx_sessions_scope ON sessions(agent_id, scope_key, status);
