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
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    tx.execute(
        "INSERT INTO session_messages (id, session_id, role, content) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![id, session_id, role, content],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    tx.execute(
        "UPDATE sessions SET updated_at = datetime('now') WHERE id = ?1",
        [session_id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn list_messages(db: &Database, session_id: &str, limit: Option<i64>) -> Result<Vec<Message>> {
    let conn = db.conn();
    let effective_limit = limit.unwrap_or(i64::MAX);
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, parent_id, role, content, usage_json, created_at \
             FROM session_messages WHERE session_id = ?1 ORDER BY created_at ASC LIMIT ?2",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(rusqlite::params![session_id, effective_limit], |row| {
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

/// Create a turn record for tool-use tracking within a session.
pub fn create_turn(
    db: &Database,
    session_id: &str,
    model: Option<&str>,
    tokens_in: Option<i64>,
    tokens_out: Option<i64>,
    cost: Option<f64>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO turns (id, session_id, model, tokens_in, tokens_out, cost) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, session_id, model, tokens_in, tokens_out, cost],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

/// Update the JSON metadata blob for a session.
pub fn update_metadata(db: &Database, session_id: &str, metadata_json: &str) -> Result<()> {
    let conn = db.conn();
    let changed = conn
        .execute(
            "UPDATE sessions SET metadata = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![metadata_json, session_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    if changed == 0 {
        return Err(IroncladError::Database(format!(
            "session not found: {session_id}"
        )));
    }
    Ok(())
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
    fn concurrent_find_or_create_same_key_both_succeed() {
        let db = std::sync::Arc::new(test_db());
        let key = "concurrent-agent";
        let (tx, rx) = std::sync::mpsc::channel();
        let tx2 = tx.clone();
        let th1 = {
            let db = std::sync::Arc::clone(&db);
            std::thread::spawn(move || {
                let id = find_or_create(db.as_ref(), key).unwrap();
                tx.send(id).unwrap();
            })
        };
        let th2 = {
            let db = std::sync::Arc::clone(&db);
            std::thread::spawn(move || {
                let id = find_or_create(db.as_ref(), key).unwrap();
                tx2.send(id).unwrap();
            })
        };
        let id1 = rx.recv().unwrap();
        let id2 = rx.recv().unwrap();
        th1.join().unwrap();
        th2.join().unwrap();
        assert_eq!(
            id1, id2,
            "concurrent find_or_create with same key should return same session id"
        );
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

        let msgs = list_messages(&db, &sid, None).unwrap();
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

    #[test]
    fn list_messages_with_limit() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-lim").unwrap();
        for i in 0..10 {
            append_message(&db, &sid, "user", &format!("msg-{i}")).unwrap();
        }

        let all = list_messages(&db, &sid, None).unwrap();
        assert_eq!(all.len(), 10);

        let limited = list_messages(&db, &sid, Some(3)).unwrap();
        assert_eq!(limited.len(), 3);
        assert_eq!(limited[0].content, "msg-0");
    }

    #[test]
    fn update_metadata_roundtrip() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-meta").unwrap();
        update_metadata(&db, &sid, r#"{"topic":"testing"}"#).unwrap();

        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.metadata.as_deref(), Some(r#"{"topic":"testing"}"#));
    }

    #[test]
    fn update_metadata_missing_session() {
        let db = test_db();
        let result = update_metadata(&db, "nonexistent", "{}");
        assert!(result.is_err());
    }

    // 9C: Edge case tests

    #[test]
    fn update_metadata_accepts_malformed_json() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-meta").unwrap();
        update_metadata(&db, &sid, "{invalid json blob").unwrap();
        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.metadata.as_deref(), Some("{invalid json blob"));
    }

    #[test]
    fn get_session_empty_id_returns_none() {
        let db = test_db();
        let session = get_session(&db, "").unwrap();
        assert!(session.is_none());
    }

    #[test]
    fn append_message_very_long_content() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-long").unwrap();
        let long = "x".repeat(100_000);
        let id = append_message(&db, &sid, "user", &long).unwrap();
        assert!(!id.is_empty());
        let msgs = list_messages(&db, &sid, Some(1)).unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].content.len(), 100_000);
    }

    #[test]
    fn create_turn_all_fields() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-turn").unwrap();
        let turn_id =
            create_turn(&db, &sid, Some("gpt-4"), Some(100), Some(200), Some(0.03)).unwrap();
        assert!(!turn_id.is_empty());
    }

    #[test]
    fn create_turn_all_none() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-turn-none").unwrap();
        let turn_id = create_turn(&db, &sid, None, None, None, None).unwrap();
        assert!(!turn_id.is_empty());
    }

    #[test]
    fn create_turn_multiple_per_session() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-multi-turn").unwrap();
        let t1 = create_turn(&db, &sid, Some("gpt-4"), Some(10), Some(20), None).unwrap();
        let t2 = create_turn(&db, &sid, Some("gpt-4"), Some(30), Some(40), None).unwrap();
        assert_ne!(t1, t2);
    }

    #[test]
    fn find_or_create_different_agents_different_sessions() {
        let db = test_db();
        let id1 = find_or_create(&db, "agent-a").unwrap();
        let id2 = find_or_create(&db, "agent-b").unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn list_messages_nonexistent_session() {
        let db = test_db();
        let msgs = list_messages(&db, "no-such-session", None).unwrap();
        assert!(msgs.is_empty());
    }

    #[test]
    fn list_messages_ordering_is_chronological() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-order").unwrap();
        append_message(&db, &sid, "user", "first").unwrap();
        append_message(&db, &sid, "assistant", "second").unwrap();
        append_message(&db, &sid, "user", "third").unwrap();

        let msgs = list_messages(&db, &sid, None).unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].content, "first");
        assert_eq!(msgs[1].content, "second");
        assert_eq!(msgs[2].content, "third");
    }

    #[test]
    fn message_fields_populated() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-fields").unwrap();
        let msg_id = append_message(&db, &sid, "user", "hello").unwrap();
        let msgs = list_messages(&db, &sid, Some(1)).unwrap();
        assert_eq!(msgs[0].id, msg_id);
        assert_eq!(msgs[0].session_id, sid);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
    }
}
