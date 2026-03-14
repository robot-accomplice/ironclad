use crate::{Database, DbResultExt};
use ironclad_core::Result;

#[derive(Debug, Clone)]
pub struct AbuseEventRecord {
    pub id: String,
    pub actor_id: String,
    pub origin: String,
    pub channel: String,
    pub signal_type: String,
    pub severity: String,
    pub action_taken: String,
    pub detail: Option<String>,
    pub score: f64,
    pub created_at: String,
}

#[allow(clippy::too_many_arguments)]
pub fn record_abuse_event(
    db: &Database,
    actor_id: &str,
    origin: &str,
    channel: &str,
    signal_type: &str,
    severity: &str,
    action_taken: &str,
    detail: Option<&str>,
    score: f64,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO abuse_events (id, actor_id, origin, channel, signal_type, severity, action_taken, detail, score) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![id, actor_id, origin, channel, signal_type, severity, action_taken, detail, score],
    )
    .db_err()?;
    Ok(id)
}

/// Returns recent abuse events for a given actor, ordered newest-first.
pub fn recent_events_for_actor(
    db: &Database,
    actor_id: &str,
    limit: i64,
) -> Result<Vec<AbuseEventRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, actor_id, origin, channel, signal_type, severity, action_taken, detail, score, created_at \
             FROM abuse_events WHERE actor_id = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )
        .db_err()?;

    let rows = stmt
        .query_map(rusqlite::params![actor_id, limit.max(1)], |row| {
            Ok(AbuseEventRecord {
                id: row.get(0)?,
                actor_id: row.get(1)?,
                origin: row.get(2)?,
                channel: row.get(3)?,
                signal_type: row.get(4)?,
                severity: row.get(5)?,
                action_taken: row.get(6)?,
                detail: row.get(7)?,
                score: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

/// Returns recent abuse events for a given origin (IP or source), ordered newest-first.
pub fn recent_events_for_origin(
    db: &Database,
    origin: &str,
    limit: i64,
) -> Result<Vec<AbuseEventRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, actor_id, origin, channel, signal_type, severity, action_taken, detail, score, created_at \
             FROM abuse_events WHERE origin = ?1 ORDER BY created_at DESC, rowid DESC LIMIT ?2",
        )
        .db_err()?;

    let rows = stmt
        .query_map(rusqlite::params![origin, limit.max(1)], |row| {
            Ok(AbuseEventRecord {
                id: row.get(0)?,
                actor_id: row.get(1)?,
                origin: row.get(2)?,
                channel: row.get(3)?,
                signal_type: row.get(4)?,
                severity: row.get(5)?,
                action_taken: row.get(6)?,
                detail: row.get(7)?,
                score: row.get(8)?,
                created_at: row.get(9)?,
            })
        })
        .db_err()?;

    rows.collect::<std::result::Result<Vec<_>, _>>().db_err()
}

/// Count abuse events since a given ISO-8601 timestamp (for aggregate scoring).
pub fn count_events_since(db: &Database, actor_id: &str, since: &str) -> Result<u64> {
    let conn = db.conn();
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM abuse_events WHERE actor_id = ?1 AND created_at >= ?2",
            rusqlite::params![actor_id, since],
            |row| row.get(0),
        )
        .db_err()?;
    Ok(count as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn record_and_retrieve_by_actor() {
        let db = test_db();
        let id = record_abuse_event(
            &db,
            "actor-1",
            "192.168.1.1",
            "api",
            "rate_burst",
            "medium",
            "slowdown",
            Some("50 requests in 5s"),
            0.65,
        )
        .unwrap();
        assert!(!id.is_empty());

        let events = recent_events_for_actor(&db, "actor-1", 10).unwrap();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].signal_type, "rate_burst");
        assert_eq!(events[0].action_taken, "slowdown");
        assert!((events[0].score - 0.65).abs() < f64::EPSILON);
    }

    #[test]
    fn retrieve_by_origin() {
        let db = test_db();
        record_abuse_event(
            &db,
            "a1",
            "10.0.0.1",
            "api",
            "rate_burst",
            "low",
            "allow",
            None,
            0.2,
        )
        .unwrap();
        record_abuse_event(
            &db,
            "a2",
            "10.0.0.1",
            "telegram",
            "spam",
            "high",
            "quarantine",
            None,
            0.9,
        )
        .unwrap();

        let events = recent_events_for_origin(&db, "10.0.0.1", 10).unwrap();
        assert_eq!(events.len(), 2);
        // Newest first
        assert_eq!(events[0].signal_type, "spam");
    }

    #[test]
    fn count_since_filters_correctly() {
        let db = test_db();
        record_abuse_event(
            &db, "actor-x", "ip1", "api", "burst", "low", "allow", None, 0.1,
        )
        .unwrap();
        // All events have created_at = datetime('now'), so count should include them
        let count = count_events_since(&db, "actor-x", "2020-01-01 00:00:00").unwrap();
        assert_eq!(count, 1);

        let count_future = count_events_since(&db, "actor-x", "2099-01-01 00:00:00").unwrap();
        assert_eq!(count_future, 0);
    }

    #[test]
    fn empty_actor_returns_empty() {
        let db = test_db();
        let events = recent_events_for_actor(&db, "nobody", 10).unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn respects_limit() {
        let db = test_db();
        for i in 0..5 {
            record_abuse_event(
                &db,
                "actor-many",
                "ip",
                "api",
                &format!("sig-{i}"),
                "low",
                "allow",
                None,
                0.1,
            )
            .unwrap();
        }
        let events = recent_events_for_actor(&db, "actor-many", 3).unwrap();
        assert_eq!(events.len(), 3);
    }
}
