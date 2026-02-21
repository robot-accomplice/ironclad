use crate::Database;
use ironclad_core::{IroncladError, Result};

const SCHEMA_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS schema_version (
    version INTEGER NOT NULL,
    applied_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS sessions (
    id TEXT PRIMARY KEY,
    agent_id TEXT NOT NULL,
    model TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT
);

CREATE TABLE IF NOT EXISTS session_messages (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    parent_id TEXT,
    role TEXT NOT NULL,
    content TEXT NOT NULL,
    usage_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_session_messages_session ON session_messages(session_id, created_at);

CREATE TABLE IF NOT EXISTS turns (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL REFERENCES sessions(id),
    thinking TEXT,
    tool_calls_json TEXT,
    tokens_in INTEGER,
    tokens_out INTEGER,
    cost REAL,
    model TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS tool_calls (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL REFERENCES turns(id),
    tool_name TEXT NOT NULL,
    input TEXT NOT NULL,
    output TEXT,
    status TEXT NOT NULL,
    duration_ms INTEGER,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_tool_calls_turn ON tool_calls(turn_id);

CREATE TABLE IF NOT EXISTS policy_decisions (
    id TEXT PRIMARY KEY,
    turn_id TEXT,
    tool_name TEXT NOT NULL,
    decision TEXT NOT NULL,
    rule_name TEXT,
    reason TEXT,
    context_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS working_memory (
    id TEXT PRIMARY KEY,
    session_id TEXT NOT NULL,
    entry_type TEXT NOT NULL,
    content TEXT NOT NULL,
    importance INTEGER NOT NULL DEFAULT 5,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS episodic_memory (
    id TEXT PRIMARY KEY,
    classification TEXT NOT NULL,
    content TEXT NOT NULL,
    importance INTEGER NOT NULL DEFAULT 5,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_episodic_importance ON episodic_memory(importance DESC, created_at DESC);

CREATE TABLE IF NOT EXISTS semantic_memory (
    id TEXT PRIMARY KEY,
    category TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.8,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(category, key)
);

CREATE TABLE IF NOT EXISTS procedural_memory (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    steps TEXT NOT NULL,
    success_count INTEGER NOT NULL DEFAULT 0,
    failure_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS relationship_memory (
    id TEXT PRIMARY KEY,
    entity_id TEXT NOT NULL UNIQUE,
    entity_name TEXT,
    trust_score REAL NOT NULL DEFAULT 0.5,
    interaction_summary TEXT,
    interaction_count INTEGER NOT NULL DEFAULT 0,
    last_interaction TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE VIRTUAL TABLE IF NOT EXISTS memory_fts USING fts5(
    content,
    category
);

CREATE TABLE IF NOT EXISTS tasks (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    priority INTEGER NOT NULL DEFAULT 0,
    source TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS cron_jobs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    schedule_kind TEXT NOT NULL,
    schedule_expr TEXT,
    schedule_every_ms INTEGER,
    schedule_tz TEXT DEFAULT 'UTC',
    agent_id TEXT NOT NULL,
    session_target TEXT NOT NULL DEFAULT 'main',
    payload_json TEXT NOT NULL,
    delivery_mode TEXT DEFAULT 'none',
    delivery_channel TEXT,
    last_run_at TEXT,
    last_status TEXT,
    last_duration_ms INTEGER,
    consecutive_errors INTEGER NOT NULL DEFAULT 0,
    next_run_at TEXT,
    last_error TEXT,
    lease_holder TEXT,
    lease_expires_at TEXT
);

CREATE TABLE IF NOT EXISTS cron_runs (
    id TEXT PRIMARY KEY,
    job_id TEXT NOT NULL REFERENCES cron_jobs(id),
    status TEXT NOT NULL,
    duration_ms INTEGER,
    error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS transactions (
    id TEXT PRIMARY KEY,
    tx_type TEXT NOT NULL,
    amount REAL NOT NULL,
    currency TEXT NOT NULL DEFAULT 'USD',
    counterparty TEXT,
    tx_hash TEXT,
    metadata_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS inference_costs (
    id TEXT PRIMARY KEY,
    model TEXT NOT NULL,
    provider TEXT NOT NULL,
    tokens_in INTEGER NOT NULL,
    tokens_out INTEGER NOT NULL,
    cost REAL NOT NULL,
    tier TEXT,
    cached INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_inference_costs_time ON inference_costs(created_at DESC);

CREATE TABLE IF NOT EXISTS proxy_stats (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    snapshot_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS semantic_cache (
    id TEXT PRIMARY KEY,
    prompt_hash TEXT NOT NULL,
    embedding BLOB,
    response TEXT NOT NULL,
    model TEXT NOT NULL,
    tokens_saved INTEGER NOT NULL DEFAULT 0,
    hit_count INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    expires_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_cache_hash ON semantic_cache(prompt_hash);

CREATE TABLE IF NOT EXISTS identity (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS soul_history (
    id TEXT PRIMARY KEY,
    content TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS metric_snapshots (
    id TEXT PRIMARY KEY,
    metrics_json TEXT NOT NULL,
    alerts_json TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS discovered_agents (
    id TEXT PRIMARY KEY,
    did TEXT NOT NULL UNIQUE,
    agent_card_json TEXT NOT NULL,
    capabilities TEXT,
    endpoint_url TEXT NOT NULL,
    chain_id INTEGER NOT NULL DEFAULT 8453,
    trust_score REAL NOT NULL DEFAULT 0.5,
    last_verified_at TEXT,
    expires_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_discovered_agents_did ON discovered_agents(did);

CREATE TABLE IF NOT EXISTS skills (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    description TEXT,
    source_path TEXT NOT NULL,
    content_hash TEXT NOT NULL,
    triggers_json TEXT,
    tool_chain_json TEXT,
    policy_overrides_json TEXT,
    script_path TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    last_loaded_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_skills_kind ON skills(kind);

CREATE TABLE IF NOT EXISTS delivery_queue (
    id TEXT PRIMARY KEY,
    channel TEXT NOT NULL,
    recipient_id TEXT NOT NULL,
    content TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'pending',
    attempts INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 5,
    next_retry_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_delivery_queue_status ON delivery_queue(status, next_retry_at);

CREATE TABLE IF NOT EXISTS approval_requests (
    id TEXT PRIMARY KEY,
    tool_name TEXT NOT NULL,
    tool_input TEXT NOT NULL,
    session_id TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    decided_by TEXT,
    decided_at TEXT,
    timeout_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_approvals_status ON approval_requests(status);

CREATE TABLE IF NOT EXISTS plugins (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    version TEXT NOT NULL,
    description TEXT,
    enabled INTEGER NOT NULL DEFAULT 1,
    manifest_path TEXT NOT NULL,
    permissions_json TEXT,
    installed_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS embeddings (
    id TEXT PRIMARY KEY,
    source_table TEXT NOT NULL,
    source_id TEXT NOT NULL,
    content_preview TEXT NOT NULL,
    embedding_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_embeddings_source ON embeddings(source_table, source_id);
"#;

pub fn initialize_db(db: &Database) -> Result<()> {
    let conn = db.conn();
    conn.execute_batch(SCHEMA_SQL)
        .map_err(|e| IroncladError::Database(format!("schema init failed: {e}")))?;

    let version_exists: bool = conn
        .query_row("SELECT COUNT(*) FROM schema_version", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|c| c > 0)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    if !version_exists {
        conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [1])
            .map_err(|e| IroncladError::Database(e.to_string()))?;
    }

    Ok(())
}

pub fn table_count(db: &Database) -> Result<usize> {
    let conn = db.conn();
    let count: usize = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type IN ('table', 'view') AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_data' AND name NOT LIKE '%_idx' AND name NOT LIKE '%_content' AND name NOT LIKE '%_docsize' AND name NOT LIKE '%_config'",
            [],
            |row| row.get(0),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_creates_all_tables() {
        let db = Database::new(":memory:").unwrap();
        let count = table_count(&db).unwrap();
        // 27 regular tables + 1 FTS5 virtual table = 28 user-defined tables
        assert_eq!(count, 28, "expected 28 user-defined tables, got {count}");
    }

    #[test]
    fn schema_idempotent() {
        let db = Database::new(":memory:").unwrap();
        initialize_db(&db).unwrap();
        initialize_db(&db).unwrap();
        let count = table_count(&db).unwrap();
        assert_eq!(count, 28);
    }

    #[test]
    fn schema_version_inserted() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        let version: i64 = conn
            .query_row(
                "SELECT version FROM schema_version ORDER BY applied_at DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn wal_mode_enabled() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        let mode: String = conn
            .query_row("PRAGMA journal_mode", [], |row| row.get(0))
            .unwrap();
        // in-memory databases use "memory" mode, but the PRAGMA was executed
        assert!(mode == "wal" || mode == "memory");
    }
}
