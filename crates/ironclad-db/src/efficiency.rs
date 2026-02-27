use std::collections::HashMap;

use serde::{Deserialize, Serialize};

use crate::Database;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryImpact {
    pub with_memory: f64,
    pub without_memory: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    pub avg_grade: f64,
    pub grade_count: i64,
    pub grade_coverage: f64,
    pub cost_per_quality_point: f64,
    pub by_complexity: HashMap<String, f64>,
    pub memory_impact: MemoryImpact,
    pub trend: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelEfficiency {
    pub model: String,
    pub total_turns: i64,
    pub avg_output_density: f64,
    pub avg_budget_utilization: f64,
    pub avg_memory_roi: f64,
    pub avg_system_prompt_weight: f64,
    pub cache_hit_rate: f64,
    pub context_pressure_rate: f64,
    pub cost: CostMetrics,
    pub trend: TrendMetrics,
    pub quality: Option<QualityMetrics>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostMetrics {
    pub total: f64,
    pub per_output_token: f64,
    pub effective_per_turn: f64,
    pub cache_savings: f64,
    pub cumulative_trend: String,
    pub attribution: CostAttribution,
    pub wasted_budget_cost: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CostAttribution {
    pub system_prompt: AttributionDetail,
    pub memories: AttributionDetail,
    pub history: AttributionDetail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttributionDetail {
    pub tokens: i64,
    pub cost: f64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendMetrics {
    pub output_density: String,
    pub cost_per_turn: String,
    pub cache_hit_rate: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeSeriesPoint {
    pub bucket: String,
    pub model: String,
    pub output_density: f64,
    pub cost: f64,
    pub turns: i64,
    pub budget_utilization: f64,
    pub cached_count: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyTotals {
    pub total_cost: f64,
    pub total_cache_savings: f64,
    pub total_turns: i64,
    pub most_expensive_model: Option<String>,
    pub most_efficient_model: Option<String>,
    pub biggest_cost_driver: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EfficiencyReport {
    pub period: String,
    pub models: HashMap<String, ModelEfficiency>,
    pub time_series: Vec<TimeSeriesPoint>,
    pub totals: EfficiencyTotals,
}

fn cutoff_expr(period: &str) -> &'static str {
    match period {
        "1h" => "datetime('now', '-1 hour')",
        "24h" => "datetime('now', '-1 day')",
        "7d" => "datetime('now', '-7 days')",
        "30d" => "datetime('now', '-30 days')",
        _ => "datetime('1970-01-01')",
    }
}

struct RawModelRow {
    model: String,
    total_turns: i64,
    avg_output_density: f64,
    total_cost: f64,
    total_tokens_out: i64,
    total_tokens_in: i64,
    cached_count: i64,
    avg_cost_per_turn: f64,
}

fn trend_label(first_half: f64, second_half: f64) -> String {
    if first_half == 0.0 && second_half == 0.0 {
        return "stable".into();
    }
    let delta = second_half - first_half;
    let base = first_half.max(0.001);
    let pct = delta / base;
    if pct > 0.05 {
        "increasing".into()
    } else if pct < -0.05 {
        "decreasing".into()
    } else {
        "stable".into()
    }
}

fn compute_quality_for_model(
    conn: &rusqlite::Connection,
    model: &str,
    cutoff: &str,
    total_turns: i64,
) -> Option<QualityMetrics> {
    let sql = format!(
        "SELECT tf.grade, t.model, t.cost \
         FROM turn_feedback tf \
         JOIN turns t ON t.id = tf.turn_id \
         WHERE t.model = ?1 AND tf.created_at >= {cutoff}"
    );
    let mut stmt = conn.prepare(&sql).ok()?;
    let rows: Vec<(i32, String, f64)> = stmt
        .query_map(rusqlite::params![model], |row| {
            Ok((
                row.get::<_, i32>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2).unwrap_or(0.0),
            ))
        })
        .ok()?
        .collect::<std::result::Result<Vec<_>, _>>()
        .ok()?;

    if rows.is_empty() {
        return None;
    }

    let grade_count = rows.len() as i64;
    let sum_grade: i64 = rows.iter().map(|(g, _, _)| *g as i64).sum();
    let avg_grade = sum_grade as f64 / grade_count as f64;
    let grade_coverage = if total_turns > 0 {
        grade_count as f64 / total_turns as f64
    } else {
        0.0
    };

    let total_cost: f64 = rows.iter().map(|(_, _, c)| c).sum();
    let total_quality: f64 = rows.iter().map(|(g, _, _)| *g as f64).sum();
    let cost_per_quality_point = if total_quality > 0.0 {
        total_cost / total_quality
    } else {
        0.0
    };

    let half = rows.len() / 2;
    let trend = if rows.len() >= 4 {
        let first_avg = rows[..half].iter().map(|(g, _, _)| *g as f64).sum::<f64>() / half as f64;
        let second_avg = rows[half..].iter().map(|(g, _, _)| *g as f64).sum::<f64>()
            / (rows.len() - half) as f64;
        trend_label(first_avg, second_avg)
    } else {
        "stable".into()
    };

    Some(QualityMetrics {
        avg_grade,
        grade_count,
        grade_coverage,
        cost_per_quality_point,
        by_complexity: HashMap::new(),
        memory_impact: MemoryImpact {
            with_memory: 0.0,
            without_memory: 0.0,
        },
        trend,
    })
}

pub fn compute_efficiency(
    db: &Database,
    period: &str,
    model_filter: Option<&str>,
) -> Result<EfficiencyReport> {
    let cutoff = cutoff_expr(period);
    let conn = db.conn();

    let model_clause = if model_filter.is_some() {
        " AND model = ?1"
    } else {
        ""
    };

    // ── Per-model aggregates ─────────────────────────────────
    let main_sql = format!(
        "SELECT \
            model, \
            COUNT(*) AS total_turns, \
            AVG(CAST(tokens_out AS REAL) / NULLIF(tokens_in, 0)) AS avg_output_density, \
            SUM(cost) AS total_cost, \
            SUM(tokens_out) AS total_tokens_out, \
            SUM(tokens_in) AS total_tokens_in, \
            SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS cached_count, \
            AVG(cost) AS avg_cost_per_turn \
         FROM inference_costs \
         WHERE created_at >= {cutoff}{model_clause} \
         GROUP BY model \
         ORDER BY total_cost DESC"
    );

    let mut stmt = conn
        .prepare(&main_sql)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let map_row = |row: &rusqlite::Row| -> rusqlite::Result<RawModelRow> {
        Ok(RawModelRow {
            model: row.get(0)?,
            total_turns: row.get(1)?,
            avg_output_density: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
            total_cost: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            total_tokens_out: row.get::<_, Option<i64>>(4)?.unwrap_or(0),
            total_tokens_in: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
            cached_count: row.get::<_, Option<i64>>(6)?.unwrap_or(0),
            avg_cost_per_turn: row.get::<_, Option<f64>>(7)?.unwrap_or(0.0),
        })
    };

    let rows: Vec<RawModelRow> = if let Some(mf) = model_filter {
        stmt.query_map(rusqlite::params![mf], map_row)
    } else {
        stmt.query_map([], map_row)
    }
    .map_err(|e| IroncladError::Database(e.to_string()))?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    // ── Time-series (daily buckets) ──────────────────────────
    let ts_sql = format!(
        "SELECT \
            strftime('%Y-%m-%d', created_at) AS bucket, \
            model, \
            AVG(CAST(tokens_out AS REAL) / NULLIF(tokens_in, 0)) AS output_density, \
            SUM(cost) AS cost, \
            COUNT(*) AS turns, \
            SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS cached_count \
         FROM inference_costs \
         WHERE created_at >= {cutoff}{model_clause} \
         GROUP BY bucket, model \
         ORDER BY bucket"
    );

    let mut ts_stmt = conn
        .prepare(&ts_sql)
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let ts_map = |row: &rusqlite::Row| -> rusqlite::Result<TimeSeriesPoint> {
        Ok(TimeSeriesPoint {
            bucket: row.get(0)?,
            model: row.get(1)?,
            output_density: row.get::<_, Option<f64>>(2)?.unwrap_or(0.0),
            cost: row.get::<_, Option<f64>>(3)?.unwrap_or(0.0),
            turns: row.get(4)?,
            budget_utilization: 0.0,
            cached_count: row.get::<_, Option<i64>>(5)?.unwrap_or(0),
        })
    };

    let time_series: Vec<TimeSeriesPoint> = if let Some(mf) = model_filter {
        ts_stmt.query_map(rusqlite::params![mf], ts_map)
    } else {
        ts_stmt.query_map([], ts_map)
    }
    .map_err(|e| IroncladError::Database(e.to_string()))?
    .collect::<std::result::Result<Vec<_>, _>>()
    .map_err(|e| IroncladError::Database(e.to_string()))?;

    // ── Build per-model trend data from time series ──────────
    let mut model_ts: HashMap<String, Vec<&TimeSeriesPoint>> = HashMap::new();
    for pt in &time_series {
        model_ts.entry(pt.model.clone()).or_default().push(pt);
    }

    // ── Assemble ModelEfficiency map ─────────────────────────
    let mut models: HashMap<String, ModelEfficiency> = HashMap::new();
    let mut grand_total_cost = 0.0_f64;
    let mut grand_total_turns = 0_i64;
    let mut most_expensive: Option<(String, f64)> = None;
    let mut most_efficient: Option<(String, f64)> = None;

    for r in &rows {
        let cache_hit_rate = if r.total_turns > 0 {
            r.cached_count as f64 / r.total_turns as f64
        } else {
            0.0
        };

        let per_output_token = if r.total_tokens_out > 0 {
            r.total_cost / r.total_tokens_out as f64
        } else {
            0.0
        };

        // Estimate cache savings: cached requests would have cost roughly the
        // average per-turn cost, so savings ≈ cached_count × avg_cost_per_turn × input_fraction.
        let input_fraction = if r.total_tokens_in + r.total_tokens_out > 0 {
            r.total_tokens_in as f64 / (r.total_tokens_in + r.total_tokens_out) as f64
        } else {
            0.5
        };
        let cache_savings = r.cached_count as f64 * r.avg_cost_per_turn * input_fraction;

        // Trends from time-series split
        let pts = model_ts.get(&r.model).cloned().unwrap_or_default();
        let trend = if pts.len() >= 2 {
            let mid = pts.len() / 2;
            let (first, second) = pts.split_at(mid);

            let avg = |slice: &[&TimeSeriesPoint], f: fn(&TimeSeriesPoint) -> f64| -> f64 {
                if slice.is_empty() {
                    return 0.0;
                }
                slice.iter().map(|p| f(p)).sum::<f64>() / slice.len() as f64
            };

            let first_density = avg(first, |p| p.output_density);
            let second_density = avg(second, |p| p.output_density);

            let first_cpt = avg(first, |p| {
                if p.turns > 0 {
                    p.cost / p.turns as f64
                } else {
                    0.0
                }
            });
            let second_cpt = avg(second, |p| {
                if p.turns > 0 {
                    p.cost / p.turns as f64
                } else {
                    0.0
                }
            });

            let first_cache = avg(first, |p| {
                if p.turns > 0 {
                    p.cached_count as f64 / p.turns as f64
                } else {
                    0.0
                }
            });
            let second_cache = avg(second, |p| {
                if p.turns > 0 {
                    p.cached_count as f64 / p.turns as f64
                } else {
                    0.0
                }
            });

            TrendMetrics {
                output_density: trend_label(first_density, second_density),
                cost_per_turn: trend_label(first_cpt, second_cpt),
                cache_hit_rate: trend_label(first_cache, second_cache),
            }
        } else {
            TrendMetrics {
                output_density: "stable".into(),
                cost_per_turn: "stable".into(),
                cache_hit_rate: "stable".into(),
            }
        };

        let cumulative_trend = trend.cost_per_turn.clone();

        // Without context_snapshots, attribute all input tokens to "history".
        let attribution = CostAttribution {
            system_prompt: AttributionDetail {
                tokens: 0,
                cost: 0.0,
                pct: 0.0,
            },
            memories: AttributionDetail {
                tokens: 0,
                cost: 0.0,
                pct: 0.0,
            },
            history: AttributionDetail {
                tokens: r.total_tokens_in,
                cost: r.total_cost * input_fraction,
                pct: 100.0,
            },
        };

        let quality = compute_quality_for_model(&conn, &r.model, cutoff, r.total_turns);

        let eff = ModelEfficiency {
            model: r.model.clone(),
            total_turns: r.total_turns,
            avg_output_density: r.avg_output_density,
            avg_budget_utilization: 0.0,
            avg_memory_roi: 0.0,
            avg_system_prompt_weight: 0.0,
            cache_hit_rate,
            context_pressure_rate: 0.0,
            cost: CostMetrics {
                total: r.total_cost,
                per_output_token,
                effective_per_turn: r.avg_cost_per_turn,
                cache_savings,
                cumulative_trend,
                attribution,
                wasted_budget_cost: 0.0,
            },
            trend,
            quality,
        };

        grand_total_cost += r.total_cost;
        grand_total_turns += r.total_turns;

        match &most_expensive {
            None => most_expensive = Some((r.model.clone(), r.total_cost)),
            Some((_, c)) if r.total_cost > *c => {
                most_expensive = Some((r.model.clone(), r.total_cost));
            }
            _ => {}
        }

        let density = r.avg_output_density;
        match &most_efficient {
            None => most_efficient = Some((r.model.clone(), density)),
            Some((_, d)) if density > *d => {
                most_efficient = Some((r.model.clone(), density));
            }
            _ => {}
        }

        models.insert(r.model.clone(), eff);
    }

    let total_cache_savings: f64 = models.values().map(|m| m.cost.cache_savings).sum();

    let biggest_cost_driver = most_expensive
        .as_ref()
        .map(|(m, _)| m.clone())
        .unwrap_or_else(|| "none".into());

    let totals = EfficiencyTotals {
        total_cost: grand_total_cost,
        total_cache_savings,
        total_turns: grand_total_turns,
        most_expensive_model: most_expensive.map(|(m, _)| m),
        most_efficient_model: most_efficient.map(|(m, _)| m),
        biggest_cost_driver,
    };

    Ok(EfficiencyReport {
        period: period.to_string(),
        models,
        time_series,
        totals,
    })
}

// ── UserProfile types for recommendations ────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationModelStats {
    pub turns: i64,
    pub avg_cost: f64,
    pub avg_quality: Option<f64>,
    pub cache_hit_rate: f64,
    pub avg_output_density: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecommendationUserProfile {
    pub total_sessions: i64,
    pub total_turns: i64,
    pub total_cost: f64,
    pub avg_quality: Option<f64>,
    pub grade_coverage: f64,
    pub models_used: Vec<String>,
    pub model_stats: HashMap<String, RecommendationModelStats>,
    pub avg_session_length: f64,
    pub avg_tokens_per_turn: f64,
    pub tool_success_rate: f64,
    pub cache_hit_rate: f64,
    pub memory_retrieval_rate: f64,
}

pub fn build_user_profile(db: &Database, period: &str) -> Result<RecommendationUserProfile> {
    let cutoff = cutoff_expr(period);
    let conn = db.conn();

    let (total_sessions, avg_session_length): (i64, f64) = conn
        .query_row(
            &format!(
                "SELECT COUNT(*), COALESCE(AVG(msg_count), 0) FROM (\
                   SELECT s.id, COUNT(m.id) AS msg_count \
                   FROM sessions s \
                   LEFT JOIN session_messages m ON m.session_id = s.id \
                   WHERE s.created_at >= {cutoff} \
                   GROUP BY s.id\
                 )"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let (total_turns, total_cost, avg_tokens_per_turn, cache_hit_rate): (i64, f64, f64, f64) = conn
        .query_row(
            &format!(
                "SELECT \
                   COUNT(*), \
                   COALESCE(SUM(cost), 0), \
                   COALESCE(AVG(tokens_in + tokens_out), 0), \
                   CASE WHEN COUNT(*) > 0 \
                     THEN CAST(SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS REAL) / COUNT(*) \
                     ELSE 0.0 END \
                 FROM inference_costs \
                 WHERE created_at >= {cutoff}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let mut model_stmt = conn
        .prepare(&format!(
            "SELECT \
               model, \
               COUNT(*) AS turns, \
               AVG(cost) AS avg_cost, \
               CASE WHEN COUNT(*) > 0 \
                 THEN CAST(SUM(CASE WHEN cached = 1 THEN 1 ELSE 0 END) AS REAL) / COUNT(*) \
                 ELSE 0.0 END AS cache_rate, \
               AVG(CAST(tokens_out AS REAL) / NULLIF(tokens_in, 0)) AS avg_density \
             FROM inference_costs \
             WHERE created_at >= {cutoff} \
             GROUP BY model \
             ORDER BY turns DESC"
        ))
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let mut models_used = Vec::new();
    let mut model_stats = HashMap::new();

    let rows = model_stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        })
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    for row in rows {
        let (model, turns, avg_cost, cache_rate, avg_density) =
            row.map_err(|e| IroncladError::Database(e.to_string()))?;
        models_used.push(model.clone());
        model_stats.insert(
            model,
            RecommendationModelStats {
                turns,
                avg_cost,
                avg_quality: None,
                cache_hit_rate: cache_rate,
                avg_output_density: avg_density.unwrap_or(0.0),
            },
        );
    }

    let tool_success_rate: f64 = conn
        .query_row(
            &format!(
                "SELECT CASE WHEN COUNT(*) > 0 \
                   THEN CAST(SUM(CASE WHEN status = 'success' THEN 1 ELSE 0 END) AS REAL) / COUNT(*) \
                   ELSE 1.0 END \
                 FROM tool_calls WHERE created_at >= {cutoff}"
            ),
            [],
            |row| row.get(0),
        )
        .map_err(|e| IroncladError::Database(e.to_string()))?;

    let (graded_turns, avg_quality): (i64, Option<f64>) = conn
        .query_row(
            &format!(
                "SELECT COUNT(*), AVG(CAST(tf.grade AS REAL)) \
                 FROM turn_feedback tf \
                 JOIN turns t ON t.id = tf.turn_id \
                 JOIN sessions s ON s.id = t.session_id \
                 WHERE s.created_at >= {cutoff}"
            ),
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap_or((0, None));

    let grade_coverage = if total_turns > 0 {
        graded_turns as f64 / total_turns as f64
    } else {
        0.0
    };

    // Compute memory retrieval rate from context_snapshots if available,
    // otherwise default to 0.5
    let memory_retrieval_rate: f64 = conn
        .query_row(
            &format!(
                "SELECT CASE WHEN COUNT(*) > 0 \
                   THEN CAST(SUM(CASE WHEN memory_tokens > 0 THEN 1 ELSE 0 END) AS REAL) / COUNT(*) \
                   ELSE 0.5 END \
                 FROM context_snapshots WHERE created_at >= {cutoff}"
            ),
            [],
            |row| row.get(0),
        )
        .unwrap_or(0.5);

    Ok(RecommendationUserProfile {
        total_sessions,
        total_turns,
        total_cost,
        avg_quality,
        grade_coverage,
        models_used,
        model_stats,
        avg_session_length,
        avg_tokens_per_turn,
        tool_success_rate,
        cache_hit_rate,
        memory_retrieval_rate,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::record_inference_cost;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn empty_database_returns_empty_report() {
        let db = test_db();
        let report = compute_efficiency(&db, "7d", None).unwrap();
        assert!(report.models.is_empty());
        assert!(report.time_series.is_empty());
        assert_eq!(report.totals.total_turns, 0);
        assert_eq!(report.totals.total_cost, 0.0);
    }

    #[test]
    fn single_model_report() {
        let db = test_db();
        record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            1000,
            500,
            0.015,
            Some("T1"),
            false,
        )
        .unwrap();
        record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            2000,
            800,
            0.025,
            Some("T1"),
            true,
        )
        .unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(report.models.len(), 1);
        let m = &report.models["claude-4"];
        assert_eq!(m.total_turns, 2);
        assert!(m.avg_output_density > 0.0);
        assert!((m.cost.total - 0.04).abs() < 1e-9);
        assert_eq!(m.cache_hit_rate, 0.5);
    }

    #[test]
    fn model_filter_works() {
        let db = test_db();
        record_inference_cost(&db, "claude-4", "anthropic", 100, 50, 0.01, None, false).unwrap();
        record_inference_cost(&db, "gpt-4", "openai", 200, 100, 0.02, None, false).unwrap();

        let report = compute_efficiency(&db, "all", Some("gpt-4")).unwrap();
        assert_eq!(report.models.len(), 1);
        assert!(report.models.contains_key("gpt-4"));
    }

    #[test]
    fn multiple_models_totals() {
        let db = test_db();
        record_inference_cost(&db, "claude-4", "anthropic", 100, 50, 0.01, None, false).unwrap();
        record_inference_cost(&db, "gpt-4", "openai", 200, 100, 0.02, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(report.totals.total_turns, 2);
        assert!((report.totals.total_cost - 0.03).abs() < 1e-9);
        assert_eq!(report.totals.most_expensive_model.as_deref(), Some("gpt-4"));
    }

    #[test]
    fn time_series_has_entries() {
        let db = test_db();
        record_inference_cost(&db, "claude-4", "anthropic", 100, 50, 0.01, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert!(!report.time_series.is_empty());
        assert_eq!(report.time_series[0].model, "claude-4");
    }

    #[test]
    fn trend_label_logic() {
        assert_eq!(trend_label(1.0, 1.5), "increasing");
        assert_eq!(trend_label(1.0, 0.5), "decreasing");
        assert_eq!(trend_label(1.0, 1.02), "stable");
        assert_eq!(trend_label(0.0, 0.0), "stable");
    }

    #[test]
    fn all_cached_full_rate() {
        let db = test_db();
        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, true).unwrap();
        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, true).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(report.models["m1"].cache_hit_rate, 1.0);
    }

    #[test]
    fn period_all_vs_default() {
        assert_eq!(cutoff_expr("all"), "datetime('1970-01-01')");
        assert_eq!(cutoff_expr("7d"), "datetime('now', '-7 days')");
        assert_eq!(cutoff_expr("unknown"), "datetime('1970-01-01')");
    }

    #[test]
    fn zero_tokens_in_no_division_by_zero() {
        let db = test_db();
        record_inference_cost(&db, "m1", "p1", 0, 50, 0.01, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        let m = &report.models["m1"];
        assert!(m.avg_output_density.is_finite());
    }

    #[test]
    fn cutoff_expr_1h() {
        assert_eq!(cutoff_expr("1h"), "datetime('now', '-1 hour')");
    }

    #[test]
    fn cutoff_expr_24h() {
        assert_eq!(cutoff_expr("24h"), "datetime('now', '-1 day')");
    }

    #[test]
    fn cutoff_expr_30d() {
        assert_eq!(cutoff_expr("30d"), "datetime('now', '-30 days')");
    }

    #[test]
    fn trend_label_edge_cases() {
        // Clearly within stable band (3% change)
        assert_eq!(trend_label(1.0, 1.03), "stable");
        // Clearly above threshold (10% increase)
        assert_eq!(trend_label(1.0, 1.10), "increasing");
        // Clearly within stable band (3% decrease)
        assert_eq!(trend_label(1.0, 0.97), "stable");
        // Clearly below threshold (10% decrease)
        assert_eq!(trend_label(1.0, 0.90), "decreasing");
        // First half near zero uses 0.001 as base
        assert_eq!(trend_label(0.001, 0.5), "increasing");
        // Both near zero
        assert_eq!(trend_label(0.0001, 0.0001), "stable");
    }

    #[test]
    fn zero_tokens_out_no_division_by_zero() {
        let db = test_db();
        record_inference_cost(&db, "m1", "p1", 1000, 0, 0.01, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        let m = &report.models["m1"];
        assert_eq!(m.cost.per_output_token, 0.0);
        assert!(m.cost.total.is_finite());
    }

    #[test]
    fn no_cached_zero_rate() {
        let db = test_db();
        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, false).unwrap();
        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(report.models["m1"].cache_hit_rate, 0.0);
    }

    #[test]
    fn multiple_models_identifies_most_efficient() {
        let db = test_db();
        // m1: 1000 in, 500 out -> density ~0.5
        record_inference_cost(&db, "m1", "p1", 1000, 500, 0.01, None, false).unwrap();
        // m2: 100 in, 200 out -> density ~2.0  (more efficient)
        record_inference_cost(&db, "m2", "p2", 100, 200, 0.005, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(
            report.totals.most_efficient_model.as_deref(),
            Some("m2"),
            "m2 has higher output density"
        );
    }

    #[test]
    fn trend_metrics_with_time_series() {
        let db = test_db();
        // Insert enough records over multiple days to generate time-series buckets
        let conn = db.conn();
        for i in 0..6 {
            let day = format!("2025-01-{:02}T12:00:00", i + 1);
            conn.execute(
                "INSERT INTO inference_costs (id, model, provider, tokens_in, tokens_out, cost, cached, created_at) \
                 VALUES (?1, 'claude-4', 'anthropic', ?2, ?3, ?4, 0, ?5)",
                rusqlite::params![
                    format!("ic-{i}"),
                    1000 + i * 100,
                    500 + i * 50,
                    0.01 + i as f64 * 0.005,
                    day,
                ],
            )
            .unwrap();
        }
        drop(conn);

        let report = compute_efficiency(&db, "all", None).unwrap();
        let m = &report.models["claude-4"];
        // With 6 data points over 6 different days, we should have time series data
        assert!(report.time_series.len() >= 2);
        // Trend should be computed (not just "stable")
        assert!(!m.trend.output_density.is_empty());
        assert!(!m.trend.cost_per_turn.is_empty());
        assert!(!m.trend.cache_hit_rate.is_empty());
    }

    #[test]
    fn build_user_profile_empty_db() {
        let db = test_db();
        let profile = build_user_profile(&db, "7d").unwrap();
        assert_eq!(profile.total_sessions, 0);
        assert_eq!(profile.total_turns, 0);
        assert_eq!(profile.total_cost, 0.0);
        assert!(profile.models_used.is_empty());
        assert!(profile.model_stats.is_empty());
        assert_eq!(profile.avg_session_length, 0.0);
        assert_eq!(profile.avg_tokens_per_turn, 0.0);
        assert_eq!(profile.tool_success_rate, 1.0); // No tools => default 1.0
    }

    #[test]
    fn build_user_profile_with_data() {
        let db = test_db();
        let conn = db.conn();
        // Create a session
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s1', 'agent-1', 'agent', 'active')",
            [],
        )
        .unwrap();
        // Create messages for the session
        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, content) VALUES ('m1', 's1', 'user', 'hello')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO session_messages (id, session_id, role, content) VALUES ('m2', 's1', 'assistant', 'hi')",
            [],
        )
        .unwrap();
        drop(conn);

        // Add inference costs
        record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            1000,
            500,
            0.015,
            Some("T1"),
            false,
        )
        .unwrap();
        record_inference_cost(
            &db,
            "claude-4",
            "anthropic",
            2000,
            800,
            0.025,
            Some("T1"),
            true,
        )
        .unwrap();
        record_inference_cost(&db, "gpt-4", "openai", 500, 200, 0.01, None, false).unwrap();

        // Add a tool call
        {
            let conn = db.conn();
            conn.execute("INSERT INTO turns (id, session_id) VALUES ('t1', 's1')", [])
                .unwrap();
            conn.execute(
                "INSERT INTO tool_calls (id, turn_id, tool_name, input, status) VALUES ('tc1', 't1', 'bash', '{}', 'success')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO tool_calls (id, turn_id, tool_name, input, status) VALUES ('tc2', 't1', 'bash', '{}', 'error')",
                [],
            )
            .unwrap();
        }

        let profile = build_user_profile(&db, "all").unwrap();
        assert_eq!(profile.total_sessions, 1);
        assert_eq!(profile.total_turns, 3);
        assert!((profile.total_cost - 0.05).abs() < 1e-9);
        assert!(profile.models_used.contains(&"claude-4".to_string()));
        assert!(profile.models_used.contains(&"gpt-4".to_string()));
        assert_eq!(profile.model_stats.len(), 2);
        assert_eq!(profile.model_stats["claude-4"].turns, 2);
        assert_eq!(profile.model_stats["gpt-4"].turns, 1);
        assert_eq!(profile.avg_session_length, 2.0); // 2 messages
        assert!(profile.avg_tokens_per_turn > 0.0);
        assert_eq!(profile.tool_success_rate, 0.5); // 1 success out of 2
        assert!(profile.cache_hit_rate > 0.0);
    }

    #[test]
    fn build_user_profile_grade_coverage() {
        let db = test_db();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s1', 'agent-1', 'agent', 'active')",
            [],
        )
        .unwrap();
        conn.execute("INSERT INTO turns (id, session_id) VALUES ('t1', 's1')", [])
            .unwrap();
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade, source) VALUES ('tf1', 't1', 's1', 4, 'dashboard')",
            [],
        )
        .unwrap();
        drop(conn);

        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, false).unwrap();
        record_inference_cost(&db, "m1", "p1", 100, 50, 0.01, None, false).unwrap();

        let profile = build_user_profile(&db, "all").unwrap();
        assert!(profile.avg_quality.is_some());
        assert!((profile.avg_quality.unwrap() - 4.0).abs() < 1e-9);
        assert!(profile.grade_coverage > 0.0);
        assert!(profile.grade_coverage <= 1.0);
    }

    #[test]
    fn compute_quality_for_model_with_feedback() {
        let db = test_db();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s1', 'a1', 'agent', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, model, cost) VALUES ('t1', 's1', 'claude-4', 0.01)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, model, cost) VALUES ('t2', 's1', 'claude-4', 0.02)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, model, cost) VALUES ('t3', 's1', 'claude-4', 0.015)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, model, cost) VALUES ('t4', 's1', 'claude-4', 0.025)",
            [],
        )
        .unwrap();
        // Add feedback for all 4 turns
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade) VALUES ('f1', 't1', 's1', 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade) VALUES ('f2', 't2', 's1', 4)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade) VALUES ('f3', 't3', 's1', 5)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade) VALUES ('f4', 't4', 's1', 5)",
            [],
        )
        .unwrap();

        let quality = compute_quality_for_model(&conn, "claude-4", "datetime('1970-01-01')", 4);
        drop(conn);

        assert!(quality.is_some());
        let q = quality.unwrap();
        assert_eq!(q.grade_count, 4);
        assert!((q.avg_grade - 4.25).abs() < 1e-9);
        assert_eq!(q.grade_coverage, 1.0);
        assert!(q.cost_per_quality_point > 0.0);
        // With 4 feedback entries and improvement from first half (3,4) to second half (5,5),
        // trend should be "increasing"
        assert_eq!(q.trend, "increasing");
    }

    #[test]
    fn compute_quality_for_model_no_feedback() {
        let db = test_db();
        let conn = db.conn();
        let quality = compute_quality_for_model(&conn, "claude-4", "datetime('1970-01-01')", 10);
        drop(conn);
        assert!(quality.is_none());
    }

    #[test]
    fn compute_quality_few_entries_stable_trend() {
        let db = test_db();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES ('s1', 'a1', 'agent', 'active')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, model, cost) VALUES ('t1', 's1', 'claude-4', 0.01)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turn_feedback (id, turn_id, session_id, grade) VALUES ('f1', 't1', 's1', 4)",
            [],
        )
        .unwrap();

        let quality = compute_quality_for_model(&conn, "claude-4", "datetime('1970-01-01')", 1);
        drop(conn);

        assert!(quality.is_some());
        let q = quality.unwrap();
        // With fewer than 4 entries, trend should be "stable"
        assert_eq!(q.trend, "stable");
    }

    #[test]
    fn report_cost_attribution_all_history() {
        let db = test_db();
        record_inference_cost(&db, "m1", "p1", 1000, 500, 0.03, None, false).unwrap();

        let report = compute_efficiency(&db, "all", None).unwrap();
        let m = &report.models["m1"];
        // Without context_snapshots, all input tokens are attributed to "history"
        assert_eq!(m.cost.attribution.history.pct, 100.0);
        assert!(m.cost.attribution.history.tokens > 0);
        assert_eq!(m.cost.attribution.system_prompt.tokens, 0);
        assert_eq!(m.cost.attribution.memories.tokens, 0);
    }

    #[test]
    fn report_biggest_cost_driver_with_no_models() {
        let db = test_db();
        let report = compute_efficiency(&db, "all", None).unwrap();
        assert_eq!(report.totals.biggest_cost_driver, "none");
        assert!(report.totals.most_expensive_model.is_none());
        assert!(report.totals.most_efficient_model.is_none());
    }

    #[test]
    fn build_user_profile_memory_retrieval_default() {
        let db = test_db();
        let profile = build_user_profile(&db, "all").unwrap();
        // Without context_snapshots, memory_retrieval_rate defaults to 0.5
        assert!((profile.memory_retrieval_rate - 0.5).abs() < 1e-9);
    }
}
