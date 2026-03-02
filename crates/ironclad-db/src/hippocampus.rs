use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

/// A schema map entry describing a table in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaEntry {
    pub table_name: String,
    pub description: String,
    pub columns: Vec<ColumnDef>,
    pub created_by: String,
    pub agent_owned: bool,
    pub created_at: String,
    pub updated_at: String,
    pub access_level: String,
    pub row_count: i64,
}

/// Column definition within a schema entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnDef {
    pub name: String,
    pub col_type: String,
    pub nullable: bool,
    pub description: Option<String>,
}

/// Register a table in the hippocampus.
pub fn register_table(
    db: &Database,
    table_name: &str,
    description: &str,
    columns: &[ColumnDef],
    created_by: &str,
    agent_owned: bool,
    access_level: &str,
    row_count: i64,
) -> Result<()> {
    let conn = db.conn();
    let columns_json =
        serde_json::to_string(columns).map_err(|e| IroncladError::Database(e.to_string()))?;

    conn.execute(
        "INSERT OR REPLACE INTO hippocampus \
         (table_name, description, columns_json, created_by, agent_owned, access_level, row_count, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, datetime('now'))",
        rusqlite::params![
            table_name,
            description,
            columns_json,
            created_by,
            agent_owned as i32,
            access_level,
            row_count
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

const SELECT_COLS: &str = "table_name, description, columns_json, created_by, agent_owned, \
                           created_at, updated_at, access_level, row_count";

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<SchemaEntry> {
    let columns_json: String = row.get(2)?;
    let columns: Vec<ColumnDef> = serde_json::from_str(&columns_json).unwrap_or_else(|e| {
        tracing::warn!(error = %e, "failed to deserialize column definitions, using empty list");
        Vec::new()
    });
    Ok(SchemaEntry {
        table_name: row.get(0)?,
        description: row.get(1)?,
        columns,
        created_by: row.get(3)?,
        agent_owned: row.get::<_, i32>(4)? != 0,
        created_at: row.get(5)?,
        updated_at: row.get(6)?,
        access_level: row.get::<_, Option<String>>(7)?.unwrap_or_else(|| "internal".into()),
        row_count: row.get::<_, Option<i64>>(8)?.unwrap_or(0),
    })
}

/// Look up a table's schema entry.
pub fn get_table(db: &Database, table_name: &str) -> Result<Option<SchemaEntry>> {
    let conn = db.conn();
    conn.query_row(
        &format!("SELECT {SELECT_COLS} FROM hippocampus WHERE table_name = ?1"),
        [table_name],
        row_to_entry,
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

/// List all tables in the hippocampus.
pub fn list_tables(db: &Database) -> Result<Vec<SchemaEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLS} FROM hippocampus ORDER BY table_name"
        ))
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], row_to_entry)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// List only agent-created tables.
pub fn list_agent_tables(db: &Database, agent_id: &str) -> Result<Vec<SchemaEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {SELECT_COLS} FROM hippocampus WHERE agent_owned = 1 AND created_by = ?1 ORDER BY table_name"
        ))
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([agent_id], row_to_entry)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

fn validate_identifier(s: &str) -> Result<()> {
    if s.is_empty()
        || s.chars().next().is_some_and(|c| c.is_ascii_digit())
        || !s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(IroncladError::Database(format!(
            "invalid SQL identifier: {s}"
        )));
    }
    Ok(())
}

/// Create an agent-owned table with the given columns.
/// Table names are prefixed with the agent ID for isolation.
pub fn create_agent_table(
    db: &Database,
    agent_id: &str,
    table_suffix: &str,
    description: &str,
    columns: &[ColumnDef],
) -> Result<String> {
    let table_name = format!("{agent_id}_{table_suffix}");

    if !table_name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(IroncladError::Database(
            "table name contains invalid characters".into(),
        ));
    }

    for col in columns {
        validate_identifier(&col.name)?;
        validate_identifier(&col.col_type)?;
    }

    let col_defs: Vec<String> = columns
        .iter()
        .map(|c| {
            let null = if c.nullable { "" } else { " NOT NULL" };
            format!("{} {}{}", c.name, c.col_type, null)
        })
        .collect();

    let middle = if col_defs.is_empty() {
        String::new()
    } else {
        format!(", {}", col_defs.join(", "))
    };

    let create_sql = format!(
        "CREATE TABLE IF NOT EXISTS \"{}\" (id TEXT PRIMARY KEY{}, created_at TEXT NOT NULL DEFAULT (datetime('now')))",
        table_name, middle
    );

    {
        let conn = db.conn();
        conn.execute(&create_sql, [])
            .map_err(|e| IroncladError::Database(e.to_string()))?;
    }

    register_table(
        db,
        &table_name,
        description,
        columns,
        agent_id,
        true,
        "readwrite",
        0,
    )?;

    Ok(table_name)
}

/// Drop an agent-owned table. Only tables created by the specified agent can be dropped.
/// Auth check + DROP + registry DELETE are performed in a single transaction to prevent TOCTOU.
pub fn drop_agent_table(db: &Database, agent_id: &str, table_name: &str) -> Result<()> {
    validate_identifier(table_name)?;

    let conn = db.conn();
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    // Atomic check: verify ownership inside the transaction
    let owned: bool = tx
        .query_row(
            "SELECT agent_owned AND created_by = ?2 FROM hippocampus WHERE table_name = ?1",
            rusqlite::params![table_name, agent_id],
            |row| row.get(0),
        )
        .map_err(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => {
                IroncladError::Database(format!("table {table_name} not found in hippocampus"))
            }
            other => IroncladError::Database(other.to_string()),
        })?;

    if !owned {
        return Err(IroncladError::Database(
            "cannot drop: table not owned by this agent".into(),
        ));
    }

    tx.execute(&format!("DROP TABLE IF EXISTS \"{}\"", table_name), [])
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    tx.execute(
        "DELETE FROM hippocampus WHERE table_name = ?1",
        [table_name],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

/// Generate a schema map summary for injection into the agent's context.
pub fn schema_summary(db: &Database) -> Result<String> {
    let tables = list_tables(db)?;
    if tables.is_empty() {
        return Ok("No tables registered in hippocampus.".into());
    }

    let mut summary = String::from("## Database Schema Map\n\n");
    for entry in &tables {
        let owner = if entry.agent_owned {
            format!(" (owned by: {})", entry.created_by)
        } else {
            " (system)".to_string()
        };
        summary.push_str(&format!(
            "### {}{} [{}, {} rows]\n",
            entry.table_name, owner, entry.access_level, entry.row_count
        ));
        summary.push_str(&format!("{}\n", entry.description));
        for col in &entry.columns {
            let null_str = if col.nullable { ", nullable" } else { "" };
            let desc = col.description.as_deref().unwrap_or("");
            summary.push_str(&format!(
                "- `{}` ({}{}){}\n",
                col.name,
                col.col_type,
                null_str,
                if desc.is_empty() {
                    String::new()
                } else {
                    format!(" — {desc}")
                }
            ));
        }
        summary.push('\n');
    }
    Ok(summary)
}

/// Return (description, access_level) for known system tables.
fn system_table_metadata(table_name: &str) -> (&'static str, &'static str) {
    match table_name {
        "schema_version" => ("Schema migration version tracking", "internal"),
        "sessions" => ("User conversation sessions", "read"),
        "session_messages" => ("Messages within sessions", "read"),
        "turns" => ("Conversation turn tracking", "internal"),
        "tool_calls" => ("Tool invocation log", "read"),
        "policy_decisions" => ("Policy evaluation results", "internal"),
        "working_memory" => ("Session-scoped working memory", "read"),
        "episodic_memory" => ("Long-term event memory", "read"),
        "semantic_memory" => ("Factual knowledge store", "read"),
        "procedural_memory" => ("Learned procedure memory", "read"),
        "relationship_memory" => ("Entity relationship memory", "read"),
        "tasks" => ("Task queue for agent work items", "read"),
        "cron_jobs" => ("Scheduled cron jobs", "read"),
        "cron_runs" => ("Cron job execution history", "read"),
        "transactions" => ("Wallet transaction log", "internal"),
        "inference_costs" => ("LLM inference cost tracking", "internal"),
        "proxy_stats" => ("API proxy statistics", "internal"),
        "semantic_cache" => ("Semantic response cache", "internal"),
        "identity" => ("Agent identity and credentials", "internal"),
        "soul_history" => ("Agent personality evolution log", "internal"),
        "metric_snapshots" => ("System metric snapshots", "internal"),
        "discovered_agents" => ("Discovered peer agents", "read"),
        "skills" => ("Registered agent skills", "read"),
        "delivery_queue" => ("Durable message delivery queue", "internal"),
        "approval_requests" => ("Pending human approval requests", "read"),
        "plugins" => ("Installed plugins", "read"),
        "embeddings" => ("Vector embeddings store", "internal"),
        "sub_agents" => ("Spawned sub-agent registry", "read"),
        "context_checkpoints" => ("Context checkpoint snapshots", "internal"),
        "hippocampus" => ("Schema map (this table)", "internal"),
        _ => ("Agent-managed table", "readwrite"),
    }
}

/// Introspect columns of a table via `PRAGMA table_info`.
fn introspect_columns(
    conn: &rusqlite::Connection,
    table_name: &str,
) -> std::result::Result<Vec<ColumnDef>, rusqlite::Error> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{}\")", table_name))?;
    let cols = stmt.query_map([], |row| {
        let name: String = row.get(1)?;
        let col_type: String = row.get(2)?;
        let notnull: i32 = row.get(3)?;
        Ok(ColumnDef {
            name,
            col_type,
            nullable: notnull == 0,
            description: None,
        })
    })?;
    cols.collect()
}

/// Bootstrap the hippocampus by auto-discovering all tables in the database,
/// introspecting their columns, and registering them. Also runs a consistency
/// check to remove stale entries for tables that no longer exist.
pub fn bootstrap_hippocampus(db: &Database) -> Result<()> {
    // Phase 1: Discover all tables and collect metadata
    let table_data: Vec<(String, Vec<ColumnDef>, i64)> = {
        let conn = db.conn();
        let mut stmt = conn
            .prepare(
                "SELECT name FROM sqlite_master \
                 WHERE type = 'table' AND name NOT LIKE 'sqlite_%' \
                 ORDER BY name",
            )
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let table_names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .map_err(|e| IroncladError::Database(e.to_string()))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let mut data = Vec::with_capacity(table_names.len());
        for name in table_names {
            let columns = introspect_columns(&conn, &name)
                .map_err(|e| IroncladError::Database(e.to_string()))?;

            let row_count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM \"{}\"", name), [], |row| {
                    row.get(0)
                })
                .unwrap_or(0);

            data.push((name, columns, row_count));
        }
        data
    };

    // Phase 2: Register each table (connection released, register_table acquires its own)
    for (name, columns, row_count) in &table_data {
        let (description, access_level) = system_table_metadata(name);

        // Preserve existing agent-owned entries — only upsert system tables
        if let Some(existing) = get_table(db, name)? {
            if existing.agent_owned {
                // Update row_count only for agent-owned tables, keep their metadata
                register_table(
                    db,
                    name,
                    &existing.description,
                    columns,
                    &existing.created_by,
                    true,
                    &existing.access_level,
                    *row_count,
                )?;
                continue;
            }
        }

        register_table(
            db,
            name,
            description,
            columns,
            "system",
            false,
            access_level,
            *row_count,
        )?;
    }

    // Phase 3: Consistency check — remove stale entries for non-existent tables
    let registered = list_tables(db)?;
    let existing_names: std::collections::HashSet<&str> =
        table_data.iter().map(|(n, _, _)| n.as_str()).collect();

    for entry in &registered {
        if !existing_names.contains(entry.table_name.as_str()) {
            tracing::warn!(
                table = %entry.table_name,
                "hippocampus entry for missing table, removing"
            );
            let conn = db.conn();
            conn.execute(
                "DELETE FROM hippocampus WHERE table_name = ?1",
                [&entry.table_name],
            )
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        }
    }

    tracing::info!(
        tables = table_data.len(),
        "hippocampus bootstrapped with schema map"
    );
    Ok(())
}

/// Seed the hippocampus with entries for core system tables (legacy helper).
pub fn seed_system_tables(db: &Database) -> Result<()> {
    let system_tables = vec![
        (
            "sessions",
            "User conversation sessions",
            vec![
                ColumnDef {
                    name: "id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Primary key".into()),
                },
                ColumnDef {
                    name: "agent_id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Owning agent".into()),
                },
                ColumnDef {
                    name: "scope_key".into(),
                    col_type: "TEXT".into(),
                    nullable: true,
                    description: Some("Session scope identifier".into()),
                },
                ColumnDef {
                    name: "status".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("active/archived/expired".into()),
                },
            ],
        ),
        (
            "episodic_memory",
            "Long-term event memory",
            vec![
                ColumnDef {
                    name: "id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Primary key".into()),
                },
                ColumnDef {
                    name: "classification".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Memory category".into()),
                },
                ColumnDef {
                    name: "content".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Memory content".into()),
                },
                ColumnDef {
                    name: "importance".into(),
                    col_type: "INTEGER".into(),
                    nullable: false,
                    description: Some("1-10 importance score".into()),
                },
            ],
        ),
        (
            "semantic_memory",
            "Factual knowledge store",
            vec![
                ColumnDef {
                    name: "id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Primary key".into()),
                },
                ColumnDef {
                    name: "category".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Knowledge category".into()),
                },
                ColumnDef {
                    name: "key".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Fact key".into()),
                },
                ColumnDef {
                    name: "value".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Fact value".into()),
                },
            ],
        ),
        (
            "working_memory",
            "Session-scoped working memory",
            vec![
                ColumnDef {
                    name: "id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Primary key".into()),
                },
                ColumnDef {
                    name: "session_id".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Associated session".into()),
                },
                ColumnDef {
                    name: "entry_type".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Type of entry".into()),
                },
                ColumnDef {
                    name: "content".into(),
                    col_type: "TEXT".into(),
                    nullable: false,
                    description: Some("Entry content".into()),
                },
            ],
        ),
    ];

    for (name, desc, cols) in system_tables {
        register_table(db, name, desc, &cols, "system", false, "read", 0)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn register_and_get_table() {
        let db = test_db();
        let cols = vec![
            ColumnDef {
                name: "name".into(),
                col_type: "TEXT".into(),
                nullable: false,
                description: Some("User name".into()),
            },
            ColumnDef {
                name: "age".into(),
                col_type: "INTEGER".into(),
                nullable: true,
                description: None,
            },
        ];
        register_table(&db, "users", "User records", &cols, "system", false, "read", 0).unwrap();

        let entry = get_table(&db, "users").unwrap().unwrap();
        assert_eq!(entry.table_name, "users");
        assert_eq!(entry.description, "User records");
        assert_eq!(entry.columns.len(), 2);
        assert!(!entry.agent_owned);
        assert_eq!(entry.access_level, "read");
        assert_eq!(entry.row_count, 0);
    }

    #[test]
    fn get_table_not_found() {
        let db = test_db();
        assert!(get_table(&db, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn list_tables_includes_bootstrap() {
        let db = test_db();
        // Database::new runs bootstrap_hippocampus, so system tables are already registered
        let tables = list_tables(&db).unwrap();
        assert!(
            tables.len() >= 20,
            "bootstrap should register system tables, got {}",
            tables.len()
        );
    }

    #[test]
    fn list_tables_grows_with_registration() {
        let db = test_db();
        let before = list_tables(&db).unwrap().len();
        register_table(&db, "custom_a", "Table A", &[], "test", false, "internal", 0).unwrap();
        register_table(&db, "custom_b", "Table B", &[], "test", false, "internal", 0).unwrap();
        let after = list_tables(&db).unwrap().len();
        assert_eq!(after, before + 2);
    }

    #[test]
    fn create_agent_table_success() {
        let db = test_db();
        let cols = vec![
            ColumnDef {
                name: "key".into(),
                col_type: "TEXT".into(),
                nullable: false,
                description: None,
            },
            ColumnDef {
                name: "value".into(),
                col_type: "TEXT".into(),
                nullable: true,
                description: None,
            },
        ];
        let table_name = create_agent_table(&db, "agent42", "notes", "Agent notes", &cols).unwrap();
        assert_eq!(table_name, "agent42_notes");

        let entry = get_table(&db, "agent42_notes").unwrap().unwrap();
        assert!(entry.agent_owned);
        assert_eq!(entry.created_by, "agent42");
        assert_eq!(entry.access_level, "readwrite");
    }

    #[test]
    fn create_agent_table_invalid_chars() {
        let db = test_db();
        let result = create_agent_table(&db, "agent", "bad;name", "test", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn drop_agent_table_success() {
        let db = test_db();
        create_agent_table(&db, "agent1", "temp", "temp table", &[]).unwrap();
        drop_agent_table(&db, "agent1", "agent1_temp").unwrap();
        assert!(get_table(&db, "agent1_temp").unwrap().is_none());
    }

    #[test]
    fn drop_agent_table_wrong_owner() {
        let db = test_db();
        create_agent_table(&db, "agent1", "data", "data", &[]).unwrap();
        let result = drop_agent_table(&db, "agent2", "agent1_data");
        assert!(result.is_err());
    }

    #[test]
    fn drop_system_table_fails() {
        let db = test_db();
        register_table(&db, "sessions", "Sessions", &[], "system", false, "read", 0).unwrap();
        let result = drop_agent_table(&db, "agent1", "sessions");
        assert!(result.is_err());
    }

    #[test]
    fn list_agent_tables_filters() {
        let db = test_db();
        register_table(&db, "sessions", "System", &[], "system", false, "read", 0).unwrap();
        create_agent_table(&db, "agent1", "notes", "Notes", &[]).unwrap();
        create_agent_table(&db, "agent2", "data", "Data", &[]).unwrap();

        let agent1_tables = list_agent_tables(&db, "agent1").unwrap();
        assert_eq!(agent1_tables.len(), 1);
        assert_eq!(agent1_tables[0].table_name, "agent1_notes");
    }

    #[test]
    fn schema_summary_after_bootstrap() {
        let db = test_db();
        let summary = schema_summary(&db).unwrap();
        // Bootstrap runs at init, so summary is never empty
        assert!(summary.contains("## Database Schema Map"));
        assert!(summary.contains("sessions"));
    }

    #[test]
    fn schema_summary_with_tables() {
        let db = test_db();
        seed_system_tables(&db).unwrap();
        let summary = schema_summary(&db).unwrap();
        assert!(summary.contains("sessions"));
        assert!(summary.contains("episodic_memory"));
        assert!(summary.contains("(system)"));
        assert!(summary.contains("[read, 0 rows]"));
    }

    #[test]
    fn seed_system_tables_upserts_over_bootstrap() {
        let db = test_db();
        let before = list_tables(&db).unwrap().len();
        seed_system_tables(&db).unwrap();
        let after = list_tables(&db).unwrap().len();
        // seed_system_tables covers 4 tables already registered by bootstrap — no new entries
        assert_eq!(before, after, "seed should upsert, not add duplicates");
    }

    #[test]
    fn register_table_upsert() {
        let db = test_db();
        register_table(&db, "test", "Version 1", &[], "system", false, "internal", 0).unwrap();
        register_table(&db, "test", "Version 2", &[], "system", false, "read", 42).unwrap();

        let entry = get_table(&db, "test").unwrap().unwrap();
        assert_eq!(entry.description, "Version 2");
        assert_eq!(entry.access_level, "read");
        assert_eq!(entry.row_count, 42);
    }

    #[test]
    fn column_def_serialization() {
        let col = ColumnDef {
            name: "test_col".into(),
            col_type: "TEXT".into(),
            nullable: true,
            description: Some("A test column".into()),
        };
        let json = serde_json::to_string(&col).unwrap();
        let decoded: ColumnDef = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.name, "test_col");
        assert!(decoded.nullable);
    }

    #[test]
    fn validate_identifier_valid() {
        validate_identifier("hello").unwrap();
        validate_identifier("my_table").unwrap();
        validate_identifier("col123").unwrap();
        validate_identifier("A").unwrap();
    }

    #[test]
    fn validate_identifier_empty_fails() {
        assert!(validate_identifier("").is_err());
    }

    #[test]
    fn validate_identifier_special_chars_fail() {
        assert!(validate_identifier("name;drop").is_err());
        assert!(validate_identifier("col name").is_err());
        assert!(validate_identifier("table-name").is_err());
        assert!(validate_identifier("col.name").is_err());
    }

    #[test]
    fn create_agent_table_empty_columns() {
        let db = test_db();
        let name = create_agent_table(&db, "agent", "empty", "No columns", &[]).unwrap();
        assert_eq!(name, "agent_empty");

        // Table should have at least id and created_at
        let conn = db.conn();
        conn.execute("INSERT INTO \"agent_empty\" (id) VALUES ('row1')", [])
            .unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM \"agent_empty\"", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn create_agent_table_invalid_column_name() {
        let db = test_db();
        let cols = vec![ColumnDef {
            name: "bad;col".into(),
            col_type: "TEXT".into(),
            nullable: false,
            description: None,
        }];
        let result = create_agent_table(&db, "agent", "badcol", "test", &cols);
        assert!(result.is_err());
    }

    #[test]
    fn create_agent_table_invalid_column_type() {
        let db = test_db();
        let cols = vec![ColumnDef {
            name: "good_col".into(),
            col_type: "TEXT;DROP".into(),
            nullable: false,
            description: None,
        }];
        let result = create_agent_table(&db, "agent", "badtype", "test", &cols);
        assert!(result.is_err());
    }

    #[test]
    fn drop_agent_table_nonexistent() {
        let db = test_db();
        let result = drop_agent_table(&db, "agent", "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn schema_summary_with_agent_owned_table() {
        let db = test_db();
        let cols = vec![
            ColumnDef {
                name: "note".into(),
                col_type: "TEXT".into(),
                nullable: false,
                description: Some("The note content".into()),
            },
            ColumnDef {
                name: "priority".into(),
                col_type: "INTEGER".into(),
                nullable: true,
                description: None,
            },
        ];
        create_agent_table(&db, "agent1", "notes", "Agent notes storage", &cols).unwrap();

        let summary = schema_summary(&db).unwrap();
        assert!(
            summary.contains("(owned by: agent1)"),
            "summary should show agent owner"
        );
        assert!(
            summary.contains("note"),
            "summary should include column names"
        );
        assert!(
            summary.contains("nullable"),
            "nullable columns should be marked"
        );
        assert!(
            summary.contains("The note content"),
            "column descriptions should appear"
        );
        assert!(
            summary.contains("[readwrite, 0 rows]"),
            "summary should show access level and row count"
        );
    }

    #[test]
    fn schema_summary_column_without_description() {
        let db = test_db();
        let cols = vec![ColumnDef {
            name: "val".into(),
            col_type: "REAL".into(),
            nullable: false,
            description: None,
        }];
        register_table(&db, "metrics", "Metric values", &cols, "system", false, "internal", 0)
            .unwrap();

        let summary = schema_summary(&db).unwrap();
        assert!(summary.contains("`val` (REAL)"));
    }

    #[test]
    fn list_agent_tables_empty_for_unknown_agent() {
        let db = test_db();
        create_agent_table(&db, "agent1", "data", "Data", &[]).unwrap();
        let tables = list_agent_tables(&db, "agent_unknown").unwrap();
        assert!(tables.is_empty());
    }

    #[test]
    fn seed_system_tables_idempotent() {
        let db = test_db();
        let baseline = list_tables(&db).unwrap().len();
        seed_system_tables(&db).unwrap();
        seed_system_tables(&db).unwrap();
        let after = list_tables(&db).unwrap().len();
        assert_eq!(
            baseline, after,
            "seeding twice should not create duplicates"
        );
    }

    #[test]
    fn bootstrap_discovers_all_system_tables() {
        let db = test_db();
        bootstrap_hippocampus(&db).unwrap();

        let tables = list_tables(&db).unwrap();
        // The :memory: database created via Database::new runs initialize_db which creates
        // all system tables. bootstrap_hippocampus should discover them all.
        assert!(
            tables.len() >= 20,
            "expected at least 20 system tables, got {}",
            tables.len()
        );

        // Check specific known tables
        let names: Vec<&str> = tables.iter().map(|t| t.table_name.as_str()).collect();
        assert!(names.contains(&"sessions"), "missing sessions table");
        assert!(
            names.contains(&"inference_costs"),
            "missing inference_costs table"
        );
        assert!(
            names.contains(&"hippocampus"),
            "missing hippocampus table itself"
        );
    }

    #[test]
    fn bootstrap_introspects_columns() {
        let db = test_db();
        bootstrap_hippocampus(&db).unwrap();

        let entry = get_table(&db, "sessions").unwrap().unwrap();
        assert!(
            !entry.columns.is_empty(),
            "sessions should have introspected columns"
        );
        let col_names: Vec<&str> = entry.columns.iter().map(|c| c.name.as_str()).collect();
        assert!(col_names.contains(&"id"), "sessions should have id column");
        assert!(
            col_names.contains(&"agent_id"),
            "sessions should have agent_id column"
        );
    }

    #[test]
    fn bootstrap_sets_access_levels() {
        let db = test_db();
        bootstrap_hippocampus(&db).unwrap();

        let sessions = get_table(&db, "sessions").unwrap().unwrap();
        assert_eq!(sessions.access_level, "read");

        let inference = get_table(&db, "inference_costs").unwrap().unwrap();
        assert_eq!(inference.access_level, "internal");
    }

    #[test]
    fn bootstrap_preserves_agent_tables() {
        let db = test_db();
        create_agent_table(&db, "agent1", "notes", "My notes", &[]).unwrap();
        bootstrap_hippocampus(&db).unwrap();

        let entry = get_table(&db, "agent1_notes").unwrap().unwrap();
        assert!(entry.agent_owned);
        assert_eq!(entry.created_by, "agent1");
        assert_eq!(entry.description, "My notes");
        assert_eq!(entry.access_level, "readwrite");
    }

    #[test]
    fn bootstrap_idempotent() {
        let db = test_db();
        bootstrap_hippocampus(&db).unwrap();
        let count1 = list_tables(&db).unwrap().len();
        bootstrap_hippocampus(&db).unwrap();
        let count2 = list_tables(&db).unwrap().len();
        assert_eq!(count1, count2, "bootstrap should be idempotent");
    }

    #[test]
    fn bootstrap_consistency_removes_stale_entries() {
        let db = test_db();
        // Register a fake table that doesn't actually exist
        register_table(
            &db,
            "phantom_table",
            "Does not exist",
            &[],
            "system",
            false,
            "internal",
            0,
        )
        .unwrap();
        assert!(get_table(&db, "phantom_table").unwrap().is_some());

        // Bootstrap should remove it
        bootstrap_hippocampus(&db).unwrap();
        assert!(
            get_table(&db, "phantom_table").unwrap().is_none(),
            "stale entry should be removed by consistency check"
        );
    }

    #[test]
    fn bootstrap_counts_rows() {
        let db = test_db();

        // Insert some data into sessions (unique on agent_id + scope_key)
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s1', 'test', 'scope_a', 'active')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s2', 'test', 'scope_b', 'active')",
                [],
            )
            .unwrap();
        }

        bootstrap_hippocampus(&db).unwrap();

        let entry = get_table(&db, "sessions").unwrap().unwrap();
        assert_eq!(entry.row_count, 2, "should count existing rows");
    }

    #[test]
    fn system_table_metadata_known_tables() {
        let (desc, level) = system_table_metadata("sessions");
        assert_eq!(desc, "User conversation sessions");
        assert_eq!(level, "read");

        let (desc, level) = system_table_metadata("inference_costs");
        assert_eq!(desc, "LLM inference cost tracking");
        assert_eq!(level, "internal");
    }

    #[test]
    fn system_table_metadata_unknown_table() {
        let (desc, level) = system_table_metadata("unknown_custom_table");
        assert_eq!(desc, "Agent-managed table");
        assert_eq!(level, "readwrite");
    }

    #[test]
    fn access_level_and_row_count_round_trip() {
        let db = test_db();
        register_table(&db, "test", "Test", &[], "system", false, "read", 99).unwrap();

        let entry = get_table(&db, "test").unwrap().unwrap();
        assert_eq!(entry.access_level, "read");
        assert_eq!(entry.row_count, 99);
    }
}
