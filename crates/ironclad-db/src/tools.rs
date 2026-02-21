use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct ToolCallRecord {
    pub id: String,
    pub turn_id: String,
    pub tool_name: String,
    pub input: String,
    pub output: Option<String>,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub created_at: String,
}

pub fn record_tool_call(
    db: &Database,
    turn_id: &str,
    tool_name: &str,
    input: &str,
    output: Option<&str>,
    status: &str,
    duration_ms: Option<i64>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO tool_calls (id, turn_id, tool_name, input, output, status, duration_ms) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, turn_id, tool_name, input, output, status, duration_ms],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_tool_calls_for_turn(db: &Database, turn_id: &str) -> Result<Vec<ToolCallRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, tool_name, input, output, status, duration_ms, created_at \
             FROM tool_calls WHERE turn_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([turn_id], |row| {
            Ok(ToolCallRecord {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                tool_name: row.get(2)?,
                input: row.get(3)?,
                output: row.get(4)?,
                status: row.get(5)?,
                duration_ms: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        // tool_calls has FK to turns, which has FK to sessions — seed parent rows
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id) VALUES ('s1', 'agent-1')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO turns (id, session_id) VALUES ('t1', 's1')", [])
            .unwrap();
        drop(conn);
        db
    }

    #[test]
    fn record_and_retrieve_tool_call() {
        let db = test_db();
        let id = record_tool_call(
            &db,
            "t1",
            "bash",
            r#"{"cmd":"ls"}"#,
            Some("file1\nfile2"),
            "success",
            Some(42),
        )
        .unwrap();
        assert!(!id.is_empty());

        let calls = get_tool_calls_for_turn(&db, "t1").unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].tool_name, "bash");
        assert_eq!(calls[0].duration_ms, Some(42));
    }

    #[test]
    fn empty_turn_returns_empty_vec() {
        let db = test_db();
        let calls = get_tool_calls_for_turn(&db, "t1").unwrap();
        assert!(calls.is_empty());
    }

    #[test]
    fn multiple_calls_ordered_by_time() {
        let db = test_db();
        record_tool_call(&db, "t1", "read", "{}", None, "success", Some(10)).unwrap();
        record_tool_call(&db, "t1", "write", "{}", None, "success", Some(20)).unwrap();

        let calls = get_tool_calls_for_turn(&db, "t1").unwrap();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].tool_name, "read");
        assert_eq!(calls[1].tool_name, "write");
    }
}
