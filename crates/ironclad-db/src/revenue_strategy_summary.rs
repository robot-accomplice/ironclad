use crate::Database;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};

pub fn revenue_strategy_summary(db: &Database) -> Result<Vec<Value>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT strategy, COUNT(*) AS total_jobs, \
                    SUM(CASE WHEN status = 'settled' THEN 1 ELSE 0 END) AS settled_jobs, \
                    SUM(COALESCE(settled_amount_usdc, 0)) AS gross_revenue_usdc, \
                    SUM(COALESCE(net_profit_usdc, 0)) AS net_profit_usdc, \
                    AVG(priority_score) AS avg_priority_score \
             FROM revenue_opportunities \
             GROUP BY strategy \
             ORDER BY net_profit_usdc DESC, gross_revenue_usdc DESC, strategy ASC \
             LIMIT 200",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "strategy": row.get::<_, String>(0)?,
                "total_jobs": row.get::<_, i64>(1)?,
                "settled_jobs": row.get::<_, i64>(2)?,
                "gross_revenue_usdc": row.get::<_, f64>(3)?,
                "net_profit_usdc": row.get::<_, f64>(4)?,
                "avg_priority_score": row.get::<_, f64>(5)?,
            }))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_strategy_summary_groups_by_strategy() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, net_profit_usdc, priority_score) VALUES ('ro_1','a','oracle_feed','{}',5.0,'settled',5.0,4.0,80.0)",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, net_profit_usdc, priority_score) VALUES ('ro_2','b','micro_bounty','{}',2.0,'settled',2.0,1.0,35.0)",
            [],
        ).unwrap();
        drop(conn);

        let rows = revenue_strategy_summary(&db).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["strategy"], "oracle_feed");
    }
}
