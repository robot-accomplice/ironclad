use serde::{Deserialize, Serialize};

use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SessionScope {
    Agent,
    Peer { peer_id: String, channel: String },
    Group { group_id: String, channel: String },
}

impl SessionScope {
    pub fn scope_key(&self) -> String {
        match self {
            Self::Agent => "agent".to_string(),
            Self::Peer { peer_id, channel } => format!("peer:{channel}:{peer_id}"),
            Self::Group { group_id, channel } => format!("group:{channel}:{group_id}"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
    pub scope_key: Option<String>,
    pub status: String,
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

/// Returns the existing active session for `agent_id` (optionally scoped), or creates one.
pub fn find_or_create(
    db: &Database,
    agent_id: &str,
    scope: Option<&SessionScope>,
) -> Result<String> {
    let conn = db.conn();
    let scope_key = scope.map(|s| s.scope_key());

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let existing: Option<String> = if let Some(ref key) = scope_key {
        tx.query_row(
            "SELECT id FROM sessions WHERE agent_id = ?1 AND scope_key = ?2 AND status = 'active' ORDER BY created_at DESC LIMIT 1",
            rusqlite::params![agent_id, key],
            |row| row.get(0),
        )
        .ok()
    } else {
        tx.query_row(
            "SELECT id FROM sessions WHERE agent_id = ?1 AND scope_key IS NULL AND status = 'active' ORDER BY created_at DESC LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .ok()
    };

    if let Some(id) = existing {
        tx.commit()
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        return Ok(id);
    }

    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO sessions (id, agent_id, scope_key) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, agent_id, scope_key],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    tx.commit()
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    Ok(id)
}

pub fn get_session(db: &Database, id: &str) -> Result<Option<Session>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, agent_id, scope_key, status, model, created_at, updated_at, metadata FROM sessions WHERE id = ?1",
        [id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                scope_key: row.get(2)?,
                status: row.get(3)?,
                model: row.get(4)?,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
                metadata: row.get(7)?,
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

/// Mark a session as archived (inactive).
pub fn archive_session(db: &Database, session_id: &str) -> Result<()> {
    let conn = db.conn();
    let changed = conn
        .execute(
            "UPDATE sessions SET status = 'archived', updated_at = datetime('now') WHERE id = ?1",
            [session_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    if changed == 0 {
        return Err(IroncladError::Database(format!(
            "session not found: {session_id}"
        )));
    }
    Ok(())
}

/// Expire active sessions older than `max_age_seconds`.
pub fn expire_stale_sessions(db: &Database, max_age_seconds: u64) -> Result<usize> {
    let conn = db.conn();
    let expired = conn
        .execute(
            "UPDATE sessions SET status = 'expired', updated_at = datetime('now') \
             WHERE status = 'active' \
             AND (julianday('now') - julianday(updated_at)) * 86400 > ?1",
            rusqlite::params![max_age_seconds as f64],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(expired)
}

/// List all active sessions, optionally filtered by agent_id.
pub fn list_active_sessions(db: &Database, agent_id: Option<&str>) -> Result<Vec<Session>> {
    let conn = db.conn();
    let mut sessions = Vec::new();

    if let Some(aid) = agent_id {
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, scope_key, status, model, created_at, updated_at, metadata \
                 FROM sessions WHERE agent_id = ?1 AND status = 'active' ORDER BY created_at DESC",
            )
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([aid], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    scope_key: row.get(2)?,
                    status: row.get(3)?,
                    model: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        for row in rows {
            sessions.push(row.map_err(|e| IroncladError::Database(e.to_string()))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, scope_key, status, model, created_at, updated_at, metadata \
                 FROM sessions WHERE status = 'active' ORDER BY created_at DESC",
            )
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(Session {
                    id: row.get(0)?,
                    agent_id: row.get(1)?,
                    scope_key: row.get(2)?,
                    status: row.get(3)?,
                    model: row.get(4)?,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    metadata: row.get(7)?,
                })
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        for row in rows {
            sessions.push(row.map_err(|e| IroncladError::Database(e.to_string()))?);
        }
    }

    Ok(sessions)
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
        let id1 = find_or_create(&db, "agent-1", None).unwrap();
        let id2 = find_or_create(&db, "agent-1", None).unwrap();
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
                let id = find_or_create(db.as_ref(), key, None).unwrap();
                tx.send(id).unwrap();
            })
        };
        let th2 = {
            let db = std::sync::Arc::clone(&db);
            std::thread::spawn(move || {
                let id = find_or_create(db.as_ref(), key, None).unwrap();
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
        let sid = find_or_create(&db, "agent-1", None).unwrap();
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
        let sid = find_or_create(&db, "agent-x", None).unwrap();
        let session = get_session(&db, &sid)
            .unwrap()
            .expect("session should exist");
        assert_eq!(session.agent_id, "agent-x");
        assert_eq!(session.status, "active");
        assert!(session.model.is_none());
    }

    #[test]
    fn list_messages_with_limit() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-lim", None).unwrap();
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
        let sid = find_or_create(&db, "agent-meta", None).unwrap();
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
        let sid = find_or_create(&db, "agent-meta", None).unwrap();
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
        let sid = find_or_create(&db, "agent-long", None).unwrap();
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
        let sid = find_or_create(&db, "agent-turn", None).unwrap();
        let turn_id =
            create_turn(&db, &sid, Some("gpt-4"), Some(100), Some(200), Some(0.03)).unwrap();
        assert!(!turn_id.is_empty());
    }

    #[test]
    fn create_turn_all_none() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-turn-none", None).unwrap();
        let turn_id = create_turn(&db, &sid, None, None, None, None).unwrap();
        assert!(!turn_id.is_empty());
    }

    #[test]
    fn create_turn_multiple_per_session() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-multi-turn", None).unwrap();
        let t1 = create_turn(&db, &sid, Some("gpt-4"), Some(10), Some(20), None).unwrap();
        let t2 = create_turn(&db, &sid, Some("gpt-4"), Some(30), Some(40), None).unwrap();
        assert_ne!(t1, t2);
    }

    #[test]
    fn find_or_create_different_agents_different_sessions() {
        let db = test_db();
        let id1 = find_or_create(&db, "agent-a", None).unwrap();
        let id2 = find_or_create(&db, "agent-b", None).unwrap();
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
        let sid = find_or_create(&db, "agent-order", None).unwrap();
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
        let sid = find_or_create(&db, "agent-fields", None).unwrap();
        let msg_id = append_message(&db, &sid, "user", "hello").unwrap();
        let msgs = list_messages(&db, &sid, Some(1)).unwrap();
        assert_eq!(msgs[0].id, msg_id);
        assert_eq!(msgs[0].session_id, sid);
        assert_eq!(msgs[0].role, "user");
        assert_eq!(msgs[0].content, "hello");
    }

    // ── Session scoping tests ──

    #[test]
    fn scope_key_agent() {
        let scope = SessionScope::Agent;
        assert_eq!(scope.scope_key(), "agent");
    }

    #[test]
    fn scope_key_peer() {
        let scope = SessionScope::Peer {
            peer_id: "user123".into(),
            channel: "telegram".into(),
        };
        assert_eq!(scope.scope_key(), "peer:telegram:user123");
    }

    #[test]
    fn scope_key_group() {
        let scope = SessionScope::Group {
            group_id: "grp-42".into(),
            channel: "discord".into(),
        };
        assert_eq!(scope.scope_key(), "group:discord:grp-42");
    }

    #[test]
    fn find_or_create_with_peer_scope() {
        let db = test_db();
        let scope = SessionScope::Peer {
            peer_id: "alice".into(),
            channel: "telegram".into(),
        };
        let id1 = find_or_create(&db, "agent-1", Some(&scope)).unwrap();
        let id2 = find_or_create(&db, "agent-1", Some(&scope)).unwrap();
        assert_eq!(id1, id2);

        let id_no_scope = find_or_create(&db, "agent-1", None).unwrap();
        assert_ne!(
            id1, id_no_scope,
            "scoped and unscoped sessions should differ"
        );
    }

    #[test]
    fn find_or_create_different_scopes_different_sessions() {
        let db = test_db();
        let scope_a = SessionScope::Peer {
            peer_id: "alice".into(),
            channel: "telegram".into(),
        };
        let scope_b = SessionScope::Peer {
            peer_id: "bob".into(),
            channel: "telegram".into(),
        };
        let id_a = find_or_create(&db, "agent-1", Some(&scope_a)).unwrap();
        let id_b = find_or_create(&db, "agent-1", Some(&scope_b)).unwrap();
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn archive_session_sets_status() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-archive", None).unwrap();
        archive_session(&db, &sid).unwrap();
        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.status, "archived");
    }

    #[test]
    fn archive_session_not_found() {
        let db = test_db();
        let result = archive_session(&db, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn find_or_create_skips_archived_sessions() {
        let db = test_db();
        let sid1 = find_or_create(&db, "agent-skip", None).unwrap();
        archive_session(&db, &sid1).unwrap();
        let sid2 = find_or_create(&db, "agent-skip", None).unwrap();
        assert_ne!(sid1, sid2, "should create new session after archiving");
    }

    #[test]
    fn list_active_sessions_filters_correctly() {
        let db = test_db();
        let sid1 = find_or_create(&db, "agent-list", None).unwrap();
        let _sid2 = find_or_create(&db, "agent-list-2", None).unwrap();
        archive_session(&db, &sid1).unwrap();

        let active = list_active_sessions(&db, Some("agent-list")).unwrap();
        assert!(active.is_empty());

        let all_active = list_active_sessions(&db, None).unwrap();
        assert_eq!(all_active.len(), 1);
        assert_eq!(all_active[0].agent_id, "agent-list-2");
    }

    #[test]
    fn session_scope_serde_roundtrip() {
        let scope = SessionScope::Peer {
            peer_id: "u1".into(),
            channel: "tg".into(),
        };
        let json = serde_json::to_string(&scope).unwrap();
        let back: SessionScope = serde_json::from_str(&json).unwrap();
        assert_eq!(scope, back);
    }
}
