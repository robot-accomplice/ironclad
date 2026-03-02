use std::collections::{HashMap, VecDeque};

use tracing::info;

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

    /// Seed the tracker with historical observations (e.g. loaded from DB on startup).
    /// Each `(model, quality_score)` pair is recorded into the ring buffer as if
    /// the observations arrived in order. This gives metascore routing a warm start
    /// instead of assuming 0.8 for every model.
    pub fn seed_from_history(&mut self, observations: &[(String, f64)]) {
        let mut count = 0usize;
        for (model, score) in observations {
            self.record(model, *score);
            count += 1;
        }
        if count > 0 {
            info!(
                count,
                models = self.observations.len(),
                "seeded QualityTracker from historical observations"
            );
        }
    }
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

    #[test]
    fn seed_from_history_populates_tracker() {
        let mut tracker = QualityTracker::new(100);
        let history = vec![
            ("model-a".to_string(), 0.7),
            ("model-a".to_string(), 0.9),
            ("model-b".to_string(), 0.85),
        ];
        tracker.seed_from_history(&history);
        assert_eq!(tracker.observation_count("model-a"), 2);
        assert_eq!(tracker.observation_count("model-b"), 1);
        let q = tracker.estimated_quality("model-a").unwrap();
        assert!((q - 0.8).abs() < f64::EPSILON);
    }

    #[test]
    fn seed_from_history_empty() {
        let mut tracker = QualityTracker::new(100);
        tracker.seed_from_history(&[]);
        assert!(tracker.tracked_models().is_empty());
    }

    #[test]
    fn seed_from_history_respects_window() {
        let mut tracker = QualityTracker::new(2);
        let history = vec![
            ("m".to_string(), 0.1),
            ("m".to_string(), 0.2),
            ("m".to_string(), 0.9),
        ];
        tracker.seed_from_history(&history);
        assert_eq!(tracker.observation_count("m"), 2);
        // Window keeps last 2: 0.2, 0.9
        let q = tracker.estimated_quality("m").unwrap();
        assert!((q - 0.55).abs() < f64::EPSILON);
    }
}
