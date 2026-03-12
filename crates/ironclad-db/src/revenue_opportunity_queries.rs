use crate::Database;
use ironclad_core::{IroncladError, Result};
use rusqlite::params;
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
pub struct RevenueOpportunityListQuery<'a> {
    pub status: Option<&'a str>,
    pub limit: usize,
}

pub fn list_revenue_opportunities(
    db: &Database,
    query: RevenueOpportunityListQuery<'_>,
) -> Result<Vec<Value>> {
    let conn = db.conn();
    let limit = query.limit.clamp(1, 200) as i64;
    let sql = if query.status.is_some() {
        "SELECT id, source, strategy, status, expected_revenue_usdc, confidence_score, effort_score, risk_score, priority_score, recommended_approved, score_reason, settlement_ref, settled_amount_usdc, net_profit_usdc, created_at, updated_at FROM revenue_opportunities WHERE status = ?1 ORDER BY priority_score DESC, created_at DESC LIMIT ?2"
    } else {
        "SELECT id, source, strategy, status, expected_revenue_usdc, confidence_score, effort_score, risk_score, priority_score, recommended_approved, score_reason, settlement_ref, settled_amount_usdc, net_profit_usdc, created_at, updated_at FROM revenue_opportunities ORDER BY priority_score DESC, created_at DESC LIMIT ?1"
    };
    let mut stmt = conn
        .prepare(sql)
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = if let Some(status) = query.status {
        stmt.query_map(params![status, limit], map_row)
    } else {
        stmt.query_map(params![limit], map_row)
    }
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Value> {
    Ok(json!({
        "id": row.get::<_, String>(0)?,
        "source": row.get::<_, String>(1)?,
        "strategy": row.get::<_, String>(2)?,
        "status": row.get::<_, String>(3)?,
        "expected_revenue_usdc": row.get::<_, f64>(4)?,
        "score": {
            "confidence_score": row.get::<_, f64>(5)?,
            "effort_score": row.get::<_, f64>(6)?,
            "risk_score": row.get::<_, f64>(7)?,
            "priority_score": row.get::<_, f64>(8)?,
            "recommended_approved": row.get::<_, i64>(9)? != 0,
            "score_reason": row.get::<_, Option<String>>(10)?,
        },
        "settlement_ref": row.get::<_, Option<String>>(11)?,
        "settled_amount_usdc": row.get::<_, Option<f64>>(12)?,
        "net_profit_usdc": row.get::<_, Option<f64>>(13)?,
        "created_at": row.get::<_, String>(14)?,
        "updated_at": row.get::<_, String>(15)?,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_revenue_opportunities_orders_by_priority_desc() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, priority_score) VALUES ('ro_1','a','micro_bounty','{}',1.0,'intake',25.0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, priority_score) VALUES ('ro_2','b','oracle_feed','{}',3.0,'qualified',80.0)",
            [],
        )
        .unwrap();
        drop(conn);

        let rows = list_revenue_opportunities(
            &db,
            RevenueOpportunityListQuery {
                status: None,
                limit: 10,
            },
        )
        .unwrap();
        assert_eq!(rows[0]["id"], "ro_2");
        assert_eq!(rows[1]["id"], "ro_1");
    }
}
