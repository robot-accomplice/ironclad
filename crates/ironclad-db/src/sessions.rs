use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub agent_id: String,
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
        "SELECT id, agent_id, model, nickname, created_at, updated_at, metadata \
         FROM sessions WHERE id = ?1",
        [id],
        |row| {
            Ok(Session {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                model: row.get(2)?,
                nickname: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                metadata: row.get(6)?,
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

    let sentence_end = text
        .find(['.', '?', '!', '\n'])
        .unwrap_or(text.len());
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

    // ── derive_nickname tests ────────────────────────────────

    #[test]
    fn derive_nickname_strips_greeting() {
        assert_eq!(derive_nickname("Hey can you help me with Rust?"), "Can you help me with Rust");
    }

    #[test]
    fn derive_nickname_strips_hello() {
        assert_eq!(derive_nickname("Hello, I need a database schema"), "I need a database schema");
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
        assert_eq!(derive_nickname("refactor the auth module"), "Refactor the auth module");
    }

    #[test]
    fn derive_nickname_question_mark() {
        assert_eq!(derive_nickname("what is the meaning of life?"), "What is the meaning of life");
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
        let sid = find_or_create(&db, "agent-nick").unwrap();
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
        let sid = find_or_create(&db, "agent-overwrite").unwrap();
        update_nickname(&db, &sid, "First").unwrap();
        update_nickname(&db, &sid, "Second").unwrap();
        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("Second"));
    }

    // ── backfill_nicknames tests ─────────────────────────────

    #[test]
    fn backfill_nicknames_sets_from_first_message() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-bf").unwrap();
        append_message(&db, &sid, "user", "Help me debug this crash").unwrap();

        let count = backfill_nicknames(&db).unwrap();
        assert_eq!(count, 1);

        let session = get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.nickname.as_deref(), Some("Debug this crash"));
    }

    #[test]
    fn backfill_nicknames_untitled_for_empty_session() {
        let db = test_db();
        find_or_create(&db, "agent-empty-bf").unwrap();

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
        let sid = find_or_create(&db, "agent-skip-bf").unwrap();
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
        let sid = find_or_create(&db, "agent-count").unwrap();
        assert_eq!(message_count(&db, &sid).unwrap(), 0);
    }

    #[test]
    fn message_count_after_append() {
        let db = test_db();
        let sid = find_or_create(&db, "agent-count2").unwrap();
        append_message(&db, &sid, "user", "a").unwrap();
        append_message(&db, &sid, "assistant", "b").unwrap();
        append_message(&db, &sid, "user", "c").unwrap();
        assert_eq!(message_count(&db, &sid).unwrap(), 3);
    }
}
