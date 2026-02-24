-- Normalize legacy unscoped sessions into explicit agent scope.
UPDATE sessions
SET scope_key = 'agent'
WHERE scope_key IS NULL;

-- Keep one active session per (agent_id, scope_key), archive older duplicates.
WITH ranked AS (
    SELECT
        id,
        ROW_NUMBER() OVER (
            PARTITION BY agent_id, scope_key
            ORDER BY datetime(updated_at) DESC, datetime(created_at) DESC, id DESC
        ) AS rn
    FROM sessions
    WHERE status = 'active'
)
UPDATE sessions
SET status = 'archived',
    updated_at = datetime('now')
WHERE id IN (SELECT id FROM ranked WHERE rn > 1);

-- Enforce uniqueness for active scoped sessions.
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_active_scope_unique
ON sessions(agent_id, scope_key)
WHERE status = 'active';

-- Helper index for lifecycle sweeps.
CREATE INDEX IF NOT EXISTS idx_sessions_status_updated
ON sessions(status, updated_at);
