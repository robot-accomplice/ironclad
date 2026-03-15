//! Shadow routing predictions — counterfactual ML recommendations stored
//! alongside production decisions for offline validation.
//!
//! The shadow pipeline records what a candidate ML model *would* have chosen
//! without affecting live routing. Agreement rate and regret analysis run
//! against this data to decide when (if ever) to promote the ML model.

use ironclad_core::Result;

use crate::{Database, DbResultExt};

/// A single shadow routing prediction row.
#[derive(Debug, Clone)]
pub struct ShadowPredictionRow {
    pub id: String,
    pub turn_id: String,
    /// The model that production routing actually selected.
    pub production_model: String,
    /// The model the shadow recommender would have selected (None if shadow abstained).
    pub shadow_model: Option<String>,
    /// Complexity estimate used by production routing.
    pub production_complexity: Option<f64>,
    /// Complexity estimate from the shadow model (may differ).
    pub shadow_complexity: Option<f64>,
    /// 1 if production and shadow agree, 0 otherwise.
    pub agreed: bool,
    /// Arbitrary JSON detail blob (scores, feature weights, etc.).
    pub detail_json: Option<String>,
    pub created_at: String,
}

/// Insert a shadow prediction record.
pub fn record_shadow_prediction(db: &Database, row: &ShadowPredictionRow) -> Result<()> {
    let conn = db.conn();
    conn.execute(
        "INSERT INTO shadow_routing_predictions
         (id, turn_id, production_model, shadow_model, production_complexity,
          shadow_complexity, agreed, detail_json, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        rusqlite::params![
            row.id,
            row.turn_id,
            row.production_model,
            row.shadow_model,
            row.production_complexity,
            row.shadow_complexity,
            row.agreed as i32,
            row.detail_json,
            row.created_at,
        ],
    )
    .db_err()?;
    Ok(())
}

/// Summary statistics for shadow prediction agreement.
#[derive(Debug, Clone)]
pub struct ShadowAgreementSummary {
    pub total: usize,
    pub agreed: usize,
    pub disagreed: usize,
    /// Agreement rate [0.0, 1.0], or None if no predictions.
    pub agreement_rate: Option<f64>,
}

/// Compute agreement summary for shadow predictions, optionally filtered by
/// a time window (`since` in ISO-8601 format).
pub fn shadow_agreement_summary(
    db: &Database,
    since: Option<&str>,
) -> Result<ShadowAgreementSummary> {
    let conn = db.conn();
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
        (
            "SELECT
                COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN agreed = 1 THEN 1 ELSE 0 END), 0) AS agreed
             FROM shadow_routing_predictions
             WHERE created_at >= ?1",
            vec![Box::new(s.to_string())],
        )
    } else {
        (
            "SELECT
                COUNT(*) AS total,
                COALESCE(SUM(CASE WHEN agreed = 1 THEN 1 ELSE 0 END), 0) AS agreed
             FROM shadow_routing_predictions",
            vec![],
        )
    };

    let (total, agreed): (usize, usize) = conn
        .query_row(sql, rusqlite::params_from_iter(params.iter()), |r| {
            Ok((r.get::<_, usize>(0)?, r.get::<_, usize>(1)?))
        })
        .db_err()?;

    let disagreed = total.saturating_sub(agreed);
    let agreement_rate = if total > 0 {
        Some(agreed as f64 / total as f64)
    } else {
        None
    };

    Ok(ShadowAgreementSummary {
        total,
        agreed,
        disagreed,
        agreement_rate,
    })
}

/// Fetch the N most recent shadow predictions (newest first).
pub fn recent_shadow_predictions(db: &Database, limit: usize) -> Result<Vec<ShadowPredictionRow>> {
    let conn = db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, turn_id, production_model, shadow_model,
                    production_complexity, shadow_complexity, agreed,
                    detail_json, created_at
             FROM shadow_routing_predictions
             ORDER BY created_at DESC
             LIMIT ?1",
        )
        .db_err()?;

    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |r| {
            Ok(ShadowPredictionRow {
                id: r.get(0)?,
                turn_id: r.get(1)?,
                production_model: r.get(2)?,
                shadow_model: r.get(3)?,
                production_complexity: r.get(4)?,
                shadow_complexity: r.get(5)?,
                agreed: r.get::<_, i32>(6)? != 0,
                detail_json: r.get(7)?,
                created_at: r.get(8)?,
            })
        })
        .db_err()?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.db_err()?);
    }
    Ok(results)
}

/// Delete shadow routing predictions older than `retention_days` days.
///
/// Returns the number of rows deleted.
pub fn prune_shadow_predictions(db: &Database, retention_days: u32) -> Result<usize> {
    let conn = db.conn();
    let deleted = conn
        .execute(
            "DELETE FROM shadow_routing_predictions \
             WHERE created_at < datetime('now', ?1)",
            [format!("-{retention_days} days")],
        )
        .db_err()?;
    Ok(deleted)
}

/// Count disagreements where shadow would have picked a different model,
/// grouped by (production_model, shadow_model) pair. Useful for identifying
/// systematic divergence patterns.
pub fn disagreement_pairs(
    db: &Database,
    since: Option<&str>,
) -> Result<Vec<(String, String, usize)>> {
    let conn = db.conn();
    let (sql, params): (&str, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(s) = since {
        (
            "SELECT production_model, shadow_model, COUNT(*) AS cnt
             FROM shadow_routing_predictions
             WHERE agreed = 0 AND shadow_model IS NOT NULL AND created_at >= ?1
             GROUP BY production_model, shadow_model
             ORDER BY cnt DESC",
            vec![Box::new(s.to_string())],
        )
    } else {
        (
            "SELECT production_model, shadow_model, COUNT(*) AS cnt
             FROM shadow_routing_predictions
             WHERE agreed = 0 AND shadow_model IS NOT NULL
             GROUP BY production_model, shadow_model
             ORDER BY cnt DESC",
            vec![],
        )
    };

    let mut stmt = conn.prepare(sql).db_err()?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, usize>(2)?,
            ))
        })
        .db_err()?;

    let mut results = Vec::new();
    for row in rows {
        results.push(row.db_err()?);
    }
    Ok(results)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").expect("in-memory db")
    }

    fn make_row(
        id: &str,
        turn: &str,
        prod: &str,
        shadow: Option<&str>,
        agreed: bool,
    ) -> ShadowPredictionRow {
        ShadowPredictionRow {
            id: id.into(),
            turn_id: turn.into(),
            production_model: prod.into(),
            shadow_model: shadow.map(String::from),
            production_complexity: Some(0.5),
            shadow_complexity: Some(0.5),
            agreed,
            detail_json: None,
            created_at: "2025-01-15T10:00:00".into(),
        }
    }

    #[test]
    fn record_and_retrieve() {
        let db = test_db();
        let row = make_row(
            "sp-1",
            "t-1",
            "openai/gpt-4o",
            Some("ollama/qwen3:8b"),
            false,
        );
        record_shadow_prediction(&db, &row).unwrap();

        let recent = recent_shadow_predictions(&db, 10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].production_model, "openai/gpt-4o");
        assert_eq!(recent[0].shadow_model.as_deref(), Some("ollama/qwen3:8b"));
        assert!(!recent[0].agreed);
    }

    #[test]
    fn agreement_summary_empty() {
        let db = test_db();
        let summary = shadow_agreement_summary(&db, None).unwrap();
        assert_eq!(summary.total, 0);
        assert!(summary.agreement_rate.is_none());
    }

    #[test]
    fn agreement_summary_mixed() {
        let db = test_db();
        // 3 agreed, 2 disagreed
        for (i, agreed) in [true, true, false, true, false].iter().enumerate() {
            let row = make_row(
                &format!("sp-{i}"),
                &format!("t-{i}"),
                "openai/gpt-4o",
                Some("ollama/qwen3:8b"),
                *agreed,
            );
            record_shadow_prediction(&db, &row).unwrap();
        }

        let summary = shadow_agreement_summary(&db, None).unwrap();
        assert_eq!(summary.total, 5);
        assert_eq!(summary.agreed, 3);
        assert_eq!(summary.disagreed, 2);
        let rate = summary.agreement_rate.unwrap();
        assert!((rate - 0.6).abs() < 1e-9);
    }

    #[test]
    fn agreement_summary_with_since_filter() {
        let db = test_db();
        // Old prediction
        let mut old = make_row("sp-old", "t-old", "openai/gpt-4o", Some("local"), false);
        old.created_at = "2024-01-01T00:00:00".into();
        record_shadow_prediction(&db, &old).unwrap();

        // Recent prediction
        let recent_row = make_row("sp-new", "t-new", "openai/gpt-4o", Some("local"), true);
        record_shadow_prediction(&db, &recent_row).unwrap();

        let summary = shadow_agreement_summary(&db, Some("2025-01-01T00:00:00")).unwrap();
        assert_eq!(summary.total, 1);
        assert_eq!(summary.agreed, 1);
    }

    #[test]
    fn recent_predictions_ordering() {
        let db = test_db();
        let mut r1 = make_row("sp-1", "t-1", "m1", None, true);
        r1.created_at = "2025-01-15T10:00:00".into();
        let mut r2 = make_row("sp-2", "t-2", "m2", None, true);
        r2.created_at = "2025-01-15T11:00:00".into();
        record_shadow_prediction(&db, &r1).unwrap();
        record_shadow_prediction(&db, &r2).unwrap();

        let recent = recent_shadow_predictions(&db, 10).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].id, "sp-2"); // newer first
        assert_eq!(recent[1].id, "sp-1");
    }

    #[test]
    fn recent_predictions_limit() {
        let db = test_db();
        for i in 0..5 {
            let row = make_row(&format!("sp-{i}"), &format!("t-{i}"), "m", None, true);
            record_shadow_prediction(&db, &row).unwrap();
        }

        let recent = recent_shadow_predictions(&db, 2).unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn disagreement_pairs_basic() {
        let db = test_db();
        // 2 disagreements on same pair, 1 on different pair, 1 agreement (excluded)
        record_shadow_prediction(
            &db,
            &make_row("sp-1", "t-1", "gpt-4o", Some("qwen3:8b"), false),
        )
        .unwrap();
        record_shadow_prediction(
            &db,
            &make_row("sp-2", "t-2", "gpt-4o", Some("qwen3:8b"), false),
        )
        .unwrap();
        record_shadow_prediction(
            &db,
            &make_row("sp-3", "t-3", "gpt-4o", Some("claude-3"), false),
        )
        .unwrap();
        record_shadow_prediction(
            &db,
            &make_row("sp-4", "t-4", "gpt-4o", Some("qwen3:8b"), true),
        )
        .unwrap();

        let pairs = disagreement_pairs(&db, None).unwrap();
        assert_eq!(pairs.len(), 2);
        // Most frequent disagreement first
        assert_eq!(pairs[0], ("gpt-4o".into(), "qwen3:8b".into(), 2));
        assert_eq!(pairs[1], ("gpt-4o".into(), "claude-3".into(), 1));
    }

    #[test]
    fn shadow_model_none_excluded_from_disagreement_pairs() {
        let db = test_db();
        // Shadow abstained (None) — should NOT appear in disagreement pairs
        record_shadow_prediction(&db, &make_row("sp-1", "t-1", "gpt-4o", None, false)).unwrap();

        let pairs = disagreement_pairs(&db, None).unwrap();
        assert!(pairs.is_empty());
    }
}
