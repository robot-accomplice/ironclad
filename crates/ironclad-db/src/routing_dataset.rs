//! Historical routing dataset extraction pipeline.
//!
//! Joins `model_selection_events` with `inference_costs` via `turn_id` to produce
//! flat, exportable rows that capture both the routing decision and its cost
//! outcome. This is the training-data foundation for the shadow ML pipeline
//! (Phase 2a) and the offline evaluation harness.

use crate::Database;
use ironclad_core::{IroncladError, Result};

/// A single row in the routing dataset — one routing decision joined with its
/// aggregated inference cost outcome.
#[derive(Debug, Clone)]
pub struct RoutingDatasetRow {
    // ── routing decision (from model_selection_events) ──
    pub event_id: String,
    pub turn_id: String,
    pub session_id: String,
    pub agent_id: String,
    pub channel: String,
    pub selected_model: String,
    pub strategy: String,
    pub primary_model: String,
    pub override_model: Option<String>,
    pub complexity: Option<String>,
    pub user_excerpt: String,
    pub candidates_json: String,
    pub attribution: Option<String>,
    pub metascore_json: Option<String>,
    pub features_json: Option<String>,
    pub schema_version: i64,
    pub decision_at: String,

    // ── cost outcome (aggregated from inference_costs) ──
    pub total_tokens_in: i64,
    pub total_tokens_out: i64,
    pub total_cost: f64,
    pub inference_count: i64,
    pub any_cached: bool,
    pub avg_latency_ms: Option<f64>,
    pub avg_quality_score: Option<f64>,
    pub any_escalation: bool,
}

/// Filter parameters for dataset extraction.
#[derive(Debug, Clone, Default)]
pub struct DatasetFilter {
    /// Only include rows with `created_at >= since`.
    pub since: Option<String>,
    /// Only include rows with `created_at < until`.
    pub until: Option<String>,
    /// Only include rows at this schema version.
    pub schema_version: Option<i64>,
    /// Maximum rows to return (default: 10_000).
    pub limit: Option<usize>,
}

/// Extract the routing dataset by joining routing decisions with cost outcomes.
///
/// Each row represents one routing decision with aggregated cost metrics from
/// all inference calls made during that turn. Decisions with no matching
/// inference costs are excluded (INNER JOIN) since they provide no cost signal.
pub fn extract_routing_dataset(
    db: &Database,
    filter: &DatasetFilter,
) -> Result<Vec<RoutingDatasetRow>> {
    let conn = db.conn();

    // Build WHERE clause dynamically
    let mut conditions = Vec::new();
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut idx = 1;

    if let Some(ref since) = filter.since {
        conditions.push(format!("mse.created_at >= ?{idx}"));
        params.push(Box::new(since.clone()));
        idx += 1;
    }
    if let Some(ref until) = filter.until {
        conditions.push(format!("mse.created_at < ?{idx}"));
        params.push(Box::new(until.clone()));
        idx += 1;
    }
    if let Some(sv) = filter.schema_version {
        conditions.push(format!("mse.schema_version = ?{idx}"));
        params.push(Box::new(sv));
        idx += 1;
    }

    let limit = filter.limit.unwrap_or(10_000) as i64;
    let limit_placeholder = format!("?{idx}");
    params.push(Box::new(limit));

    let where_clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    let sql = format!(
        "SELECT
            mse.id,
            mse.turn_id,
            mse.session_id,
            mse.agent_id,
            mse.channel,
            mse.selected_model,
            mse.strategy,
            mse.primary_model,
            mse.override_model,
            mse.complexity,
            mse.user_excerpt,
            mse.candidates_json,
            mse.attribution,
            mse.metascore_json,
            mse.features_json,
            mse.schema_version,
            mse.created_at,
            COALESCE(SUM(ic.tokens_in), 0)  AS total_tokens_in,
            COALESCE(SUM(ic.tokens_out), 0) AS total_tokens_out,
            COALESCE(SUM(ic.cost), 0.0)     AS total_cost,
            COUNT(ic.id)                     AS inference_count,
            MAX(ic.cached)                   AS any_cached,
            AVG(ic.latency_ms)              AS avg_latency_ms,
            AVG(ic.quality_score)           AS avg_quality_score,
            MAX(ic.escalation)              AS any_escalation
         FROM model_selection_events mse
         INNER JOIN inference_costs ic ON ic.turn_id = mse.turn_id
         {where_clause}
         GROUP BY mse.id
         ORDER BY mse.created_at ASC
         LIMIT {limit_placeholder}"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| IroncladError::Database(format!("prepare routing dataset: {e}")))?;

    let rows = stmt
        .query_map(rusqlite::params_from_iter(params.iter()), |r| {
            Ok(RoutingDatasetRow {
                event_id: r.get(0)?,
                turn_id: r.get(1)?,
                session_id: r.get(2)?,
                agent_id: r.get(3)?,
                channel: r.get(4)?,
                selected_model: r.get(5)?,
                strategy: r.get(6)?,
                primary_model: r.get(7)?,
                override_model: r.get(8)?,
                complexity: r.get(9)?,
                user_excerpt: r.get(10)?,
                candidates_json: r.get(11)?,
                attribution: r.get(12)?,
                metascore_json: r.get(13)?,
                features_json: r.get(14)?,
                schema_version: r.get(15)?,
                decision_at: r.get(16)?,
                total_tokens_in: r.get(17)?,
                total_tokens_out: r.get(18)?,
                total_cost: r.get(19)?,
                inference_count: r.get(20)?,
                any_cached: r.get::<_, i32>(21)? != 0,
                avg_latency_ms: r.get(22)?,
                avg_quality_score: r.get(23)?,
                any_escalation: r.get::<_, i32>(24).unwrap_or(0) != 0,
            })
        })
        .map_err(|e| IroncladError::Database(format!("query routing dataset: {e}")))?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| IroncladError::Database(format!("collect routing dataset: {e}")))?;

    Ok(rows)
}

/// Summary statistics for the extracted dataset.
#[derive(Debug, Clone)]
pub struct DatasetSummary {
    pub total_rows: usize,
    pub distinct_models: usize,
    pub distinct_strategies: usize,
    pub total_cost: f64,
    pub avg_cost_per_decision: f64,
    pub schema_versions: Vec<i64>,
}

/// Compute summary statistics for the routing dataset.
pub fn dataset_summary(db: &Database, filter: &DatasetFilter) -> Result<DatasetSummary> {
    let mut summary_filter = filter.clone();
    // Summary stats should represent the full filtered dataset, not a pagination cap.
    summary_filter.limit = None;
    let rows = extract_routing_dataset(db, &summary_filter)?;
    if rows.is_empty() {
        return Ok(DatasetSummary {
            total_rows: 0,
            distinct_models: 0,
            distinct_strategies: 0,
            total_cost: 0.0,
            avg_cost_per_decision: 0.0,
            schema_versions: vec![],
        });
    }

    use std::collections::HashSet;
    let models: HashSet<&str> = rows.iter().map(|r| r.selected_model.as_str()).collect();
    let strategies: HashSet<&str> = rows.iter().map(|r| r.strategy.as_str()).collect();
    let total_cost: f64 = rows.iter().map(|r| r.total_cost).sum();
    let svs: HashSet<i64> = rows.iter().map(|r| r.schema_version).collect();
    let mut sv_vec: Vec<i64> = svs.into_iter().collect();
    sv_vec.sort();

    Ok(DatasetSummary {
        total_rows: rows.len(),
        distinct_models: models.len(),
        distinct_strategies: strategies.len(),
        total_cost,
        avg_cost_per_decision: total_cost / rows.len() as f64,
        schema_versions: sv_vec,
    })
}

/// Export the dataset as tab-separated values (header + rows).
///
/// TSV chosen over CSV because user_excerpt may contain commas.
pub fn extract_routing_dataset_tsv(db: &Database, filter: &DatasetFilter) -> Result<String> {
    let rows = extract_routing_dataset(db, filter)?;
    let mut out = String::with_capacity(rows.len() * 256);

    // Header
    out.push_str(
        "event_id\tturn_id\tsession_id\tagent_id\tchannel\tselected_model\tstrategy\t\
                   primary_model\toverride_model\tcomplexity\tattribution\tschema_version\t\
                   decision_at\ttotal_tokens_in\ttotal_tokens_out\ttotal_cost\tinference_count\t\
                   any_cached\tavg_latency_ms\tavg_quality_score\tany_escalation\n",
    );

    for r in &rows {
        out.push_str(&format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{:.6}\t{}\t{}\t{}\t{}\t{}\n",
            r.event_id,
            r.turn_id,
            r.session_id,
            r.agent_id,
            r.channel,
            r.selected_model,
            r.strategy,
            r.primary_model,
            r.override_model.as_deref().unwrap_or(""),
            r.complexity.as_deref().unwrap_or(""),
            r.attribution.as_deref().unwrap_or(""),
            r.schema_version,
            r.decision_at,
            r.total_tokens_in,
            r.total_tokens_out,
            r.total_cost,
            r.inference_count,
            r.any_cached as i32,
            r.avg_latency_ms.map_or("".to_string(), |v| format!("{v:.1}")),
            r.avg_quality_score.map_or("".to_string(), |v| format!("{v:.4}")),
            r.any_escalation as i32,
        ));
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::record_inference_cost;
    use crate::model_selection::{
        ModelSelectionEventRow, ROUTING_SCHEMA_VERSION, record_model_selection_event,
    };

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn insert_decision(
        db: &Database,
        event_id: &str,
        turn_id: &str,
        model: &str,
        attribution: Option<&str>,
        created_at: &str,
    ) {
        let evt = ModelSelectionEventRow {
            id: event_id.to_string(),
            turn_id: turn_id.to_string(),
            session_id: "sess-ds".to_string(),
            agent_id: "agent-ds".to_string(),
            channel: "cli".to_string(),
            selected_model: model.to_string(),
            strategy: "complexity".to_string(),
            primary_model: model.to_string(),
            override_model: None,
            complexity: Some("medium".to_string()),
            user_excerpt: "test query".to_string(),
            candidates_json: format!(r#"["{model}"]"#),
            created_at: created_at.to_string(),
            schema_version: ROUTING_SCHEMA_VERSION,
            attribution: attribution.map(|s| s.to_string()),
            metascore_json: None,
            features_json: None,
        };
        record_model_selection_event(db, &evt).unwrap();
    }

    fn insert_cost(
        db: &Database,
        turn_id: &str,
        model: &str,
        tokens_in: i64,
        tokens_out: i64,
        cost: f64,
    ) {
        record_inference_cost(
            db,
            model,
            "test-provider",
            tokens_in,
            tokens_out,
            cost,
            None,
            false,
            Some(100),
            Some(0.85),
            false,
            Some(turn_id),
        )
        .unwrap();
    }

    #[test]
    fn empty_dataset() {
        let db = test_db();
        let rows = extract_routing_dataset(&db, &DatasetFilter::default()).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn decision_without_cost_excluded() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-1",
            "turn-1",
            "claude-4",
            Some("metascore"),
            "2025-06-01T00:00:00",
        );
        // No inference_cost for turn-1
        let rows = extract_routing_dataset(&db, &DatasetFilter::default()).unwrap();
        assert!(
            rows.is_empty(),
            "decisions with no cost should be excluded (INNER JOIN)"
        );
    }

    #[test]
    fn basic_join() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-1",
            "turn-1",
            "claude-4",
            Some("metascore"),
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-1", "claude-4", 1000, 500, 0.03);

        let rows = extract_routing_dataset(&db, &DatasetFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.event_id, "evt-1");
        assert_eq!(r.selected_model, "claude-4");
        assert_eq!(r.total_tokens_in, 1000);
        assert_eq!(r.total_tokens_out, 500);
        assert!((r.total_cost - 0.03).abs() < 1e-9);
        assert_eq!(r.inference_count, 1);
        assert!(!r.any_cached);
        assert!(r.avg_latency_ms.is_some());
        assert!(r.avg_quality_score.is_some());
    }

    #[test]
    fn multiple_costs_per_turn_aggregate() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-agg",
            "turn-agg",
            "claude-4",
            None,
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-agg", "claude-4", 500, 200, 0.01);
        insert_cost(&db, "turn-agg", "claude-4", 300, 100, 0.005);

        let rows = extract_routing_dataset(&db, &DatasetFilter::default()).unwrap();
        assert_eq!(rows.len(), 1);
        let r = &rows[0];
        assert_eq!(r.total_tokens_in, 800);
        assert_eq!(r.total_tokens_out, 300);
        assert!((r.total_cost - 0.015).abs() < 1e-9);
        assert_eq!(r.inference_count, 2);
    }

    #[test]
    fn filter_since() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-old",
            "turn-old",
            "gpt-4",
            None,
            "2024-01-01T00:00:00",
        );
        insert_cost(&db, "turn-old", "gpt-4", 100, 50, 0.01);
        insert_decision(
            &db,
            "evt-new",
            "turn-new",
            "claude-4",
            None,
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-new", "claude-4", 200, 100, 0.02);

        let rows = extract_routing_dataset(
            &db,
            &DatasetFilter {
                since: Some("2025-01-01T00:00:00".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, "evt-new");
    }

    #[test]
    fn filter_until() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-old",
            "turn-old",
            "gpt-4",
            None,
            "2024-01-01T00:00:00",
        );
        insert_cost(&db, "turn-old", "gpt-4", 100, 50, 0.01);
        insert_decision(
            &db,
            "evt-new",
            "turn-new",
            "claude-4",
            None,
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-new", "claude-4", 200, 100, 0.02);

        let rows = extract_routing_dataset(
            &db,
            &DatasetFilter {
                until: Some("2025-01-01T00:00:00".to_string()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].event_id, "evt-old");
    }

    #[test]
    fn filter_schema_version() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-v1",
            "turn-v1",
            "claude-4",
            None,
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-v1", "claude-4", 100, 50, 0.01);

        // Filter for a non-existent schema version
        let rows = extract_routing_dataset(
            &db,
            &DatasetFilter {
                schema_version: Some(99),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(rows.is_empty());

        // Filter for the actual schema version
        let rows = extract_routing_dataset(
            &db,
            &DatasetFilter {
                schema_version: Some(ROUTING_SCHEMA_VERSION),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn filter_limit() {
        let db = test_db();
        for i in 0..5 {
            let eid = format!("evt-lim-{i}");
            let tid = format!("turn-lim-{i}");
            insert_decision(
                &db,
                &eid,
                &tid,
                "claude-4",
                None,
                &format!("2025-06-0{i}T00:00:00"),
            );
            insert_cost(&db, &tid, "claude-4", 100, 50, 0.01);
        }

        let rows = extract_routing_dataset(
            &db,
            &DatasetFilter {
                limit: Some(2),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn dataset_summary_empty() {
        let db = test_db();
        let s = dataset_summary(&db, &DatasetFilter::default()).unwrap();
        assert_eq!(s.total_rows, 0);
        assert_eq!(s.distinct_models, 0);
    }

    #[test]
    fn dataset_summary_populated() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-s1",
            "turn-s1",
            "claude-4",
            Some("metascore"),
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-s1", "claude-4", 1000, 500, 0.03);
        insert_decision(
            &db,
            "evt-s2",
            "turn-s2",
            "gpt-4",
            Some("fallback"),
            "2025-06-02T00:00:00",
        );
        insert_cost(&db, "turn-s2", "gpt-4", 500, 200, 0.01);

        let s = dataset_summary(&db, &DatasetFilter::default()).unwrap();
        assert_eq!(s.total_rows, 2);
        assert_eq!(s.distinct_models, 2);
        assert_eq!(s.distinct_strategies, 1); // both "complexity"
        assert!((s.total_cost - 0.04).abs() < 1e-9);
        assert!((s.avg_cost_per_decision - 0.02).abs() < 1e-9);
        assert_eq!(s.schema_versions, vec![ROUTING_SCHEMA_VERSION]);
    }

    #[test]
    fn dataset_summary_ignores_limit_cap() {
        let db = test_db();
        for i in 0..3 {
            let eid = format!("evt-sum-{i}");
            let tid = format!("turn-sum-{i}");
            insert_decision(
                &db,
                &eid,
                &tid,
                "claude-4",
                Some("metascore"),
                &format!("2025-06-0{}T00:00:00", i + 1),
            );
            insert_cost(&db, &tid, "claude-4", 100, 50, 0.01);
        }
        let s = dataset_summary(
            &db,
            &DatasetFilter {
                limit: Some(1),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(s.total_rows, 3);
    }

    #[test]
    fn tsv_export_header_and_rows() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-tsv",
            "turn-tsv",
            "claude-4",
            Some("primary_usable"),
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-tsv", "claude-4", 100, 50, 0.005);

        let tsv = extract_routing_dataset_tsv(&db, &DatasetFilter::default()).unwrap();
        let lines: Vec<&str> = tsv.lines().collect();
        assert!(lines.len() >= 2, "should have header + at least 1 row");
        assert!(lines[0].starts_with("event_id\t"));
        assert!(lines[1].starts_with("evt-tsv\t"));
        assert!(lines[1].contains("primary_usable"));
    }

    #[test]
    fn ordering_is_ascending() {
        let db = test_db();
        insert_decision(
            &db,
            "evt-asc-2",
            "turn-asc-2",
            "claude-4",
            None,
            "2025-06-02T00:00:00",
        );
        insert_cost(&db, "turn-asc-2", "claude-4", 100, 50, 0.01);
        insert_decision(
            &db,
            "evt-asc-1",
            "turn-asc-1",
            "claude-4",
            None,
            "2025-06-01T00:00:00",
        );
        insert_cost(&db, "turn-asc-1", "claude-4", 100, 50, 0.01);

        let rows = extract_routing_dataset(&db, &DatasetFilter::default()).unwrap();
        assert_eq!(
            rows[0].event_id, "evt-asc-1",
            "oldest first (ASC for training data)"
        );
        assert_eq!(rows[1].event_id, "evt-asc-2");
    }
}
