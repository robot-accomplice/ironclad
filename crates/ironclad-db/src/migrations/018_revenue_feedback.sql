CREATE TABLE IF NOT EXISTS revenue_feedback (
    id TEXT PRIMARY KEY,
    opportunity_id TEXT NOT NULL,
    strategy TEXT NOT NULL,
    grade REAL NOT NULL,
    source TEXT NOT NULL,
    comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_revenue_feedback_opportunity
    ON revenue_feedback(opportunity_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_revenue_feedback_strategy
    ON revenue_feedback(strategy, created_at DESC);
