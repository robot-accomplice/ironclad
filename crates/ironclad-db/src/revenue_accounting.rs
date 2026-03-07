use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Default)]
pub struct RevenueAccountingSummary {
    pub settled_jobs: i64,
    pub gross_revenue_usdc: f64,
    pub attributable_costs_usdc: f64,
    pub net_profit_usdc: f64,
    pub tax_paid_usdc: f64,
    pub retained_earnings_usdc: f64,
}

pub fn revenue_accounting_summary(db: &Database) -> Result<RevenueAccountingSummary> {
    let conn = db.conn();
    conn.query_row(
        "SELECT \
            COUNT(*), \
            COALESCE(SUM(COALESCE(settled_amount_usdc, 0)), 0), \
            COALESCE(SUM(COALESCE(attributable_costs_usdc, 0)), 0), \
            COALESCE(SUM(COALESCE(net_profit_usdc, 0)), 0), \
            COALESCE(SUM(COALESCE(tax_amount_usdc, 0)), 0), \
            COALESCE(SUM(COALESCE(retained_earnings_usdc, 0)), 0) \
         FROM revenue_opportunities \
         WHERE status = 'settled'",
        [],
        |row| {
            Ok(RevenueAccountingSummary {
                settled_jobs: row.get(0)?,
                gross_revenue_usdc: row.get(1)?,
                attributable_costs_usdc: row.get(2)?,
                net_profit_usdc: row.get(3)?,
                tax_paid_usdc: row.get(4)?,
                retained_earnings_usdc: row.get(5)?,
            })
        },
    )
    .map_err(|e| IroncladError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_accounting_summary_aggregates_settled_rows() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities \
             (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, attributable_costs_usdc, net_profit_usdc, tax_rate, tax_amount_usdc, retained_earnings_usdc) \
             VALUES ('r1','src','svc','{}',5.0,'settled',5.0,1.5,3.5,0.2,0.7,2.8)",
            [],
        )
        .unwrap();
        drop(conn);
        let summary = revenue_accounting_summary(&db).unwrap();
        assert_eq!(summary.settled_jobs, 1);
        assert!((summary.gross_revenue_usdc - 5.0).abs() < f64::EPSILON);
        assert!((summary.attributable_costs_usdc - 1.5).abs() < f64::EPSILON);
        assert!((summary.net_profit_usdc - 3.5).abs() < f64::EPSILON);
        assert!((summary.tax_paid_usdc - 0.7).abs() < f64::EPSILON);
        assert!((summary.retained_earnings_usdc - 2.8).abs() < f64::EPSILON);
    }
}
