CREATE TABLE IF NOT EXISTS service_requests (
    id TEXT PRIMARY KEY,
    service_id TEXT NOT NULL,
    requester TEXT NOT NULL,
    parameters_json TEXT NOT NULL,
    status TEXT NOT NULL,
    quoted_amount REAL NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USDC',
    recipient TEXT NOT NULL,
    quote_expires_at TEXT NOT NULL,
    payment_tx_hash TEXT,
    paid_amount REAL,
    payment_verified_at TEXT,
    fulfillment_output TEXT,
    fulfilled_at TEXT,
    failure_reason TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_service_requests_status ON service_requests(status, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_service_requests_service ON service_requests(service_id, created_at DESC);
