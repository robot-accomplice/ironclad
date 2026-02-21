use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub model: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub metadata: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub parent_id: Option<String>,
    pub role: String,
    pub content: String,
    pub usage_json: Option<String>,
    pub created_at: String,
}

/// Returns the existing session for `agent_id`, or creates one if none exists.
pub fn find_or_create(db: &Database, agent_id: &str) -> Result<String> {
    let conn = db.conn();

    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM sessions WHERE agent_id = ?1 ORDER BY created_at DESC LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .ok();

    if let Some(id) = existing {
        return Ok(id);
    }

    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO sessions (id, agent_id) VALUES (?1, ?2)",
        rusqlite::params![id, agent_id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(id)
}

pub fn get_session(db: &Database, id: &str) -> Result<Option<Session>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, agent_id, model, created_at, updated_at, metadata FROM sessions WHERE id = ?1",
        [id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                model: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                metadata: row.get(5)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn append_message(
    db: &Database,
    session_id: &str,
    role: &str,
    content: &str,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO session_messages (id, session_id, role, content) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, session_id, role, content],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    conn.execute(
        "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
        [session_id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(id)
}

pub fn list_messages(db: &Database, session_id: &str) -> Result<Vec<Message>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, parent_id, role, content, usage_json, created_at \
             FROM session_messages WHERE session_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([session_id], |row| {
            Ok(Message {
                id: row.get(0)?,
                session_id: row.get(1)?,
                parent_id: row.get(2)?,
                role: row.get(3)?,
                content: row.get(4)?,
                usage_json: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

trait Optional<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> Optional<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn find_or_create_returns_same_id() {
        let db = test_db();
        let id1 = find_or_create(&db, "agent-1").unwrap();
        let id2 = find_or_create(&db, "agent-1").unwrap();
        assert_eq!(id1, id2);
    }

    #[test]
    fn get_session_returns_none_for_missing() {
        let db = test_db();
        let session = get_session(&db, "nonexistent").unwrap();
        assert!(session.is_none());
    }

    #[test]
    fn append_and_list_messages() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-1").unwrap();
        append_message(&db, &sid, "user", "hello").unwrap();
        append_message(&db, &sid, "assistant", "hi there").unwrap();

        let msgs = list_messages(&db, &sid).unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[1].role, "assistant");
    }

    #[test]
    fn get_session_after_create() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-x").unwrap();
        let session = get_session(&db, &sid)
            .unwrap()
            .expect("session should exist");
        assert_eq!(session.agent_id, "agent-x");
        assert!(session.model.is_none());
    }
}
