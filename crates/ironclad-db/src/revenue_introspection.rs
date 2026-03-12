//! # Revenue Introspection
//!
//! Unified introspection surface for the revenue pipeline.  Every function in
//! this module is designed to answer an operational question the agent (or
//! operator) might ask while deciding which strategies to pursue, which jobs to
//! approve, or whether the system is healthy.
//!
//! ## Introspection classes
//!
//! | Class            | Function                          | Question answered                                          |
//! |------------------|-----------------------------------|------------------------------------------------------------|
//! | Strategy summary | `strategy_summary`                | How is each strategy performing overall?                   |
//! | Profitability    | `strategy_profitability`          | Is this strategy worth pursuing at volume?                 |
//! | Audit log        | `audit_log`                       | What happened recently and how much did it earn?           |
//! | Feedback signal  | `feedback_signal_for_strategy`    | What does operator feedback say about a strategy?          |
//! | Pipeline health  | `pipeline_health`                 | Are there stuck/stale jobs or conversion anomalies?        |
//!
//! All functions return `Vec<serde_json::Value>` (or a single `Value` for
//! scalar reports) so they can be directly embedded in the dashboard JSON
//! without serialization gymnastics.

use crate::Database;
use ironclad_core::Result;
use serde_json::Value;

// ── Strategy summary ────────────────────────────────────────────────────
pub use crate::revenue_strategy_summary::revenue_strategy_summary as strategy_summary;

// ── Profitability ───────────────────────────────────────────────────────
pub use crate::revenue_strategy_summary::revenue_strategy_profitability as strategy_profitability;

// ── Audit log ───────────────────────────────────────────────────────────
pub use crate::revenue_strategy_summary::revenue_audit_log as audit_log;

// ── Feedback signal ─────────────────────────────────────────────────────
pub use crate::revenue_feedback::revenue_feedback_summary_by_strategy as feedback_summary;

// ── Pipeline health (new) ───────────────────────────────────────────────
/// Quick pipeline health check: counts of jobs by status, identifies stale
/// intake jobs (> 24 h old), and flags strategies with zero settlements.
pub fn pipeline_health(db: &Database) -> Result<Value> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT status, COUNT(*) AS cnt \
             FROM revenue_opportunities \
             GROUP BY status \
             ORDER BY cnt DESC",
        )
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
    let status_counts: Vec<Value> = stmt
        .query_map([], |row| {
            Ok(serde_json::json!({
                "status": row.get::<_, String>(0)?,
                "count": row.get::<_, i64>(1)?,
            }))
        })
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

    // Stale intake jobs: created > 24 hours ago and still in intake
    let stale_intake: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM revenue_opportunities \
             WHERE status = 'intake' \
             AND created_at < datetime('now', '-24 hours')",
            [],
            |row| row.get(0),
        )
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

    // Strategies with no settlements ever
    let mut stmt2 = conn
        .prepare(
            "SELECT strategy, COUNT(*) AS total \
             FROM revenue_opportunities \
             GROUP BY strategy \
             HAVING SUM(CASE WHEN status = 'settled' THEN 1 ELSE 0 END) = 0 \
             ORDER BY total DESC \
             LIMIT 50",
        )
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
    let zero_settlement_strategies: Vec<Value> = stmt2
        .query_map([], |row| {
            Ok(serde_json::json!({
                "strategy": row.get::<_, String>(0)?,
                "total_jobs": row.get::<_, i64>(1)?,
            }))
        })
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

    Ok(serde_json::json!({
        "status_distribution": status_counts,
        "stale_intake_count": stale_intake,
        "zero_settlement_strategies": zero_settlement_strategies,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_health_with_mixed_statuses() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) \
             VALUES ('ro_1','a','oracle_feed','{}',5.0,'intake')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc) \
             VALUES ('ro_2','b','oracle_feed','{}',10.0,'settled',10.0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) \
             VALUES ('ro_3','c','micro_bounty','{}',2.0,'rejected')",
            [],
        ).unwrap();
        drop(conn);

        let health = pipeline_health(&db).unwrap();
        let statuses = health["status_distribution"].as_array().unwrap();
        assert_eq!(statuses.len(), 3); // intake, settled, rejected

        // micro_bounty has zero settlements
        let zero = health["zero_settlement_strategies"].as_array().unwrap();
        assert_eq!(zero.len(), 1);
        assert_eq!(zero[0]["strategy"], "micro_bounty");
    }

    #[test]
    fn pipeline_health_empty_db() {
        let db = Database::new(":memory:").unwrap();
        let health = pipeline_health(&db).unwrap();
        assert_eq!(health["stale_intake_count"], 0);
        assert!(health["status_distribution"].as_array().unwrap().is_empty());
    }
}
