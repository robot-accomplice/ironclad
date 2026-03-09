use crate::Database;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};

#[derive(Debug, Clone, Copy)]
pub struct RevenueFeedbackSignal {
    pub feedback_count: i64,
    pub avg_grade: f64,
}

pub fn record_revenue_feedback(
    db: &Database,
    opportunity_id: &str,
    strategy: &str,
    grade: f64,
    source: &str,
    comment: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO revenue_feedback (id, opportunity_id, strategy, grade, source, comment) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![id, opportunity_id, strategy, grade, source, comment],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    conn.execute(
        "UPDATE revenue_opportunities SET updated_at = datetime('now') WHERE id = ?1",
        [opportunity_id],
    )
    .map_err(|e| IroncladError::Database(e.to_string()))?;
    Ok(id)
}

pub fn revenue_feedback_summary_by_strategy(db: &Database) -> Result<Vec<Value>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT strategy, COUNT(*), AVG(grade), MAX(created_at) \
             FROM revenue_feedback \
             GROUP BY strategy \
             ORDER BY AVG(grade) DESC, COUNT(*) DESC, strategy ASC \
             LIMIT 200",
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "strategy": row.get::<_, String>(0)?,
                "feedback_count": row.get::<_, i64>(1)?,
                "avg_grade": row.get::<_, f64>(2)?,
                "latest_feedback_at": row.get::<_, String>(3)?,
            }))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;
    rows.collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(e.to_string()))
}

pub fn revenue_feedback_signal_for_strategy(
    db: &Database,
    strategy: &str,
) -> Result<Option<RevenueFeedbackSignal>> {
    let conn = db.conn();
    conn.query_row(
        "SELECT COUNT(*), AVG(grade) \
         FROM revenue_feedback \
         WHERE strategy = ?1",
        [strategy],
        |row| {
            let feedback_count = row.get::<_, i64>(0)?;
            let avg_grade = row.get::<_, Option<f64>>(1)?;
            Ok(avg_grade.map(|avg_grade| RevenueFeedbackSignal {
                feedback_count,
                avg_grade,
            }))
        },
    )
    .map_err(|e| IroncladError::Database(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn revenue_feedback_summary_groups_by_strategy() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_1','a','oracle_feed','{}',5.0,'settled')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_2','b','oracle_feed','{}',4.0,'settled')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_3','c','micro_bounty','{}',3.0,'settled')",
            [],
        )
        .unwrap();
        drop(conn);

        record_revenue_feedback(
            &db,
            "ro_1",
            "oracle_feed",
            4.5,
            "operator",
            Some("strong result"),
        )
        .unwrap();
        record_revenue_feedback(&db, "ro_2", "oracle_feed", 3.5, "operator", None).unwrap();
        record_revenue_feedback(&db, "ro_3", "micro_bounty", 2.0, "operator", None).unwrap();

        let rows = revenue_feedback_summary_by_strategy(&db).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["strategy"], "oracle_feed");
        assert_eq!(rows[0]["feedback_count"], 2);
    }

    #[test]
    fn record_feedback_without_existing_opportunity_succeeds_at_db_layer() {
        // Existence check is the route handler's responsibility, not the DB layer.
        // The DB function is a pure insert + touch; it does not validate FK presence.
        let db = Database::new(":memory:").unwrap();
        let result =
            record_revenue_feedback(&db, "nonexistent", "any_strategy", 4.0, "operator", None);
        assert!(result.is_ok());
    }

    #[test]
    fn revenue_feedback_signal_returns_count_and_average() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_1','a','oracle_feed','{}',5.0,'settled')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO revenue_opportunities (id, source, strategy, payload_json, expected_revenue_usdc, status) VALUES ('ro_2','b','oracle_feed','{}',4.0,'settled')",
            [],
        )
        .unwrap();
        drop(conn);

        record_revenue_feedback(&db, "ro_1", "oracle_feed", 5.0, "operator", None).unwrap();
        record_revenue_feedback(&db, "ro_2", "oracle_feed", 3.0, "operator", None).unwrap();

        let signal = revenue_feedback_signal_for_strategy(&db, "oracle_feed")
            .unwrap()
            .unwrap();
        assert_eq!(signal.feedback_count, 2);
        assert!((signal.avg_grade - 4.0).abs() < f64::EPSILON);
    }
}
