use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct TransactionRecord {
    pub id: String,
    pub tx_type: String,
    pub amount: f64,
    pub currency: String,
    pub counterparty: Option<String>,
    pub tx_hash: Option<String>,
    pub metadata_json: Option<String>,
    pub created_at: String,
}

#[allow(clippy::too_many_arguments)]
pub fn record_inference_cost(
    db: &Database,
    model: &str,
    provider: &str,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    tier: Option<&str>,
    cached: bool,
    latency_ms: Option<i64>,
    quality_score: Option<f64>,
    escalation: bool,
    turn_id: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO inference_costs \
         (id, model, provider, tokens_in, tokens_out, cost, tier, cached, latency_ms, quality_score, escalation, turn_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
        rusqlite::params![
            id,
            model,
            provider,
            tokens_in,
            tokens_out,
            cost,
            tier,
            cached as i32,
            latency_ms,
            quality_score,
            escalation as i32,
            turn_id
        ],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn record_transaction(
    db: &Database,
    tx_type: &str,
    amount: f64,
    currency: &str,
    counterparty: Option<&str>,
    tx_hash: Option<&str>,
) -> Result<String> {
    record_transaction_with_metadata(db, tx_type, amount, currency, counterparty, tx_hash, None)
}

pub fn record_transaction_with_metadata(
    db: &Database,
    tx_type: &str,
    amount: f64,
    currency: &str,
    counterparty: Option<&str>,
    tx_hash: Option<&str>,
    metadata_json: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO transactions (id, tx_type, amount, currency, counterparty, tx_hash, metadata_json) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![id, tx_type, amount, currency, counterparty, tx_hash, metadata_json],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn query_transactions(db: &Database, hours: i64) -> Result<Vec<TransactionRecord>> {
    // Ensure hours is positive to prevent a negative value from producing
    // a malformed datetime modifier (e.g., "--5 hours" becomes a SQL comment).
    let hours = hours.unsigned_abs().max(1);
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, tx_type, amount, currency, counterparty, tx_hash, metadata_json, created_at \
             FROM transactions \
             WHERE created_at >= datetime('now', ?1) \
             ORDER BY created_at DESC",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let offset = format!("-{hours} hours");
    let rows = stmt
        .query_map([&offset], |row| {
            Ok(TransactionRecord {
                id: row.get(0)?,
                tx_type: row.get(1)?,
                amount: row.get(2)?,
                currency: row.get(3)?,
                counterparty: row.get(4)?,
                tx_hash: row.get(5)?,
                metadata_json: row.get(6)?,
                created_at: row.get(7)?,
            })
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Return the most recent quality observations from `inference_costs`, ordered
/// oldest-first so that the caller can feed them into a ring buffer in chronological
/// order. Each row is `(model, quality_score)`.
pub fn recent_quality_scores(db: &Database, limit: i64) -> Result<Vec<(String, f64)>> {
    let limit = limit.max(1);
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT model, quality_score FROM inference_costs \
             WHERE quality_score IS NOT NULL \
             ORDER BY created_at DESC, rowid DESC LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows: Vec<(String, f64)> = stmt
        .query_map(rusqlite::params![limit], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?
        .filter_map(|r| r.ok())
        .collect();
    // Reverse so oldest comes first (ring buffer insertion order).
    let mut rows = rows;
    rows.reverse();
    Ok(rows)
}

pub fn record_metric_snapshot(db: &Database, metrics_json: &str) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO metric_snapshots (id, metrics_json) VALUES (?1, ?2)",
        rusqlite::params![id, metrics_json],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn record_and_query_inference_cost() {
        let db = test_db();
        let id = record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            1000,
            500,
            0.015,
            Some("T1"),
            false,
            Some(150),
            Some(0.92),
            false,
            None,
        )
        .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn record_and_query_transactions() {
        let db = test_db();
        record_transaction(&db, "inference", 0.01, "USD", Some("anthropic"), None).unwrap();
        record_transaction(&db, "earning", 1.00, "USDC", Some("user-42"), Some("0xabc")).unwrap();

        let txs = query_transactions(&db, 24).unwrap();
        assert_eq!(txs.len(), 2);
    }

    #[test]
    fn record_metric_snapshot_works() {
        let db = test_db();
        let id = record_metric_snapshot(&db, r#"{"cpu":0.5,"mem_mb":128}"#).unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn query_transactions_empty() {
        let db = test_db();
        let txs = query_transactions(&db, 1).unwrap();
        assert!(txs.is_empty());
    }

    #[test]
    fn record_transaction_all_optional_none() {
        let db = test_db();
        let id = record_transaction(&db, "yield", 0.5, "USDC", None, None).unwrap();
        assert!(!id.is_empty());
        let txs = query_transactions(&db, 24).unwrap();
        assert_eq!(txs.len(), 1);
        assert!(txs[0].counterparty.is_none());
        assert!(txs[0].tx_hash.is_none());
    }

    #[test]
    fn record_inference_cost_cached() {
        let db = test_db();
        let id = record_inference_cost(
            &db, "gpt-4", "openai", 100, 50, 0.005, None, true, None, None, false, None,
        )
        .unwrap();
        assert!(!id.is_empty());
    }

    #[test]
    fn transaction_record_fields_populated() {
        let db = test_db();
        record_transaction(&db, "payment", 10.0, "USDC", Some("vendor"), Some("0xhash")).unwrap();
        let txs = query_transactions(&db, 24).unwrap();
        assert_eq!(txs[0].tx_type, "payment");
        assert!((txs[0].amount - 10.0).abs() < f64::EPSILON);
        assert_eq!(txs[0].currency, "USDC");
        assert_eq!(txs[0].counterparty.as_deref(), Some("vendor"));
        assert_eq!(txs[0].tx_hash.as_deref(), Some("0xhash"));
        assert!(!txs[0].created_at.is_empty());
    }

    #[test]
    fn multiple_metric_snapshots() {
        let db = test_db();
        let id1 = record_metric_snapshot(&db, r#"{"cpu":0.1}"#).unwrap();
        let id2 = record_metric_snapshot(&db, r#"{"cpu":0.9}"#).unwrap();
        assert_ne!(id1, id2);
    }

    #[test]
    fn recent_quality_scores_empty() {
        let db = test_db();
        let scores = recent_quality_scores(&db, 10).unwrap();
        assert!(scores.is_empty());
    }

    #[test]
    fn recent_quality_scores_returns_oldest_first() {
        let db = test_db();
        // Insert three rows with quality scores.
        record_inference_cost(
            &db,
            "model-a",
            "prov",
            100,
            50,
            0.01,
            None,
            false,
            Some(100),
            Some(0.7),
            false,
            None,
        )
        .unwrap();
        record_inference_cost(
            &db,
            "model-b",
            "prov",
            200,
            100,
            0.02,
            None,
            false,
            Some(200),
            Some(0.9),
            false,
            None,
        )
        .unwrap();
        record_inference_cost(
            &db,
            "model-a",
            "prov",
            150,
            75,
            0.015,
            None,
            false,
            Some(150),
            Some(0.85),
            false,
            None,
        )
        .unwrap();

        let scores = recent_quality_scores(&db, 10).unwrap();
        assert_eq!(scores.len(), 3);
        // Oldest first means first inserted row comes first.
        assert_eq!(scores[0].0, "model-a");
        assert!((scores[0].1 - 0.7).abs() < f64::EPSILON);
        assert_eq!(scores[2].0, "model-a");
        assert!((scores[2].1 - 0.85).abs() < f64::EPSILON);
    }

    #[test]
    fn recent_quality_scores_skips_null() {
        let db = test_db();
        record_inference_cost(
            &db,
            "m",
            "p",
            100,
            50,
            0.01,
            None,
            false,
            None,
            Some(0.8),
            false,
            None,
        )
        .unwrap();
        // Insert a row with NULL quality_score.
        record_inference_cost(
            &db, "m", "p", 100, 50, 0.01, None, true, None, None, false, None,
        )
        .unwrap();
        let scores = recent_quality_scores(&db, 10).unwrap();
        assert_eq!(scores.len(), 1);
        assert!((scores[0].1 - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn recent_quality_scores_respects_limit() {
        let db = test_db();
        for i in 0..5 {
            record_inference_cost(
                &db,
                "m",
                "p",
                100,
                50,
                0.01,
                None,
                false,
                None,
                Some(i as f64 * 0.2),
                false,
                None,
            )
            .unwrap();
        }
        let scores = recent_quality_scores(&db, 3).unwrap();
        assert_eq!(scores.len(), 3);
    }
}
