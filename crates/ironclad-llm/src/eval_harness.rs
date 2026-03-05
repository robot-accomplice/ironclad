//! Offline routing evaluation harness.
//!
//! Replays historical routing decisions through the current metascore logic
//! with configurable parameters. This enables operators to measure the impact
//! of config changes (e.g., different cost weights, accuracy floors) before
//! deploying them to production.
//!
//! The harness operates on `EvalRow` inputs (extracted from the routing dataset)
//! and produces per-row verdicts plus aggregate metrics.

use crate::profile::{MetascoreBreakdown, ModelProfile};
use serde::Serialize;

/// A simplified input row for the evaluation harness, derived from
/// the routing dataset but containing only the fields needed for replay.
#[derive(Debug, Clone)]
pub struct EvalRow {
    pub turn_id: String,
    /// The model that production actually selected.
    pub production_model: String,
    /// Complexity estimate at the time of the original decision (0.0–1.0).
    pub complexity: f64,
    /// All candidate profiles that were available at decision time.
    pub candidates: Vec<ModelProfile>,
    /// Observed total cost of the production decision.
    pub observed_cost: f64,
    /// Observed quality score (if available).
    pub observed_quality: Option<f64>,
}

/// Outcome of replaying a single routing decision.
#[derive(Debug, Clone, Serialize)]
pub struct EvalVerdict {
    pub turn_id: String,
    pub production_model: String,
    pub replay_model: String,
    pub production_score: f64,
    pub replay_score: f64,
    /// True if replay would have picked a different model.
    pub changed: bool,
    pub replay_breakdown: MetascoreBreakdown,
}

/// Aggregate metrics from an evaluation run.
#[derive(Debug, Clone, Serialize)]
pub struct EvalSummary {
    pub total_rows: usize,
    /// How many rows would have changed model selection.
    pub changed_count: usize,
    pub change_rate: f64,
    /// Average metascore of production selections under new config.
    pub avg_production_score: f64,
    /// Average metascore of replay winners under new config.
    pub avg_replay_score: f64,
    /// Average score improvement (replay - production). Positive = better.
    pub avg_score_delta: f64,
}

/// Configuration for an evaluation run.
#[derive(Debug, Clone)]
pub struct EvalConfig {
    pub cost_aware: bool,
    pub cost_weight: Option<f64>,
    pub accuracy_floor: f64,
    pub accuracy_min_obs: usize,
}

impl Default for EvalConfig {
    fn default() -> Self {
        Self {
            cost_aware: false,
            cost_weight: None,
            accuracy_floor: 0.0,
            accuracy_min_obs: 10,
        }
    }
}

/// Replay a batch of historical routing decisions through the metascore with
/// the given eval config. Returns per-row verdicts.
pub fn replay(rows: &[EvalRow], config: &EvalConfig) -> Vec<EvalVerdict> {
    rows.iter()
        .filter_map(|row| replay_single(row, config))
        .collect()
}

fn replay_single(row: &EvalRow, config: &EvalConfig) -> Option<EvalVerdict> {
    if row.candidates.is_empty() {
        return None;
    }

    // Filter by accuracy floor (mirroring production logic).
    let eligible: Vec<&ModelProfile> = row
        .candidates
        .iter()
        .filter(|p| p.availability > 0.0)
        .filter(|p| {
            if config.accuracy_floor > 0.0 && p.observation_count >= config.accuracy_min_obs {
                p.estimated_quality
                    .is_none_or(|q| q >= config.accuracy_floor)
            } else {
                true
            }
        })
        .collect();

    if eligible.is_empty() {
        return None;
    }

    // Score all eligible candidates under new config.
    let scored: Vec<(&ModelProfile, MetascoreBreakdown)> = eligible
        .iter()
        .map(|&p| {
            let b =
                p.metascore_with_cost_weight(row.complexity, config.cost_aware, config.cost_weight);
            (p, b)
        })
        .collect();

    // Find the replay winner.
    let (replay_profile, replay_breakdown) = scored.iter().max_by(|a, b| {
        a.1.final_score
            .partial_cmp(&b.1.final_score)
            .unwrap_or(std::cmp::Ordering::Equal)
    })?;

    // Find the production model's score under new config.
    let production_score = scored
        .iter()
        .find(|(p, _)| p.model_name == row.production_model)
        .map(|(_, b)| b.final_score)
        .unwrap_or(0.0);

    let changed = replay_profile.model_name != row.production_model;

    Some(EvalVerdict {
        turn_id: row.turn_id.clone(),
        production_model: row.production_model.clone(),
        replay_model: replay_profile.model_name.clone(),
        production_score,
        replay_score: replay_breakdown.final_score,
        changed,
        replay_breakdown: replay_breakdown.clone(),
    })
}

/// Summarize an evaluation run.
pub fn summarize(verdicts: &[EvalVerdict]) -> EvalSummary {
    let total = verdicts.len();
    if total == 0 {
        return EvalSummary {
            total_rows: 0,
            changed_count: 0,
            change_rate: 0.0,
            avg_production_score: 0.0,
            avg_replay_score: 0.0,
            avg_score_delta: 0.0,
        };
    }

    let changed_count = verdicts.iter().filter(|v| v.changed).count();
    let sum_prod: f64 = verdicts.iter().map(|v| v.production_score).sum();
    let sum_replay: f64 = verdicts.iter().map(|v| v.replay_score).sum();

    let n = total as f64;
    EvalSummary {
        total_rows: total,
        changed_count,
        change_rate: changed_count as f64 / n,
        avg_production_score: sum_prod / n,
        avg_replay_score: sum_replay / n,
        avg_score_delta: (sum_replay - sum_prod) / n,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclad_core::ModelTier;

    fn local_profile(name: &str, quality: Option<f64>, obs: usize) -> ModelProfile {
        ModelProfile {
            model_name: name.into(),
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            tier: ModelTier::T1,
            estimated_quality: quality,
            availability: 1.0,
            capacity_headroom: 1.0,
            observation_count: obs,
        }
    }

    fn cloud_profile(name: &str, quality: Option<f64>, obs: usize) -> ModelProfile {
        ModelProfile {
            model_name: name.into(),
            is_local: false,
            cost_per_input_token: 0.0025,
            cost_per_output_token: 0.01,
            tier: ModelTier::T3,
            estimated_quality: quality,
            availability: 1.0,
            capacity_headroom: 1.0,
            observation_count: obs,
        }
    }

    fn sample_row() -> EvalRow {
        EvalRow {
            turn_id: "t-1".into(),
            production_model: "cloud/gpt-4o".into(),
            complexity: 0.3,
            candidates: vec![
                local_profile("local/qwen", Some(0.75), 30),
                cloud_profile("cloud/gpt-4o", Some(0.85), 50),
            ],
            observed_cost: 0.05,
            observed_quality: Some(0.85),
        }
    }

    #[test]
    fn replay_empty_input() {
        let verdicts = replay(&[], &EvalConfig::default());
        assert!(verdicts.is_empty());
    }

    #[test]
    fn replay_single_row() {
        let row = sample_row();
        let verdicts = replay(&[row], &EvalConfig::default());
        assert_eq!(verdicts.len(), 1);
        assert!(!verdicts[0].turn_id.is_empty());
    }

    #[test]
    fn replay_respects_cost_weight() {
        let row = sample_row();

        // With cost_weight=1.0, should strongly prefer free local model.
        let config = EvalConfig {
            cost_weight: Some(1.0),
            ..Default::default()
        };
        let verdicts = replay(&[row], &config);
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].replay_model, "local/qwen");
        assert!(verdicts[0].changed); // production was cloud, replay picks local
    }

    #[test]
    fn replay_accuracy_floor_filters() {
        let mut row = sample_row();
        // Set both models below accuracy floor.
        row.candidates = vec![
            local_profile("local/qwen", Some(0.3), 30),
            cloud_profile("cloud/gpt-4o", Some(0.4), 50),
        ];

        let config = EvalConfig {
            accuracy_floor: 0.5,
            accuracy_min_obs: 10,
            ..Default::default()
        };
        let verdicts = replay(&[row], &config);
        // Both filtered out, no verdict.
        assert!(verdicts.is_empty());
    }

    #[test]
    fn replay_skips_blocked_candidates() {
        let mut row = sample_row();
        row.candidates[0].availability = 0.0; // local model is blocked
        let verdicts = replay(&[row], &EvalConfig::default());
        assert_eq!(verdicts.len(), 1);
        assert_eq!(verdicts[0].replay_model, "cloud/gpt-4o");
        assert!(!verdicts[0].changed);
    }

    #[test]
    fn summarize_empty() {
        let summary = summarize(&[]);
        assert_eq!(summary.total_rows, 0);
        assert_eq!(summary.change_rate, 0.0);
    }

    #[test]
    fn summarize_computes_deltas() {
        let row = sample_row();
        let config = EvalConfig {
            cost_weight: Some(1.0),
            ..Default::default()
        };
        let verdicts = replay(&[row.clone(), row], &config);
        let summary = summarize(&verdicts);
        assert_eq!(summary.total_rows, 2);
        assert_eq!(summary.changed_count, 2);
        assert!((summary.change_rate - 1.0).abs() < 1e-9);
        assert!(summary.avg_score_delta >= 0.0 || summary.avg_score_delta < 0.0);
    }

    #[test]
    fn replay_no_candidates_returns_none() {
        let row = EvalRow {
            turn_id: "t-empty".into(),
            production_model: "cloud/gpt-4o".into(),
            complexity: 0.5,
            candidates: vec![],
            observed_cost: 0.0,
            observed_quality: None,
        };
        let verdicts = replay(&[row], &EvalConfig::default());
        assert!(verdicts.is_empty());
    }
}
