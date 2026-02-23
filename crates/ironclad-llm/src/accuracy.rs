use std::collections::{HashMap, VecDeque};

use tracing::{debug, info};

use crate::provider::ProviderRegistry;

/// Tracks observed quality scores per model for accuracy-target routing.
#[derive(Debug)]
pub struct QualityTracker {
    observations: HashMap<String, VecDeque<f64>>,
    window_size: usize,
}

impl QualityTracker {
    pub fn new(window_size: usize) -> Self {
        Self {
            observations: HashMap::new(),
            window_size,
        }
    }

    /// Record an observed quality score for a model.
    pub fn record(&mut self, model: &str, quality: f64) {
        let scores = self.observations.entry(model.to_string()).or_default();
        scores.push_back(quality.clamp(0.0, 1.0));
        if scores.len() > self.window_size {
            scores.pop_front();
        }
    }

    /// Get the estimated quality for a model (moving average).
    pub fn estimated_quality(&self, model: &str) -> Option<f64> {
        self.observations.get(model).and_then(|scores| {
            if scores.is_empty() {
                None
            } else {
                Some(scores.iter().sum::<f64>() / scores.len() as f64)
            }
        })
    }

    /// Get the number of observations for a model.
    pub fn observation_count(&self, model: &str) -> usize {
        self.observations.get(model).map(|s| s.len()).unwrap_or(0)
    }

    pub fn tracked_models(&self) -> Vec<&str> {
        self.observations.keys().map(|s| s.as_str()).collect()
    }
}

/// Selects the cheapest model that meets the quality target.
/// Uses a Lagrangian-style constraint: minimize cost subject to E\[quality\] >= target.
pub fn select_for_quality_target(
    target: f64,
    candidates: &[&str],
    quality_tracker: &QualityTracker,
    registry: &ProviderRegistry,
    input_tokens: u32,
    output_tokens: u32,
) -> Option<String> {
    let mut qualified: Vec<(&str, f64, f64)> = Vec::new();

    for &model in candidates {
        let quality = quality_tracker.estimated_quality(model).unwrap_or(0.8);

        if quality >= target {
            let cost = match registry.get_by_model(model) {
                Some(provider) => {
                    provider.cost_per_input_token * input_tokens as f64
                        + provider.cost_per_output_token * output_tokens as f64
                }
                None => f64::MAX,
            };
            qualified.push((model, quality, cost));
        }
    }

    if qualified.is_empty() {
        debug!(
            target,
            "no model meets quality target, falling back to highest quality"
        );
        let mut all: Vec<(&str, f64)> = candidates
            .iter()
            .map(|&m| (m, quality_tracker.estimated_quality(m).unwrap_or(0.8)))
            .collect();
        all.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        return all.first().map(|(m, _)| m.to_string());
    }

    qualified.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));

    let selected = qualified[0].0;
    info!(
        selected,
        quality = qualified[0].1,
        cost = qualified[0].2,
        target,
        "quality-target routing selected model"
    );

    Some(selected.to_string())
}

/// Compute the Lagrangian cost function: cost + lambda * max(0, target - quality).
/// This penalizes models that fall below the quality target.
pub fn lagrangian_cost(cost: f64, quality: f64, target: f64, lambda: f64) -> f64 {
    let constraint_violation = (target - quality).max(0.0);
    cost + lambda * constraint_violation
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{Provider, ProviderRegistry};
    use ironclad_core::{ApiFormat, ModelTier};
    use std::collections::HashMap as StdHashMap;

    fn test_registry() -> ProviderRegistry {
        let mut reg = ProviderRegistry::new();
        reg.register(Provider {
            name: "cheap".into(),
            url: "http://cheap.test".into(),
            tier: ModelTier::T1,
            api_key_env: "K".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            auth_header: "Authorization".into(),
            extra_headers: StdHashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg.register(Provider {
            name: "mid".into(),
            url: "http://mid.test".into(),
            tier: ModelTier::T2,
            api_key_env: "K".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: false,
            cost_per_input_token: 0.00005,
            cost_per_output_token: 0.00015,
            auth_header: "Authorization".into(),
            extra_headers: StdHashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg.register(Provider {
            name: "premium".into(),
            url: "http://premium.test".into(),
            tier: ModelTier::T4,
            api_key_env: "K".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: false,
            cost_per_input_token: 0.001,
            cost_per_output_token: 0.003,
            auth_header: "Authorization".into(),
            extra_headers: StdHashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg
    }

    #[test]
    fn tracker_record_and_query() {
        let mut tracker = QualityTracker::new(100);
        tracker.record("model-a", 0.9);
        tracker.record("model-a", 0.8);
        let q = tracker.estimated_quality("model-a").unwrap();
        assert!((q - 0.85).abs() < f64::EPSILON);
        assert_eq!(tracker.observation_count("model-a"), 2);
    }

    #[test]
    fn tracker_unknown_model() {
        let tracker = QualityTracker::new(100);
        assert!(tracker.estimated_quality("unknown").is_none());
        assert_eq!(tracker.observation_count("unknown"), 0);
    }

    #[test]
    fn tracker_window_size() {
        let mut tracker = QualityTracker::new(3);
        for i in 0..5 {
            tracker.record("m", i as f64 * 0.2);
        }
        assert_eq!(tracker.observation_count("m"), 3);
        let q = tracker.estimated_quality("m").unwrap();
        assert!((q - (0.4 + 0.6 + 0.8) / 3.0).abs() < 1e-10);
    }

    #[test]
    fn tracker_clamp() {
        let mut tracker = QualityTracker::new(10);
        tracker.record("m", 1.5);
        tracker.record("m", -0.5);
        assert!((tracker.estimated_quality("m").unwrap() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn select_cheapest_meeting_target() {
        let mut tracker = QualityTracker::new(100);
        tracker.record("cheap/model", 0.7);
        tracker.record("mid/model", 0.85);
        tracker.record("premium/model", 0.95);

        let reg = test_registry();
        let candidates = vec!["cheap/model", "mid/model", "premium/model"];

        let selected =
            select_for_quality_target(0.8, &candidates, &tracker, &reg, 1000, 500).unwrap();
        assert_eq!(
            selected, "mid/model",
            "should pick cheapest that meets 0.8 target"
        );
    }

    #[test]
    fn select_fallback_highest_quality() {
        let mut tracker = QualityTracker::new(100);
        tracker.record("cheap/model", 0.5);
        tracker.record("mid/model", 0.6);
        tracker.record("premium/model", 0.7);

        let reg = test_registry();
        let candidates = vec!["cheap/model", "mid/model", "premium/model"];

        let selected =
            select_for_quality_target(0.99, &candidates, &tracker, &reg, 1000, 500).unwrap();
        assert_eq!(
            selected, "premium/model",
            "no model meets 0.99, fallback to highest quality"
        );
    }

    #[test]
    fn select_with_no_data_uses_default() {
        let tracker = QualityTracker::new(100);
        let reg = test_registry();
        let candidates = vec!["cheap/model", "mid/model"];

        let selected = select_for_quality_target(0.7, &candidates, &tracker, &reg, 1000, 500);
        assert!(selected.is_some());
    }

    #[test]
    fn lagrangian_no_violation() {
        let cost = lagrangian_cost(1.0, 0.9, 0.8, 10.0);
        assert!(
            (cost - 1.0).abs() < f64::EPSILON,
            "no violation, cost unchanged"
        );
    }

    #[test]
    fn lagrangian_with_violation() {
        let cost = lagrangian_cost(1.0, 0.5, 0.8, 10.0);
        let expected = 1.0 + 10.0 * 0.3;
        assert!(
            (cost - expected).abs() < f64::EPSILON,
            "violation penalty applied"
        );
    }

    #[test]
    fn tracked_models_list() {
        let mut tracker = QualityTracker::new(100);
        tracker.record("a", 0.5);
        tracker.record("b", 0.6);
        let models = tracker.tracked_models();
        assert_eq!(models.len(), 2);
    }
}
