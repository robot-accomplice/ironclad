use crate::Database;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};

pub fn record_revenue_feedback(
    db: &Database,
    opportunity_id: &str,
    grade: f64,
    source: &str,
    comment: Option<&str>,
) -> Result<String> {
    let conn = db.conn();
    let strategy: String = conn
        .query_row(
            "SELECT strategy FROM revenue_opportunities WHERE id = ?1",
            [opportunity_id],
            |row| row.get(0),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;
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
             ORDER BY AVG(grade) DESC, COUNT(*) DESC, strategy ASC",
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

        record_revenue_feedback(&db, "ro_1", 4.5, "operator", Some("strong result")).unwrap();
        record_revenue_feedback(&db, "ro_2", 3.5, "operator", None).unwrap();
        record_revenue_feedback(&db, "ro_3", 2.0, "operator", None).unwrap();

        let rows = revenue_feedback_summary_by_strategy(&db).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0]["strategy"], "oracle_feed");
        assert_eq!(rows[0]["feedback_count"], 2);
    }
}
