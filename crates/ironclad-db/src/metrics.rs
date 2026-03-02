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
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO inference_costs \
         (id, model, provider, tokens_in, tokens_out, cost, tier, cached, latency_ms, quality_score, escalation) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
            escalation as i32
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
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO transactions (id, tx_type, amount, currency, counterparty, tx_hash) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, tx_type, amount, currency, counterparty, tx_hash],
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
        let id = record_inference_cost(&db, "gpt-4", "openai", 100, 50, 0.005, None, true, None, None, false).unwrap();
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
}
