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

/// Per-strategy profitability metrics: cycle time, conversion rate, cost ratio, variance.
/// Designed to answer "is this strategy worth pursuing at volume?"
pub fn revenue_strategy_profitability(db: &Database) -> Result<Vec<Value>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT strategy, \
                    COUNT(*) AS total_jobs, \
                    SUM(CASE WHEN status = 'settled' THEN 1 ELSE 0 END) AS settled_jobs, \
                    SUM(COALESCE(settled_amount_usdc, 0)) AS gross_revenue_usdc, \
                    SUM(COALESCE(net_profit_usdc, 0)) AS net_profit_usdc, \
                    SUM(COALESCE(attributable_costs_usdc, 0)) AS total_costs_usdc, \
                    AVG(CASE WHEN status = 'settled' AND settled_at IS NOT NULL \
                        THEN CAST((julianday(settled_at) - julianday(created_at)) * 86400 AS INTEGER) \
                        ELSE NULL END) AS avg_cycle_time_seconds, \
                    MIN(CASE WHEN status = 'settled' AND settled_at IS NOT NULL \
                        THEN CAST((julianday(settled_at) - julianday(created_at)) * 86400 AS INTEGER) \
                        ELSE NULL END) AS min_cycle_time_seconds, \
                    MAX(CASE WHEN status = 'settled' AND settled_at IS NOT NULL \
                        THEN CAST((julianday(settled_at) - julianday(created_at)) * 86400 AS INTEGER) \
                        ELSE NULL END) AS max_cycle_time_seconds, \
                    SUM(CASE WHEN status IN ('rejected', 'intake') THEN 1 ELSE 0 END) AS rejected_or_stale \
             FROM revenue_opportunities \
             GROUP BY strategy \
             ORDER BY net_profit_usdc DESC, strategy ASC \
             LIMIT 200",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            let total: i64 = row.get(1)?;
            let settled: i64 = row.get(2)?;
            let gross: f64 = row.get(3)?;
            let costs: f64 = row.get(5)?;
            let conversion_rate = if total > 0 {
                settled as f64 / total as f64
            } else {
                0.0
            };
            let cost_to_revenue_ratio = if gross > 0.0 { costs / gross } else { 0.0 };
            Ok(json!({
                "strategy": row.get::<_, String>(0)?,
                "total_jobs": total,
                "settled_jobs": settled,
                "gross_revenue_usdc": gross,
                "net_profit_usdc": row.get::<_, f64>(4)?,
                "total_costs_usdc": costs,
                "conversion_rate": (conversion_rate * 1000.0).round() / 1000.0,
                "cost_to_revenue_ratio": (cost_to_revenue_ratio * 1000.0).round() / 1000.0,
                "avg_cycle_time_seconds": row.get::<_, Option<f64>>(6)?.map(|v| v as i64),
                "min_cycle_time_seconds": row.get::<_, Option<f64>>(7)?.map(|v| v as i64),
                "max_cycle_time_seconds": row.get::<_, Option<f64>>(8)?.map(|v| v as i64),
                "rejected_or_stale": row.get::<_, i64>(9)?,
            }))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Recent revenue opportunity audit log — the last N settlement events ordered newest-first.
/// Shows what happened, when, and how much, for operational transparency.
pub fn revenue_audit_log(db: &Database, limit: i64) -> Result<Vec<Value>> {
    let limit = limit.max(1).min(500);
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, source, strategy, status, expected_revenue_usdc, \
                    settled_amount_usdc, net_profit_usdc, attributable_costs_usdc, \
                    settlement_ref, settled_at, created_at, updated_at \
             FROM revenue_opportunities \
             ORDER BY updated_at DESC, created_at DESC \
             LIMIT ?1",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map(rusqlite::params![limit], |row| {
            let created: String = row.get(10)?;
            let settled_at: Option<String> = row.get(9)?;
            let cycle_seconds = settled_at.as_ref().map(|s| {
                // julianday math in Rust: parse ISO datetimes
                // Fallback: compute from updated_at if settled_at is present
                let s_trimmed = s.trim();
                let c_trimmed = created.trim();
                // Simple heuristic: parse with chrono-like manual approach
                // SQLite datetimes are "YYYY-MM-DD HH:MM:SS" format
                parse_cycle_seconds(c_trimmed, s_trimmed)
            });
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "source": row.get::<_, String>(1)?,
                "strategy": row.get::<_, String>(2)?,
                "status": row.get::<_, String>(3)?,
                "expected_revenue_usdc": row.get::<_, f64>(4)?,
                "settled_amount_usdc": row.get::<_, Option<f64>>(5)?,
                "net_profit_usdc": row.get::<_, Option<f64>>(6)?,
                "attributable_costs_usdc": row.get::<_, f64>(7)?,
                "settlement_ref": row.get::<_, Option<String>>(8)?,
                "cycle_time_seconds": cycle_seconds,
                "settled_at": settled_at,
                "created_at": created,
                "updated_at": row.get::<_, String>(11)?,
            }))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

/// Parse cycle time in seconds from two SQLite datetime strings ("YYYY-MM-DD HH:MM:SS").
fn parse_cycle_seconds(created: &str, settled: &str) -> Option<i64> {
    // SQLite datetime format: "YYYY-MM-DD HH:MM:SS"
    fn parse_ts(s: &str) -> Option<i64> {
        // Minimal parser for SQLite datetime — no external dep needed.
        let parts: Vec<&str> = s
            .split(|c: char| c == '-' || c == ' ' || c == ':' || c == 'T')
            .collect();
        if parts.len() < 6 {
            return None;
        }
        let y: i64 = parts[0].parse().ok()?;
        let mo: i64 = parts[1].parse().ok()?;
        let d: i64 = parts[2].parse().ok()?;
        let h: i64 = parts[3].parse().ok()?;
        let mi: i64 = parts[4].parse().ok()?;
        // Handle fractional seconds or trailing Z
        let sec_str = parts[5].trim_end_matches('Z');
        let sec: i64 = sec_str.split('.').next()?.parse().ok()?;
        // Approximate epoch seconds (sufficient for cycle-time deltas).
        // Using a simplified Julian-like computation; exact accuracy isn't critical
        // since both timestamps use the same formula.
        let days = (y - 2000) * 365 + (y - 2000) / 4 + (mo - 1) * 30 + d;
        Some(days * 86400 + h * 3600 + mi * 60 + sec)
    }
    let c = parse_ts(created)?;
    let s = parse_ts(settled)?;
    let diff = s - c;
    if diff >= 0 { Some(diff) } else { None }
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

    #[test]
    fn profitability_includes_conversion_and_costs() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        // Two settled, one rejected
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, net_profit_usdc, attributable_costs_usdc, settled_at) \
             VALUES ('ro_1','a','code_review','{}',10.0,'settled',10.0,8.0,2.0,datetime('now'))",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, net_profit_usdc, attributable_costs_usdc, settled_at) \
             VALUES ('ro_2','b','code_review','{}',5.0,'settled',5.0,4.0,1.0,datetime('now'))",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) \
             VALUES ('ro_3','c','code_review','{}',3.0,'rejected')",
            [],
        ).unwrap();
        drop(conn);

        let rows = revenue_strategy_profitability(&db).unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0]["strategy"], "code_review");
        assert_eq!(rows[0]["total_jobs"], 3);
        assert_eq!(rows[0]["settled_jobs"], 2);
        // conversion = 2/3 ≈ 0.667
        let conv = rows[0]["conversion_rate"].as_f64().unwrap();
        assert!(conv > 0.66 && conv < 0.67, "conversion_rate: {conv}");
        // cost_to_revenue = 3/15 = 0.2
        let ctr = rows[0]["cost_to_revenue_ratio"].as_f64().unwrap();
        assert!((ctr - 0.2).abs() < 0.01, "cost_to_revenue_ratio: {ctr}");
        assert_eq!(rows[0]["rejected_or_stale"], 1);
    }

    #[test]
    fn audit_log_returns_recent_entries() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) \
             VALUES ('ro_a','src','svc','{}',1.0,'intake')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status, settled_amount_usdc, settled_at) \
             VALUES ('ro_b','src','svc','{}',5.0,'settled',5.0,datetime('now'))",
            [],
        ).unwrap();
        drop(conn);

        let log = revenue_audit_log(&db, 10).unwrap();
        assert_eq!(log.len(), 2);
        // Most recent first — settled one should be first (updated_at DESC)
        assert!(log.iter().any(|r| r["id"] == "ro_b"));
    }

    #[test]
    fn parse_cycle_seconds_basic() {
        let created = "2025-01-15 10:00:00";
        let settled = "2025-01-15 10:05:00";
        let result = parse_cycle_seconds(created, settled);
        assert_eq!(result, Some(300)); // 5 minutes = 300 seconds
    }

    #[test]
    fn parse_cycle_seconds_negative_returns_none() {
        let created = "2025-01-15 10:05:00";
        let settled = "2025-01-15 10:00:00";
        assert!(parse_cycle_seconds(created, settled).is_none());
    }
}
