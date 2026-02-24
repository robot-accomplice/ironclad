use rusqlite::OptionalExtension;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionStatus {
    Active,
    Archived,
    Expired,
}

impl std::fmt::Display for SessionStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Active => write!(f, "active"),
            Self::Archived => write!(f, "archived"),
            Self::Expired => write!(f, "expired"),
        }
    }
}

impl SessionStatus {
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "archived" => Self::Archived,
            "expired" => Self::Expired,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

impl std::fmt::Display for MessageRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::User => write!(f, "user"),
            Self::Assistant => write!(f, "assistant"),
            Self::System => write!(f, "system"),
            Self::Tool => write!(f, "tool"),
        }
    }
}

impl MessageRole {
    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "assistant" => Self::Assistant,
            "system" => Self::System,
            "tool" => Self::Tool,
            _ => Self::User,
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
    pub nickname: Option<String>,
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

/// Always creates a new active session for `agent_id` (optionally scoped).
pub fn create_new(db: &Database, agent_id: &str, scope: Option<&SessionScope>) -> Result<String> {
    let conn = db.conn();
    let scope_key = scope.map(|s| s.scope_key());
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO sessions (id, agent_id, scope_key) VALUES (?1, ?2, ?3)",
        rusqlite::params![id, agent_id, scope_key],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_session(db: &Database, id: &str) -> Result<Option<Session>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, agent_id, scope_key, status, model, nickname, created_at, updated_at, metadata \
         FROM sessions WHERE id = ?1",
        [id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                scope_key: row.get(2)?,
                status: row.get(3)?,
                model: row.get(4)?,
                nickname: row.get(5)?,
                created_at: row.get(6)?,
                updated_at: row.get(7)?,
                metadata: row.get(8)?,
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

/// Update the nickname for a session.
pub fn update_nickname(db: &Database, session_id: &str, nickname: &str) -> Result<()> {
    let conn = db.conn();
    let changed = conn
        .execute(
            "UPDATE sessions SET nickname = ?1, updated_at = datetime('now') WHERE id = ?2",
            rusqlite::params![nickname, session_id],
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
                "SELECT id, agent_id, scope_key, status, model, nickname, created_at, updated_at, metadata \
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
                    nickname: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    metadata: row.get(8)?,
                })
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        for row in rows {
            sessions.push(row.map_err(|e| IroncladError::Database(e.to_string()))?);
        }
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, agent_id, scope_key, status, model, nickname, created_at, updated_at, metadata \
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
                    nickname: row.get(5)?,
                    created_at: row.get(6)?,
                    updated_at: row.get(7)?,
                    metadata: row.get(8)?,
                })
            })
            .map_err(|e| IroncladError::Database(e.to_string()))?;
        for row in rows {
            sessions.push(row.map_err(|e| IroncladError::Database(e.to_string()))?);
        }
    }

    Ok(sessions)
}

/// Find the largest byte index <= `max_bytes` that is a valid char boundary.
fn char_boundary(s: &str, max_bytes: usize) -> usize {
    if max_bytes >= s.len() {
        return s.len();
    }
    s.char_indices()
        .map(|(i, _)| i)
        .take_while(|&i| i <= max_bytes)
        .last()
        .unwrap_or(0)
}

/// Derive a short nickname from the first user message using heuristics.
pub fn derive_nickname(first_message: &str) -> String {
    let trimmed = first_message.trim();
    if trimmed.is_empty() {
        return "Untitled".into();
    }

    let greeting_prefixes: &[&str] = &[
        "hey ",
        "hi ",
        "hello ",
        "yo ",
        "can you ",
        "could you ",
        "please ",
        "i need ",
        "i want ",
        "help me ",
        "hey, ",
        "hi, ",
        "hello, ",
        "yo, ",
    ];

    let mut text = trimmed;
    let lower = text.to_lowercase();
    for prefix in greeting_prefixes {
        if lower.starts_with(prefix) {
            text = &text[prefix.len()..];
            break;
        }
    }

    let text = text.trim_start();
    if text.is_empty() {
        return "Untitled".into();
    }

    let sentence_end = text.find(['.', '?', '!', '\n']).unwrap_or(text.len());
    let end = char_boundary(text, sentence_end.min(60));

    let mut nickname: String = text[..end].trim().to_string();

    if nickname.len() > 50 {
        let boundary = char_boundary(&nickname, 50);
        if let Some(break_pos) = nickname[..boundary].rfind(' ') {
            nickname.truncate(break_pos);
            nickname.push_str("...");
        } else {
            nickname.truncate(boundary);
            nickname.push_str("...");
        }
    }

    if nickname.is_empty() {
        return "Untitled".into();
    }

    let mut chars = nickname.chars();
    let first = chars.next().unwrap().to_uppercase().to_string();
    format!("{first}{}", chars.as_str())
}

/// Backfill nicknames for all sessions that don't have one yet.
/// Returns the number of sessions updated.
pub fn backfill_nicknames(db: &Database) -> Result<usize> {
    let conn = db.conn();

    let mut stmt = conn
        .prepare(
            "SELECT s.id, \
                (SELECT content FROM session_messages \
                 WHERE session_id = s.id AND role = 'user' \
                 ORDER BY created_at ASC LIMIT 1) \
             FROM sessions s WHERE s.nickname IS NULL",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows: Vec<(String, Option<String>)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();

    let count = rows.len();
    for (session_id, first_msg) in &rows {
        let nick = match first_msg {
            Some(msg) => derive_nickname(msg),
            None => "Untitled".into(),
        };
        conn.execute(
            "UPDATE sessions SET nickname = ?1 WHERE id = ?2",
            rusqlite::params![nick, session_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    }

    Ok(count)
}

// ── Turn query helpers ─────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TurnRecord {
    pub id: String,
    pub session_id: String,
    pub thinking: Option<String>,
    pub tool_calls_json: Option<String>,
    pub tokens_in: Option<i64>,
    pub tokens_out: Option<i64>,
    pub cost: Option<f64>,
    pub model: Option<String>,
    pub created_at: String,
}

pub fn list_turns_for_session(db: &Database, session_id: &str) -> Result<Vec<TurnRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, session_id, thinking, tool_calls_json, tokens_in, tokens_out, cost, model, created_at \
             FROM turns WHERE session_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([session_id], |row| {
            Ok(TurnRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                thinking: row.get(2)?,
                tool_calls_json: row.get(3)?,
                tokens_in: row.get(4)?,
                tokens_out: row.get(5)?,
                cost: row.get(6)?,
                model: row.get(7)?,
                created_at: row.get(8)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn get_turn_by_id(db: &Database, turn_id: &str) -> Result<Option<TurnRecord>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, session_id, thinking, tool_calls_json, tokens_in, tokens_out, cost, model, created_at \
         FROM turns WHERE id = ?1",
        [turn_id],
        |row| {
            Ok(TurnRecord {
                id: row.get(0)?,
                session_id: row.get(1)?,
                thinking: row.get(2)?,
                tool_calls_json: row.get(3)?,
                tokens_in: row.get(4)?,
                tokens_out: row.get(5)?,
                cost: row.get(6)?,
                model: row.get(7)?,
                created_at: row.get(8)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

// ── Turn feedback ──────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnFeedback {
    pub id: String,
    pub turn_id: String,
    pub session_id: String,
    pub grade: i32,
    pub source: String,
    pub comment: Option<String>,
    pub created_at: String,
}

pub fn record_feedback(
    db: &Database,
    turn_id: &str,
    session_id: &str,
    grade: i32,
    source: &str,
    comment: Option<&str>,
) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    db.conn()
        .execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade, source, comment) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(turn_id) DO UPDATE SET grade = excluded.grade, source = excluded.source, comment = excluded.comment",
            rusqlite::params![id, turn_id, session_id, grade, source, comment],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn get_feedback(db: &Database, turn_id: &str) -> Result<Option<TurnFeedback>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, turn_id, session_id, grade, source, comment, created_at \
         FROM turn_feedback WHERE turn_id = ?1 LIMIT 1",
        [turn_id],
        |row| {
            Ok(TurnFeedback {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                session_id: row.get(2)?,
                grade: row.get(3)?,
                source: row.get(4)?,
                comment: row.get(5)?,
                created_at: row.get(6)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn update_feedback(
    db: &Database,
    turn_id: &str,
    grade: i32,
    comment: Option<&str>,
) -> Result<()> {
    let conn = db.conn();
    let changed = conn
        .execute(
            "UPDATE turn_feedback SET grade = ?1, comment = ?2 WHERE turn_id = ?3",
            rusqlite::params![grade, comment, turn_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    if changed == 0 {
        return Err(IroncladError::Database(format!(
            "no feedback found for turn: {turn_id}"
        )));
    }
    Ok(())
}

pub fn list_session_feedback(db: &Database, session_id: &str) -> Result<Vec<TurnFeedback>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, session_id, grade, source, comment, created_at \
             FROM turn_feedback WHERE session_id = ?1 ORDER BY created_at ASC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map([session_id], |row| {
            Ok(TurnFeedback {
                id: row.get(0)?,
                turn_id: row.get(1)?,
                session_id: row.get(2)?,
                grade: row.get(3)?,
                source: row.get(4)?,
                comment: row.get(5)?,
                created_at: row.get(6)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Count messages in a session.
pub fn message_count(db: &Database, session_id: &str) -> Result<i64> {
    let conn = db.conn();
    conn.query_row(
        "SELECT COUNT(*) FROM session_messages WHERE session_id = ?1",
        [session_id],
        |row| row.get(0),
    )
    .map_err(|e| IroncladError::Database(e.to_string()))
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

    // ── derive_nickname tests ────────────────────────────────

    #[test]
    fn derive_nickname_strips_greeting() {
        assert_eq!(
            derive_nickname("Hey can you help me with Rust?"),
            "Can you help me with Rust"
        );
    }

    #[test]
    fn derive_nickname_strips_hello() {
        assert_eq!(
            derive_nickname("Hello, I need a database schema"),
            "I need a database schema"
        );
    }

    #[test]
    fn derive_nickname_takes_first_sentence() {
        assert_eq!(
            derive_nickname("Fix the build. Also update deps."),
            "Fix the build"
        );
    }

    #[test]
    fn derive_nickname_truncates_long() {
        let long = "a ".repeat(50);
        let nick = derive_nickname(&long);
        assert!(nick.len() <= 55, "nickname too long: {} chars", nick.len());
        assert!(nick.ends_with("..."));
    }

    #[test]
    fn derive_nickname_empty_returns_untitled() {
        assert_eq!(derive_nickname(""), "Untitled");
        assert_eq!(derive_nickname("   "), "Untitled");
    }

    #[test]
    fn derive_nickname_greeting_only() {
        assert_eq!(derive_nickname("hey"), "Hey");
    }

    #[test]
    fn derive_nickname_capitalizes() {
        assert_eq!(
            derive_nickname("refactor the auth module"),
            "Refactor the auth module"
        );
    }

    #[test]
    fn derive_nickname_question_mark() {
        assert_eq!(
            derive_nickname("what is the meaning of life?"),
            "What is the meaning of life"
        );
    }

    #[test]
    fn derive_nickname_unicode() {
        let nick = derive_nickname("日本語のテスト");
        assert!(!nick.is_empty());
        assert_ne!(nick, "Untitled");
    }

    #[test]
    fn derive_nickname_multibyte_at_boundary() {
        let msg = "日".repeat(30);
        let nick = derive_nickname(&msg);
        assert!(!nick.is_empty());
        assert!(nick.len() <= 55, "nickname too long: {}", nick.len());
    }

    #[test]
    fn derive_nickname_emoji_boundary() {
        let msg = format!("{}problem", "🔥".repeat(20));
        let nick = derive_nickname(&msg);
        assert!(!nick.is_empty());
    }

    // ── update_nickname tests ────────────────────────────────

    #[test]
    fn update_nickname_roundtrip() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-nick", None).unwrap();
        assert!(get_session(&db, &sid).unwrap().unwrap().nickname.is_none());

        update_nickname(&db, &sid, "My Cool Session").unwrap();

        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("My Cool Session"));
    }

    #[test]
    fn update_nickname_missing_session() {
        let db = test_db();
        let result = update_nickname(&db, "nonexistent", "test");
        assert!(result.is_err());
    }

    #[test]
    fn update_nickname_overwrite() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-overwrite", None).unwrap();
        update_nickname(&db, &sid, "First").unwrap();
        update_nickname(&db, &sid, "Second").unwrap();
        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("Second"));
    }

    // ── backfill_nicknames tests ─────────────────────────────

    #[test]
    fn backfill_nicknames_sets_from_first_message() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-bf", None).unwrap();
        append_message(&db, &sid, "user", "Help me debug this crash").unwrap();

        let count = backfill_nicknames(&db).unwrap();
        assert_eq!(count, 1);

        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("Debug this crash"));
    }

    #[test]
    fn backfill_nicknames_untitled_for_empty_session() {
        let db = test_db();
        find_or_create(&db, "agent-empty-bf", None).unwrap();

        let count = backfill_nicknames(&db).unwrap();
        assert_eq!(count, 1);

        let conn = db.conn();
        let nick: String = conn
            .query_row(
                "SELECT nickname FROM sessions WHERE agent_id = 'agent-empty-bf'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(nick, "Untitled");
    }

    #[test]
    fn backfill_nicknames_skips_already_set() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-skip-bf", None).unwrap();
        update_nickname(&db, &sid, "Already Set").unwrap();
        append_message(&db, &sid, "user", "something else").unwrap();

        let count = backfill_nicknames(&db).unwrap();
        assert_eq!(count, 0);

        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("Already Set"));
    }

    // ── message_count tests ─────────────────────────────────

    #[test]
    fn message_count_empty() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-count", None).unwrap();
        assert_eq!(message_count(&db, &sid).unwrap(), 0);
    }

    #[test]
    fn message_count_after_append() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-count2", None).unwrap();
        append_message(&db, &sid, "user", "a").unwrap();
        append_message(&db, &sid, "assistant", "b").unwrap();
        append_message(&db, &sid, "user", "c").unwrap();
        assert_eq!(message_count(&db, &sid).unwrap(), 3);
    }

    // ── turn feedback tests ──────────────────────────────────

    #[test]
    fn record_and_get_feedback() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-fb", None).unwrap();
        let tid = create_turn(&db, &sid, Some("gpt-4"), Some(100), Some(50), Some(0.01)).unwrap();

        let fb_id = record_feedback(&db, &tid, &sid, 4, "dashboard", Some("good")).unwrap();
        assert!(!fb_id.is_empty());

        let fb = get_feedback(&db, &tid)
            .unwrap()
            .expect("feedback should exist");
        assert_eq!(fb.grade, 4);
        assert_eq!(fb.source, "dashboard");
        assert_eq!(fb.comment.as_deref(), Some("good"));
    }

    #[test]
    fn get_feedback_returns_none_for_missing() {
        let db = test_db();
        assert!(get_feedback(&db, "nonexistent").unwrap().is_none());
    }

    #[test]
    fn update_feedback_changes_grade() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-fb-up", None).unwrap();
        let tid = create_turn(&db, &sid, None, None, None, None).unwrap();
        record_feedback(&db, &tid, &sid, 3, "dashboard", None).unwrap();

        update_feedback(&db, &tid, 5, Some("revised")).unwrap();

        let fb = get_feedback(&db, &tid).unwrap().unwrap();
        assert_eq!(fb.grade, 5);
        assert_eq!(fb.comment.as_deref(), Some("revised"));
    }

    #[test]
    fn update_feedback_missing_returns_error() {
        let db = test_db();
        assert!(update_feedback(&db, "nonexistent", 3, None).is_err());
    }

    #[test]
    fn list_session_feedback_returns_all() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-fb-list", None).unwrap();
        let t1 = create_turn(&db, &sid, None, None, None, None).unwrap();
        let t2 = create_turn(&db, &sid, None, None, None, None).unwrap();
        record_feedback(&db, &t1, &sid, 4, "dashboard", None).unwrap();
        record_feedback(&db, &t2, &sid, 2, "dashboard", Some("bad")).unwrap();

        let fbs = list_session_feedback(&db, &sid).unwrap();
        assert_eq!(fbs.len(), 2);
    }

    #[test]
    fn list_session_feedback_empty() {
        let db = test_db();
        let fbs = list_session_feedback(&db, "nonexistent").unwrap();
        assert!(fbs.is_empty());
    }

    #[test]
    fn record_feedback_upserts_on_duplicate_turn() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-fb-upsert", None).unwrap();
        let tid = create_turn(&db, &sid, Some("gpt-4"), Some(100), Some(50), Some(0.01)).unwrap();

        record_feedback(&db, &tid, &sid, 3, "dashboard", Some("okay")).unwrap();
        let fb1 = get_feedback(&db, &tid).unwrap().unwrap();
        assert_eq!(fb1.grade, 3);

        record_feedback(&db, &tid, &sid, 5, "api", Some("great")).unwrap();
        let fb2 = get_feedback(&db, &tid).unwrap().unwrap();
        assert_eq!(fb2.grade, 5, "grade should be updated by upsert");
        assert_eq!(fb2.source, "api", "source should be updated by upsert");
        assert_eq!(fb2.comment.as_deref(), Some("great"));

        let all = list_session_feedback(&db, &sid).unwrap();
        assert_eq!(all.len(), 1, "upsert should not create duplicate rows");
    }
}
