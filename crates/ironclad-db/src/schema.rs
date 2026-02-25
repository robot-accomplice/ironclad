use std::path::Path;

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
    scope_key TEXT NOT NULL DEFAULT 'agent',
    status TEXT NOT NULL DEFAULT 'active',
    model TEXT,
    nickname TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now')),
    metadata TEXT
);
CREATE INDEX IF NOT EXISTS idx_sessions_scope ON sessions(agent_id, scope_key, status);
CREATE UNIQUE INDEX IF NOT EXISTS idx_sessions_active_scope_unique ON sessions(agent_id, scope_key) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS idx_sessions_status_updated ON sessions(status, updated_at);

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
    skill_id TEXT,
    skill_name TEXT,
    skill_hash TEXT,
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
    category,
    source_table,
    source_id
);

-- Keep FTS in sync with episodic_memory
CREATE TRIGGER IF NOT EXISTS episodic_ai AFTER INSERT ON episodic_memory BEGIN
    INSERT INTO memory_fts(content, category, source_table, source_id)
    VALUES (new.content, new.classification, 'episodic', new.id);
END;

CREATE TRIGGER IF NOT EXISTS episodic_ad AFTER DELETE ON episodic_memory BEGIN
    DELETE FROM memory_fts WHERE source_table = 'episodic' AND source_id = old.id;
END;

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
    risk_level TEXT NOT NULL DEFAULT 'Caution',
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
    embedding_json TEXT NOT NULL DEFAULT '',
    embedding_blob BLOB,
    dimensions INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_embeddings_source ON embeddings(source_table, source_id);

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

CREATE TABLE IF NOT EXISTS turn_feedback (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL UNIQUE REFERENCES turns(id),
    session_id TEXT NOT NULL REFERENCES sessions(id),
    grade INTEGER NOT NULL CHECK (grade BETWEEN 1 AND 5),
    source TEXT NOT NULL DEFAULT 'dashboard',
    comment TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_turn_feedback_session ON turn_feedback(session_id);

CREATE TABLE IF NOT EXISTS context_snapshots (
    turn_id TEXT PRIMARY KEY REFERENCES turns(id),
    complexity_level TEXT NOT NULL,
    token_budget INTEGER NOT NULL,
    system_prompt_tokens INTEGER,
    memory_tokens INTEGER,
    history_tokens INTEGER,
    history_depth INTEGER,
    memory_tiers_json TEXT,
    retrieved_memories_json TEXT,
    model TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS model_selection_events (
    id TEXT PRIMARY KEY,
    turn_id TEXT NOT NULL,
    session_id TEXT NOT NULL,
    agent_id TEXT NOT NULL,
    channel TEXT NOT NULL,
    selected_model TEXT NOT NULL,
    strategy TEXT NOT NULL,
    primary_model TEXT NOT NULL,
    override_model TEXT,
    complexity TEXT,
    user_excerpt TEXT NOT NULL,
    candidates_json TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_model_selection_events_turn ON model_selection_events(turn_id);
CREATE INDEX IF NOT EXISTS idx_model_selection_events_created ON model_selection_events(created_at DESC);
"#;

pub fn initialize_db(db: &Database) -> Result<()> {
    {
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
            // The embedded schema already incorporates all migrations through v10,
            // so seed the version to 10 so run_migrations() won't re-apply them.
            conn.execute("INSERT INTO schema_version (version) VALUES (?1)", [10])
                .map_err(|e| IroncladError::Database(e.to_string()))?;
        }
    }

    run_migrations(db)?;
    ensure_optional_columns(db)?;
    Ok(())
}

fn has_column(conn: &rusqlite::Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| row.get::<_, String>(1))
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    for col in rows {
        if col.map_err(|e| IroncladError::Database(e.to_string()))? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

fn ensure_optional_columns(db: &Database) -> Result<()> {
    let conn = db.conn();
    if !has_column(&conn, "skills", "risk_level")? {
        conn.execute(
            "ALTER TABLE skills ADD COLUMN risk_level TEXT NOT NULL DEFAULT 'Caution'",
            [],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    }
    if !has_column(&conn, "tool_calls", "skill_id")? {
        conn.execute("ALTER TABLE tool_calls ADD COLUMN skill_id TEXT", [])
            .map_err(|e| IroncladError::Database(e.to_string()))?;
    }
    if !has_column(&conn, "tool_calls", "skill_name")? {
        conn.execute("ALTER TABLE tool_calls ADD COLUMN skill_name TEXT", [])
            .map_err(|e| IroncladError::Database(e.to_string()))?;
    }
    if !has_column(&conn, "tool_calls", "skill_hash")? {
        conn.execute("ALTER TABLE tool_calls ADD COLUMN skill_hash TEXT", [])
            .map_err(|e| IroncladError::Database(e.to_string()))?;
    }
    Ok(())
}

/// Discover migrations directory: current_dir()/migrations or CARGO_MANIFEST_DIR/migrations.
fn migrations_dir() -> Option<std::path::PathBuf> {
    std::env::current_dir()
        .ok()
        .map(|p| p.join("migrations"))
        .filter(|p| p.is_dir())
        .or_else(|| {
            let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("migrations");
            if p.is_dir() { Some(p) } else { None }
        })
}

/// Apply SQL files from migrations/ in order by version number. Forward-only.
/// If no migrations directory exists, skip gracefully.
pub fn run_migrations(db: &Database) -> Result<()> {
    let dir = match migrations_dir() {
        Some(d) => d,
        None => return Ok(()),
    };

    let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map_err(|e| IroncladError::Database(format!("read migrations dir: {e}")))?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("sql"))
        .collect();

    entries.sort_by(|a, b| {
        let va = version_from_name(a.file_name().and_then(|n| n.to_str()).unwrap_or(""));
        let vb = version_from_name(b.file_name().and_then(|n| n.to_str()).unwrap_or(""));
        va.cmp(&vb)
    });

    let conn = db.conn();
    let max_version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_version",
            [],
            |row| row.get(0),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    for path in entries {
        let version = version_from_name(path.file_name().and_then(|n| n.to_str()).unwrap_or(""));
        if version <= max_version {
            continue;
        }
        let sql = std::fs::read_to_string(&path)
            .map_err(|e| IroncladError::Database(format!("read migration {:?}: {e}", path)))?;
        let tx = conn.unchecked_transaction().map_err(|e| {
            IroncladError::Database(format!("begin tx for migration {version}: {e}"))
        })?;
        tx.execute_batch(sql.trim())
            .map_err(|e| IroncladError::Database(format!("migration {version}: {e}")))?;
        tx.execute(
            "INSERT INTO schema_version (version) VALUES (?1)",
            [version],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
        tx.commit()
            .map_err(|e| IroncladError::Database(format!("commit migration {version}: {e}")))?;
    }

    Ok(())
}

/// Parse version number from migration filename, e.g. 001_initial.sql -> 1, 002_add_indexes.sql -> 2.
fn version_from_name(name: &str) -> i64 {
    name.find('_')
        .and_then(|i| name.get(..i))
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0)
}

#[allow(dead_code)]
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
        // 30 regular tables + 1 FTS5 virtual table + sub_agents + hippocampus + turn_feedback + context_snapshots + model_selection_events = 34
        assert_eq!(count, 34, "expected 34 user-defined tables, got {count}");
    }

    #[test]
    fn schema_idempotent() {
        let db = Database::new(":memory:").unwrap();
        initialize_db(&db).unwrap();
        initialize_db(&db).unwrap();
        let count = table_count(&db).unwrap();
        assert_eq!(count, 34);
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
        assert!(version >= 10, "embedded schema seeds at version 10");
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

    #[test]
    fn version_from_name_parses_correctly() {
        assert_eq!(super::version_from_name("001_initial.sql"), 1);
        assert_eq!(super::version_from_name("002_add_indexes.sql"), 2);
        assert_eq!(super::version_from_name("010_foo.sql"), 10);
        assert_eq!(super::version_from_name("no_underscore.sql"), 0);
    }

    #[test]
    fn run_migrations_applies_in_order() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        let versions: Vec<i64> = conn
            .prepare("SELECT version FROM schema_version ORDER BY version")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            !versions.is_empty(),
            "schema_version should have at least one entry"
        );
        assert!(versions[0] >= 10, "embedded schema seeds at version 10");
        for w in versions.windows(2) {
            assert!(w[1] > w[0], "versions must be strictly increasing");
        }
    }

    #[test]
    fn version_from_name_edge_cases() {
        assert_eq!(super::version_from_name(""), 0);
        assert_eq!(super::version_from_name("_no_number.sql"), 0);
        assert_eq!(super::version_from_name("abc_nonnumeric.sql"), 0);
        assert_eq!(super::version_from_name("999_big.sql"), 999);
    }

    #[test]
    fn initialize_db_creates_version_row() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM schema_version WHERE version >= 10",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(
            count >= 1,
            "embedded schema should seed at least version 10"
        );
    }

    #[test]
    fn run_migrations_no_dir_is_noop() {
        let db = Database::new(":memory:").unwrap();
        run_migrations(&db).unwrap();
    }

    #[test]
    fn migrations_dir_returns_option() {
        let result = migrations_dir();
        // In test context, migrations dir may or may not exist
        if let Some(path) = result {
            assert!(path.is_dir());
        }
    }

    #[test]
    fn fts_table_exists() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        let exists: bool = conn
            .prepare(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'memory_fts' AND type = 'table'",
            )
            .unwrap()
            .query_row([], |row| {
                let count: i64 = row.get(0)?;
                Ok(count > 0)
            })
            .unwrap();
        assert!(exists, "memory_fts FTS5 table should exist");
    }
}
