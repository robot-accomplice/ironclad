use crate::{Database, DbResultExt};
use ironclad_core::Result;

#[derive(Debug, Clone, Default)]
pub struct RevenueAccountingSummary {
    pub settled_jobs: i64,
    pub gross_revenue_usdc: f64,
    pub attributable_costs_usdc: f64,
    pub net_profit_usdc: f64,
    pub tax_paid_usdc: f64,
    pub retained_earnings_usdc: f64,
}

#[derive(Debug, Clone, Default)]
pub struct RevenueSwapQueueSummary {
    pub total: i64,
    pub pending: i64,
    pub in_progress: i64,
    pub failed: i64,
    pub completed: i64,
    pub stale_in_progress: i64,
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
    .db_err()
}

pub fn revenue_swap_queue_summary(db: &Database) -> Result<RevenueSwapQueueSummary> {
    let conn = db.conn();
    let tasks_exists: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='tasks'",
            [],
            |row| row.get(0),
        )
        .db_err()?;
    if tasks_exists == 0 {
        return Ok(RevenueSwapQueueSummary::default());
    }
    conn.query_row(
        "SELECT \
            COUNT(*), \
            COALESCE(SUM(CASE WHEN lower(status) = 'pending' THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN lower(status) = 'in_progress' THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN lower(status) = 'failed' THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN lower(status) IN ('completed', 'done', 'settled') THEN 1 ELSE 0 END), 0), \
            COALESCE(SUM(CASE WHEN lower(status) = 'in_progress' AND datetime(COALESCE(updated_at, created_at)) < datetime('now','-24 hours') THEN 1 ELSE 0 END), 0) \
         FROM tasks \
         WHERE lower(COALESCE(source, '')) LIKE '%\"type\":\"revenue_swap\"%'",
        [],
        |row| {
            Ok(RevenueSwapQueueSummary {
                total: row.get(0)?,
                pending: row.get(1)?,
                in_progress: row.get(2)?,
                failed: row.get(3)?,
                completed: row.get(4)?,
                stale_in_progress: row.get(5)?,
            })
        },
    )
    .db_err()
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

    #[test]
    fn revenue_swap_queue_summary_counts_swap_tasks() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO tasks (id, title, status, priority, source, created_at, updated_at) \
             VALUES \
             ('s1','swap 1','pending',95,'{\"type\":\"revenue_swap\"}',datetime('now'),datetime('now')), \
             ('s2','swap 2','in_progress',95,'{\"type\":\"revenue_swap\"}',datetime('now','-2 days'),datetime('now','-2 days')), \
             ('s3','swap 3','failed',95,'{\"type\":\"revenue_swap\"}',datetime('now'),datetime('now')), \
             ('s4','swap 4','completed',95,'{\"type\":\"revenue_swap\"}',datetime('now'),datetime('now')), \
             ('x1','other','pending',10,'{\"type\":\"other\"}',datetime('now'),datetime('now'))",
            [],
        )
        .unwrap();
        drop(conn);

        let summary = revenue_swap_queue_summary(&db).unwrap();
        assert_eq!(summary.total, 4);
        assert_eq!(summary.pending, 1);
        assert_eq!(summary.in_progress, 1);
        assert_eq!(summary.failed, 1);
        assert_eq!(summary.completed, 1);
        assert_eq!(summary.stale_in_progress, 1);
    }
}
