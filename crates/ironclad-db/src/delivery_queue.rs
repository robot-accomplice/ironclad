use chrono::{DateTime, NaiveDateTime, Utc};
use rusqlite::params;

use ironclad_core::{IroncladError, Result};

use crate::Database;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeliveryQueueRecord {
    pub id: String,
    pub channel: String,
    pub recipient_id: String,
    pub content: String,
    pub status: String,
    pub attempts: u32,
    pub max_attempts: u32,
    pub next_retry_at: DateTime<Utc>,
    pub last_error: Option<String>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
}

fn parse_db_ts(input: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(input)
        .map(|dt| dt.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            NaiveDateTime::parse_from_str(input, "%Y-%m-%d %H:%M:%S")
                .ok()
                .map(|ndt| DateTime::<Utc>::from_naive_utc_and_offset(ndt, Utc))
        })
}

pub fn upsert_delivery_item(db: &Database, item: &DeliveryQueueRecord) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        r#"
        INSERT INTO delivery_queue (
            id, channel, recipient_id, content, status, attempts, max_attempts,
            next_retry_at, last_error, idempotency_key, created_at
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)
        ON CONFLICT(id) DO UPDATE SET
            channel = excluded.channel,
            recipient_id = excluded.recipient_id,
            content = excluded.content,
            status = excluded.status,
            attempts = excluded.attempts,
            max_attempts = excluded.max_attempts,
            next_retry_at = excluded.next_retry_at,
            last_error = excluded.last_error,
            idempotency_key = excluded.idempotency_key
        "#,
        params![
            item.id,
            item.channel,
            item.recipient_id,
            item.content,
            item.status,
            item.attempts,
            item.max_attempts,
            item.next_retry_at.to_rfc3339(),
            item.last_error,
            item.idempotency_key,
            item.created_at.to_rfc3339(),
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn list_recoverable(db: &Database, max_items: usize) -> Result<Vec<DeliveryQueueRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, channel, recipient_id, content, status, attempts, max_attempts,
                   next_retry_at, last_error, idempotency_key, created_at
            FROM delivery_queue
            WHERE status IN ('pending', 'in_flight')
            ORDER BY next_retry_at ASC
            LIMIT ?1
            "#,
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![max_items as i64], |row| {
            let next_retry_raw: String = row.get(7)?;
            let created_raw: String = row.get(10)?;
            Ok(DeliveryQueueRecord {
                id: row.get(0)?,
                channel: row.get(1)?,
                recipient_id: row.get(2)?,
                content: row.get(3)?,
                status: row.get(4)?,
                attempts: row.get::<_, i64>(5)? as u32,
                max_attempts: row.get::<_, i64>(6)? as u32,
                next_retry_at: parse_db_ts(&next_retry_raw).unwrap_or_else(|| {
                    tracing::warn!(raw = %next_retry_raw, "corrupt next_retry_at timestamp, using epoch");
                    DateTime::<Utc>::UNIX_EPOCH
                }),
                last_error: row.get(8)?,
                idempotency_key: row.get(9)?,
                created_at: parse_db_ts(&created_raw).unwrap_or_else(Utc::now),
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn mark_delivered(db: &Database, id: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE delivery_queue SET status = 'delivered', last_error = NULL WHERE id = ?1",
        params![id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn mark_in_flight(db: &Database, id: &str) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "UPDATE delivery_queue SET status = 'in_flight' WHERE id = ?1",
        params![id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(())
}

pub fn list_dead_letters(db: &Database, max_items: usize) -> Result<Vec<DeliveryQueueRecord>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            r#"
            SELECT id, channel, recipient_id, content, status, attempts, max_attempts,
                   next_retry_at, last_error, idempotency_key, created_at
            FROM delivery_queue
            WHERE status = 'dead_letter'
            ORDER BY created_at DESC
            LIMIT ?1
            "#,
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let rows = stmt
        .query_map(params![max_items as i64], |row| {
            let next_retry_raw: String = row.get(7)?;
            let created_raw: String = row.get(10)?;
            Ok(DeliveryQueueRecord {
                id: row.get(0)?,
                channel: row.get(1)?,
                recipient_id: row.get(2)?,
                content: row.get(3)?,
                status: row.get(4)?,
                attempts: row.get::<_, i64>(5)? as u32,
                max_attempts: row.get::<_, i64>(6)? as u32,
                next_retry_at: parse_db_ts(&next_retry_raw).unwrap_or_else(|| {
                    tracing::warn!(raw = %next_retry_raw, "corrupt next_retry_at timestamp, using epoch");
                    DateTime::<Utc>::UNIX_EPOCH
                }),
                last_error: row.get(8)?,
                idempotency_key: row.get(9)?,
                created_at: parse_db_ts(&created_raw).unwrap_or_else(Utc::now),
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn replay_dead_letter(db: &Database, id: &str) -> Result<bool> {
    let conn = db.conn();
    let rows = conn
        .execute(
            "UPDATE delivery_queue SET status = 'pending', next_retry_at = ?1 WHERE id = ?2 AND status = 'dead_letter'",
            params![Utc::now().to_rfc3339(), id],
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(rows > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn upsert_and_list_recoverable() {
        let db = Database::new(":memory:").expect("db");
        let item = DeliveryQueueRecord {
            id: "d1".into(),
            channel: "telegram".into(),
            recipient_id: "u1".into(),
            content: "hello".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-1".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).expect("upsert");
        let rows = list_recoverable(&db, 20).expect("load");
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "d1");
    }

    #[test]
    fn mark_delivered_updates_status() {
        let db = Database::new(":memory:").expect("db");
        let item = DeliveryQueueRecord {
            id: "d2".into(),
            channel: "discord".into(),
            recipient_id: "u2".into(),
            content: "msg".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-2".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).expect("upsert");
        mark_delivered(&db, "d2").expect("mark delivered");
        let rows = list_recoverable(&db, 20).expect("load");
        assert!(rows.is_empty(), "delivered rows should not be recoverable");
    }

    #[test]
    fn replay_dead_letter_moves_back_to_pending() {
        let db = Database::new(":memory:").expect("db");
        let item = DeliveryQueueRecord {
            id: "d3".into(),
            channel: "discord".into(),
            recipient_id: "u2".into(),
            content: "msg".into(),
            status: "dead_letter".into(),
            attempts: 5,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: Some("failed".into()),
            idempotency_key: "idem-3".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).expect("upsert");
        assert_eq!(list_dead_letters(&db, 10).expect("dead").len(), 1);
        assert!(replay_dead_letter(&db, "d3").expect("replay"));
        let recovered = list_recoverable(&db, 10).expect("recoverable");
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].status, "pending");
    }

    #[test]
    fn mark_in_flight_updates_status() {
        let db = Database::new(":memory:").expect("db");
        let item = DeliveryQueueRecord {
            id: "d4".into(),
            channel: "telegram".into(),
            recipient_id: "u1".into(),
            content: "hi".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-4".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).unwrap();
        mark_in_flight(&db, "d4").unwrap();

        // in_flight items are still recoverable
        let rows = list_recoverable(&db, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "in_flight");
    }

    #[test]
    fn mark_in_flight_nonexistent_is_noop() {
        let db = Database::new(":memory:").expect("db");
        mark_in_flight(&db, "nonexistent").unwrap();
    }

    #[test]
    fn parse_db_ts_rfc3339() {
        let ts = parse_db_ts("2025-06-01T12:00:00+00:00").unwrap();
        assert_eq!(ts.year(), 2025);
        assert_eq!(ts.month(), 6);
    }

    #[test]
    fn parse_db_ts_sqlite_format() {
        // SQLite default datetime('now') format: "YYYY-MM-DD HH:MM:SS"
        let ts = parse_db_ts("2025-06-01 12:00:00").unwrap();
        assert_eq!(ts.year(), 2025);
        assert_eq!(ts.month(), 6);
    }

    #[test]
    fn parse_db_ts_invalid_returns_none() {
        assert!(parse_db_ts("not-a-date").is_none());
        assert!(parse_db_ts("").is_none());
    }

    #[test]
    fn list_dead_letters_empty() {
        let db = Database::new(":memory:").expect("db");
        let dead = list_dead_letters(&db, 10).unwrap();
        assert!(dead.is_empty());
    }

    #[test]
    fn list_recoverable_empty() {
        let db = Database::new(":memory:").expect("db");
        let rows = list_recoverable(&db, 10).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn replay_dead_letter_nonexistent_returns_false() {
        let db = Database::new(":memory:").expect("db");
        assert!(!replay_dead_letter(&db, "missing").unwrap());
    }

    #[test]
    fn replay_non_dead_letter_returns_false() {
        let db = Database::new(":memory:").expect("db");
        let item = DeliveryQueueRecord {
            id: "d5".into(),
            channel: "email".into(),
            recipient_id: "u1".into(),
            content: "hello".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 3,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-5".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).unwrap();
        // Should not replay a pending item
        assert!(!replay_dead_letter(&db, "d5").unwrap());
    }

    #[test]
    fn upsert_updates_existing() {
        let db = Database::new(":memory:").expect("db");
        let mut item = DeliveryQueueRecord {
            id: "d6".into(),
            channel: "telegram".into(),
            recipient_id: "u1".into(),
            content: "first".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-6".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &item).unwrap();

        item.content = "updated".into();
        item.attempts = 1;
        item.last_error = Some("timeout".into());
        upsert_delivery_item(&db, &item).unwrap();

        let rows = list_recoverable(&db, 10).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].content, "updated");
        assert_eq!(rows[0].attempts, 1);
        assert_eq!(rows[0].last_error.as_deref(), Some("timeout"));
    }

    #[test]
    fn list_dead_letters_only_dead() {
        let db = Database::new(":memory:").expect("db");
        let pending = DeliveryQueueRecord {
            id: "d7".into(),
            channel: "email".into(),
            recipient_id: "u1".into(),
            content: "hi".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 3,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "idem-7".into(),
            created_at: Utc::now(),
        };
        let dead = DeliveryQueueRecord {
            id: "d8".into(),
            channel: "email".into(),
            recipient_id: "u2".into(),
            content: "failed msg".into(),
            status: "dead_letter".into(),
            attempts: 5,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: Some("permanent failure".into()),
            idempotency_key: "idem-8".into(),
            created_at: Utc::now(),
        };
        upsert_delivery_item(&db, &pending).unwrap();
        upsert_delivery_item(&db, &dead).unwrap();

        let dead_items = list_dead_letters(&db, 10).unwrap();
        assert_eq!(dead_items.len(), 1);
        assert_eq!(dead_items[0].id, "d8");
    }
}
