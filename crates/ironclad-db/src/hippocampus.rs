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
) -> Result<()> {
    let conn = db.conn();
    let columns_json =
        serde_json::to_string(columns).map_err(|e| IroncladError::Database(e.to_string()))?;

    conn.execute(
        "INSERT OR REPLACE INTO hippocampus (table_name, description, columns_json, created_by, agent_owned, updated_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
        rusqlite::params![table_name, description, columns_json, created_by, agent_owned as i32],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(())
}

/// Look up a table's schema entry.
pub fn get_table(db: &Database, table_name: &str) -> Result<Option<SchemaEntry>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT table_name, description, columns_json, created_by, agent_owned, created_at, updated_at \
         FROM hippocampus WHERE table_name = ?1",
        [table_name],
        |row| {
            let columns_json: String = row.get(2)?;
            let columns: Vec<ColumnDef> =
                serde_json::from_str(&columns_json).unwrap_or_else(|e| {
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
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

/// List all tables in the hippocampus.
pub fn list_tables(db: &Database) -> Result<Vec<SchemaEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT table_name, description, columns_json, created_by, agent_owned, created_at, updated_at \
             FROM hippocampus ORDER BY table_name",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([], |row| {
            let columns_json: String = row.get(2)?;
            let columns: Vec<ColumnDef> =
                serde_json::from_str(&columns_json).unwrap_or_else(|e| {
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
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// List only agent-created tables.
pub fn list_agent_tables(db: &Database, agent_id: &str) -> Result<Vec<SchemaEntry>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT table_name, description, columns_json, created_by, agent_owned, created_at, updated_at \
             FROM hippocampus WHERE agent_owned = 1 AND created_by = ?1 ORDER BY table_name",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([agent_id], |row| {
            let columns_json: String = row.get(2)?;
            let columns: Vec<ColumnDef> =
                serde_json::from_str(&columns_json).unwrap_or_else(|e| {
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
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

fn validate_identifier(s: &str) -> Result<()> {
    if s.is_empty() || !s.chars().all(|c| c.is_alphanumeric() || c == '_') {
        return Err(IroncladError::Database(format!(
            "identifier contains invalid characters: {s}"
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

    if !table_name.chars().all(|c| c.is_alphanumeric() || c == '_') {
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

    register_table(db, &table_name, description, columns, agent_id, true)?;

    Ok(table_name)
}

/// Drop an agent-owned table. Only tables created by the specified agent can be dropped.
pub fn drop_agent_table(db: &Database, agent_id: &str, table_name: &str) -> Result<()> {
    let entry = get_table(db, table_name)?.ok_or_else(|| {
        IroncladError::Database(format!("table {table_name} not found in hippocampus"))
    })?;

    if !entry.agent_owned || entry.created_by != agent_id {
        return Err(IroncladError::Database(
            "cannot drop: table not owned by this agent".into(),
        ));
    }

    let conn = db.conn();
    conn.execute(&format!("DROP TABLE IF EXISTS \"{}\"", table_name), [])
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    conn.execute(
        "DELETE FROM hippocampus WHERE table_name = ?1",
        [table_name],
    )
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
        summary.push_str(&format!("### {}{}\n", entry.table_name, owner));
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

/// Seed the hippocampus with entries for all built-in system tables.
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
        register_table(db, name, desc, &cols, "system", false)?;
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
        register_table(&db, "users", "User records", &cols, "system", false).unwrap();

        let entry = get_table(&db, "users").unwrap().unwrap();
        assert_eq!(entry.table_name, "users");
        assert_eq!(entry.description, "User records");
        assert_eq!(entry.columns.len(), 2);
        assert!(!entry.agent_owned);
    }

    #[test]
    fn get_table_not_found() {
        let db = test_db();
        assert!(get_table(&db, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn list_tables_empty() {
        let db = test_db();
        assert!(list_tables(&db).unwrap().is_empty());
    }

    #[test]
    fn list_tables_multiple() {
        let db = test_db();
        register_table(&db, "a", "Table A", &[], "system", false).unwrap();
        register_table(&db, "b", "Table B", &[], "system", false).unwrap();
        let tables = list_tables(&db).unwrap();
        assert_eq!(tables.len(), 2);
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
        register_table(&db, "sessions", "Sessions", &[], "system", false).unwrap();
        let result = drop_agent_table(&db, "agent1", "sessions");
        assert!(result.is_err());
    }

    #[test]
    fn list_agent_tables_filters() {
        let db = test_db();
        register_table(&db, "sessions", "System", &[], "system", false).unwrap();
        create_agent_table(&db, "agent1", "notes", "Notes", &[]).unwrap();
        create_agent_table(&db, "agent2", "data", "Data", &[]).unwrap();

        let agent1_tables = list_agent_tables(&db, "agent1").unwrap();
        assert_eq!(agent1_tables.len(), 1);
        assert_eq!(agent1_tables[0].table_name, "agent1_notes");
    }

    #[test]
    fn schema_summary_empty() {
        let db = test_db();
        let summary = schema_summary(&db).unwrap();
        assert!(summary.contains("No tables"));
    }

    #[test]
    fn schema_summary_with_tables() {
        let db = test_db();
        seed_system_tables(&db).unwrap();
        let summary = schema_summary(&db).unwrap();
        assert!(summary.contains("sessions"));
        assert!(summary.contains("episodic_memory"));
        assert!(summary.contains("(system)"));
    }

    #[test]
    fn seed_system_tables_creates_entries() {
        let db = test_db();
        seed_system_tables(&db).unwrap();
        let tables = list_tables(&db).unwrap();
        assert_eq!(tables.len(), 4);
    }

    #[test]
    fn register_table_upsert() {
        let db = test_db();
        register_table(&db, "test", "Version 1", &[], "system", false).unwrap();
        register_table(&db, "test", "Version 2", &[], "system", false).unwrap();

        let entry = get_table(&db, "test").unwrap().unwrap();
        assert_eq!(entry.description, "Version 2");
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
        register_table(&db, "metrics", "Metric values", &cols, "system", false).unwrap();

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
        seed_system_tables(&db).unwrap();
        seed_system_tables(&db).unwrap();
        let tables = list_tables(&db).unwrap();
        assert_eq!(
            tables.len(),
            4,
            "seeding twice should not create duplicates"
        );
    }
}
