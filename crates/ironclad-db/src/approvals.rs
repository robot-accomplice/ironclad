use crate::Database;
use ironclad_core::{IroncladError, Result};

pub fn record_approval_request(
    db: &Database,
    id: &str,
    tool_name: &str,
    tool_input: &str,
    session_id: Option<&str>,
    status: &str,
    timeout_at: &str,
) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO approval_requests (id, tool_name, tool_input, session_id, status, timeout_at) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           tool_name = excluded.tool_name,
           tool_input = excluded.tool_input,
           session_id = excluded.session_id,
           status = excluded.status,
           timeout_at = excluded.timeout_at",
        rusqlite::params![id, tool_name, tool_input, session_id, status, timeout_at],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn record_approval_decision(
    db: &Database,
    id: &str,
    status: &str,
    decided_by: &str,
    decided_at: &str,
) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE approval_requests
         SET status = ?2, decided_by = ?3, decided_at = ?4
         WHERE id = ?1",
        rusqlite::params![id, status, decided_by, decided_at],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn record_approval_request_inserts() {
        let db = test_db();
        record_approval_request(
            &db,
            "req-1",
            "bash",
            r#"{"cmd":"rm -rf /"}"#,
            Some("session-1"),
            "pending",
            "2025-01-01T01:00:00",
        )
        .unwrap();

        let conn = db.conn();
        let (tool_name, status): (String, String) = conn
            .query_row(
                "SELECT tool_name, status FROM approval_requests WHERE id = ?1",
                ["req-1"],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(tool_name, "bash");
        assert_eq!(status, "pending");
    }

    #[test]
    fn record_approval_request_upserts_on_conflict() {
        let db = test_db();
        record_approval_request(
            &db,
            "req-2",
            "bash",
            r#"{"cmd":"ls"}"#,
            Some("s1"),
            "pending",
            "2025-01-01T01:00:00",
        )
        .unwrap();

        // Upsert with same id but different status
        record_approval_request(
            &db,
            "req-2",
            "bash",
            r#"{"cmd":"ls -la"}"#,
            Some("s1"),
            "approved",
            "2025-01-01T02:00:00",
        )
        .unwrap();

        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM approval_requests WHERE id = 'req-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1, "upsert should not create duplicate rows");

        let status: String = conn
            .query_row(
                "SELECT status FROM approval_requests WHERE id = 'req-2'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "approved");
    }

    #[test]
    fn record_approval_request_with_no_session() {
        let db = test_db();
        record_approval_request(
            &db,
            "req-3",
            "write_file",
            r#"{"path":"/etc/passwd"}"#,
            None,
            "pending",
            "2025-06-01T00:00:00",
        )
        .unwrap();

        let conn = db.conn();
        let session_id: Option<String> = conn
            .query_row(
                "SELECT session_id FROM approval_requests WHERE id = 'req-3'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!(session_id.is_none());
    }

    #[test]
    fn record_approval_decision_updates_existing() {
        let db = test_db();
        record_approval_request(
            &db,
            "req-4",
            "exec",
            r#"{"binary":"deploy.sh"}"#,
            Some("s1"),
            "pending",
            "2025-01-01T01:00:00",
        )
        .unwrap();

        record_approval_decision(
            &db,
            "req-4",
            "approved",
            "admin@example.com",
            "2025-01-01T01:05:00",
        )
        .unwrap();

        let conn = db.conn();
        let (status, decided_by, decided_at): (String, Option<String>, Option<String>) = conn
            .query_row(
                "SELECT status, decided_by, decided_at FROM approval_requests WHERE id = 'req-4'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(status, "approved");
        assert_eq!(decided_by.as_deref(), Some("admin@example.com"));
        assert_eq!(decided_at.as_deref(), Some("2025-01-01T01:05:00"));
    }

    #[test]
    fn record_approval_decision_on_nonexistent_is_noop() {
        let db = test_db();
        // Should not error even if no row with that id exists
        record_approval_decision(&db, "nonexistent", "denied", "admin", "2025-01-01T00:00:00")
            .unwrap();
    }

    #[test]
    fn multiple_approval_requests() {
        let db = test_db();
        for i in 0..5 {
            record_approval_request(
                &db,
                &format!("req-multi-{i}"),
                "tool_x",
                "{}",
                Some("s1"),
                "pending",
                &format!("2025-01-01T0{i}:00:00"),
            )
            .unwrap();
        }

        let conn = db.conn();
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM approval_requests WHERE status = 'pending'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 5);
    }

    #[test]
    fn approval_decision_denied() {
        let db = test_db();
        record_approval_request(
            &db,
            "req-deny",
            "dangerous_tool",
            r#"{"action":"delete_all"}"#,
            Some("s1"),
            "pending",
            "2025-06-01T00:00:00",
        )
        .unwrap();

        record_approval_decision(
            &db,
            "req-deny",
            "denied",
            "security-bot",
            "2025-06-01T00:00:05",
        )
        .unwrap();

        let conn = db.conn();
        let status: String = conn
            .query_row(
                "SELECT status FROM approval_requests WHERE id = 'req-deny'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "denied");
    }
}
