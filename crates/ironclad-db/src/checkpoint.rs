use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::OptionalExtension;

#[derive(Debug, Clone)]
pub struct ContextCheckpoint {
    pub id: String,
    pub session_id: String,
    pub system_prompt_hash: String,
    pub memory_summary: String,
    pub active_tasks: Option<String>,
    pub conversation_digest: Option<String>,
    pub turn_count: i64,
    pub created_at: String,
}

/// Save a new checkpoint for a session.
pub fn save_checkpoint(
    db: &Database,
    session_id: &str,
    system_prompt_hash: &str,
    memory_summary: &str,
    active_tasks: Option<&str>,
    conversation_digest: Option<&str>,
    turn_count: i64,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO context_checkpoints (id, session_id, system_prompt_hash, memory_summary, active_tasks, conversation_digest, turn_count) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, session_id, system_prompt_hash, memory_summary, active_tasks, conversation_digest, turn_count],
    ).map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

/// Load the most recent checkpoint for a session.
pub fn load_checkpoint(db: &Database, session_id: &str) -> Result<Option<ContextCheckpoint>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT id, session_id, system_prompt_hash, memory_summary, active_tasks, conversation_digest, turn_count, created_at \
         FROM context_checkpoints WHERE session_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT 1",
        [session_id],
        |row| {
            Ok(ContextCheckpoint {
                id: row.get(0)?,
                session_id: row.get(1)?,
                system_prompt_hash: row.get(2)?,
                memory_summary: row.get(3)?,
                active_tasks: row.get(4)?,
                conversation_digest: row.get(5)?,
                turn_count: row.get(6)?,
                created_at: row.get(7)?,
            })
        },
    )
    .optional()
    .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Delete all checkpoints for a session (used on session archive/expiry).
pub fn clear_checkpoints(db: &Database, session_id: &str) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM context_checkpoints WHERE session_id = ?1",
            [session_id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(deleted)
}

/// Keep only the most recent `keep_per_session` checkpoints per session,
/// deleting older ones.  Returns the total number of rows deleted.
pub fn prune_checkpoints(db: &Database, keep_per_session: usize) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM context_checkpoints \
             WHERE rowid NOT IN ( \
               SELECT rowid FROM ( \
                 SELECT rowid, ROW_NUMBER() OVER (PARTITION BY session_id ORDER BY created_at DESC, rowid DESC) AS rn \
                 FROM context_checkpoints \
               ) WHERE rn <= ?1 \
             )",
            [keep_per_session as i64],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(deleted)
}

/// Count checkpoints for a session.
pub fn count_checkpoints(db: &Database, session_id: &str) -> Result<i64> {
    let conn = db.conn();
    conn.query_row(
        "SELECT COUNT(*) FROM context_checkpoints WHERE session_id = ?1",
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

    fn create_session(db: &Database) -> String {
        crate::sessions::find_or_create(db, "test-agent", None).unwrap()
    }

    #[test]
    fn save_and_load_checkpoint() {
        let db = test_db();
        let sid = create_session(&db);
        let cid = save_checkpoint(
            &db,
            &sid,
            "hash123",
            "memory summary",
            Some("tasks"),
            Some("digest"),
            10,
        )
        .unwrap();
        assert!(!cid.is_empty());

        let cp = load_checkpoint(&db, &sid).unwrap().unwrap();
        assert_eq!(cp.session_id, sid);
        assert_eq!(cp.system_prompt_hash, "hash123");
        assert_eq!(cp.memory_summary, "memory summary");
        assert_eq!(cp.active_tasks.as_deref(), Some("tasks"));
        assert_eq!(cp.conversation_digest.as_deref(), Some("digest"));
        assert_eq!(cp.turn_count, 10);
    }

    #[test]
    fn load_checkpoint_returns_most_recent() {
        let db = test_db();
        let sid = create_session(&db);
        save_checkpoint(&db, &sid, "old", "old summary", None, None, 5).unwrap();
        save_checkpoint(&db, &sid, "new", "new summary", None, None, 15).unwrap();

        let cp = load_checkpoint(&db, &sid).unwrap().unwrap();
        assert_eq!(cp.system_prompt_hash, "new");
        assert_eq!(cp.turn_count, 15);
    }

    #[test]
    fn load_checkpoint_no_session_returns_none() {
        let db = test_db();
        let cp = load_checkpoint(&db, "nonexistent").unwrap();
        assert!(cp.is_none());
    }

    #[test]
    fn clear_checkpoints_removes_all() {
        let db = test_db();
        let sid = create_session(&db);
        save_checkpoint(&db, &sid, "h1", "s1", None, None, 1).unwrap();
        save_checkpoint(&db, &sid, "h2", "s2", None, None, 2).unwrap();

        let cleared = clear_checkpoints(&db, &sid).unwrap();
        assert_eq!(cleared, 2);

        let cp = load_checkpoint(&db, &sid).unwrap();
        assert!(cp.is_none());
    }

    #[test]
    fn count_checkpoints_accurate() {
        let db = test_db();
        let sid = create_session(&db);
        assert_eq!(count_checkpoints(&db, &sid).unwrap(), 0);
        save_checkpoint(&db, &sid, "h1", "s1", None, None, 1).unwrap();
        assert_eq!(count_checkpoints(&db, &sid).unwrap(), 1);
        save_checkpoint(&db, &sid, "h2", "s2", None, None, 2).unwrap();
        assert_eq!(count_checkpoints(&db, &sid).unwrap(), 2);
    }

    #[test]
    fn checkpoint_with_no_optional_fields() {
        let db = test_db();
        let sid = create_session(&db);
        save_checkpoint(&db, &sid, "hash", "summary", None, None, 0).unwrap();
        let cp = load_checkpoint(&db, &sid).unwrap().unwrap();
        assert!(cp.active_tasks.is_none());
        assert!(cp.conversation_digest.is_none());
    }

    #[test]
    fn prune_checkpoints_keeps_n_per_session() {
        let db = test_db();
        let s1 = create_session(&db);
        let s2 = crate::sessions::find_or_create(&db, "agent-b", None).unwrap();

        // Create 5 checkpoints for s1, 3 for s2
        for i in 0..5 {
            save_checkpoint(&db, &s1, &format!("h{i}"), &format!("s{i}"), None, None, i).unwrap();
        }
        for i in 0..3 {
            save_checkpoint(&db, &s2, &format!("h{i}"), &format!("s{i}"), None, None, i).unwrap();
        }
        assert_eq!(count_checkpoints(&db, &s1).unwrap(), 5);
        assert_eq!(count_checkpoints(&db, &s2).unwrap(), 3);

        // Keep 2 per session → should delete 3 from s1, 1 from s2
        let pruned = prune_checkpoints(&db, 2).unwrap();
        assert_eq!(pruned, 4);
        assert_eq!(count_checkpoints(&db, &s1).unwrap(), 2);
        assert_eq!(count_checkpoints(&db, &s2).unwrap(), 2);

        // Most recent checkpoint for s1 should have turn_count=4
        let cp = load_checkpoint(&db, &s1).unwrap().unwrap();
        assert_eq!(cp.turn_count, 4);
    }
}
