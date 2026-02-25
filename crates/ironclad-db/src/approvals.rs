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
