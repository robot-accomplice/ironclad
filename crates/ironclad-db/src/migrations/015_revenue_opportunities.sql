CREATE TABLE IF NOT EXISTS revenue_opportunities (
    id TEXT PRIMARY KEY,
    source TEXT NOT NULL,
    strategy TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    expected_revenue_usdc REAL NOT NULL,
    status TEXT NOT NULL,
    qualification_reason TEXT,
    plan_json TEXT,
    evidence_json TEXT,
    request_id TEXT,
    settlement_ref TEXT UNIQUE,
    settled_amount_usdc REAL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_revenue_opportunities_status ON revenue_opportunities(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_revenue_opportunities_strategy ON revenue_opportunities(strategy, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_revenue_opportunities_request ON revenue_opportunities(request_id);
