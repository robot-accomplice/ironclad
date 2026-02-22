use std::collections::HashMap;
use tracing::debug;

/// Outcome of a cascade attempt.
#[derive(Debug, Clone)]
pub struct CascadeOutcome {
    pub query_class: String,
    pub weak_model_succeeded: bool,
    pub weak_latency_ms: u64,
    pub strong_latency_ms: Option<u64>,
    pub total_cost: f64,
}

/// Tracks cascade outcomes for strategy optimization.
#[derive(Debug)]
pub struct CascadeOptimizer {
    outcomes: HashMap<String, Vec<CascadeOutcome>>,
    window_size: usize,
}

impl CascadeOptimizer {
    pub fn new(window_size: usize) -> Self {
        Self {
            outcomes: HashMap::new(),
            window_size,
        }
    }

    /// Record a cascade outcome.
    pub fn record(&mut self, outcome: CascadeOutcome) {
        let class = outcome.query_class.clone();
        let entries = self.outcomes.entry(class).or_default();
        entries.push(outcome);
        if entries.len() > self.window_size {
            entries.remove(0);
        }
    }

    /// Compute the expected utility of cascade vs. direct routing for a query class.
    /// Returns (cascade_utility, direct_utility).
    pub fn expected_utility(&self, query_class: &str) -> (f64, f64) {
        let outcomes = match self.outcomes.get(query_class) {
            Some(o) if !o.is_empty() => o,
            _ => return (0.5, 0.5),
        };

        let total = outcomes.len() as f64;
        let weak_success_count = outcomes.iter().filter(|o| o.weak_model_succeeded).count() as f64;
        let weak_success_rate = weak_success_count / total;

        let avg_weak_latency = outcomes
            .iter()
            .map(|o| o.weak_latency_ms as f64)
            .sum::<f64>()
            / total;
        let avg_strong_latency = outcomes
            .iter()
            .filter_map(|o| o.strong_latency_ms.map(|ms| ms as f64))
            .sum::<f64>()
            / outcomes
                .iter()
                .filter(|o| o.strong_latency_ms.is_some())
                .count()
                .max(1) as f64;

        let latency_weight = 0.001;
        let cascade_utility = weak_success_rate
            - latency_weight * avg_weak_latency
            - (1.0 - weak_success_rate) * latency_weight * avg_strong_latency;
        let direct_utility = 1.0 - latency_weight * avg_strong_latency;

        (cascade_utility, direct_utility)
    }

    /// Decide whether to cascade or go direct for a query class.
    pub fn should_cascade(&self, query_class: &str) -> CascadeStrategy {
        let (cascade_util, direct_util) = self.expected_utility(query_class);

        if cascade_util > direct_util {
            debug!(
                class = query_class,
                cascade = cascade_util,
                direct = direct_util,
                "cascade recommended"
            );
            CascadeStrategy::Cascade
        } else {
            debug!(
                class = query_class,
                cascade = cascade_util,
                direct = direct_util,
                "direct recommended"
            );
            CascadeStrategy::Direct
        }
    }

    /// Get the weak model success rate for a query class.
    pub fn weak_success_rate(&self, query_class: &str) -> f64 {
        match self.outcomes.get(query_class) {
            Some(outcomes) if !outcomes.is_empty() => {
                let successes = outcomes.iter().filter(|o| o.weak_model_succeeded).count();
                successes as f64 / outcomes.len() as f64
            }
            _ => 0.5,
        }
    }

    /// Get tracked query classes.
    pub fn query_classes(&self) -> Vec<&str> {
        self.outcomes.keys().map(|s| s.as_str()).collect()
    }

    /// Number of recorded outcomes for a class.
    pub fn observation_count(&self, query_class: &str) -> usize {
        self.outcomes.get(query_class).map(|o| o.len()).unwrap_or(0)
    }
}

/// Strategy decision: cascade (try weak first) or direct (go to strong).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CascadeStrategy {
    Cascade,
    Direct,
}

impl std::fmt::Display for CascadeStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CascadeStrategy::Cascade => write!(f, "cascade"),
            CascadeStrategy::Direct => write!(f, "direct"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cascade_win(class: &str) -> CascadeOutcome {
        CascadeOutcome {
            query_class: class.to_string(),
            weak_model_succeeded: true,
            weak_latency_ms: 200,
            strong_latency_ms: None,
            total_cost: 0.001,
        }
    }

    fn cascade_fail(class: &str) -> CascadeOutcome {
        CascadeOutcome {
            query_class: class.to_string(),
            weak_model_succeeded: false,
            weak_latency_ms: 200,
            strong_latency_ms: Some(2000),
            total_cost: 0.01,
        }
    }

    #[test]
    fn high_success_rate_favors_cascade() {
        let mut opt = CascadeOptimizer::new(100);
        for _ in 0..9 {
            opt.record(cascade_win("simple"));
        }
        opt.record(cascade_fail("simple"));

        let rate = opt.weak_success_rate("simple");
        assert!((rate - 0.9).abs() < f64::EPSILON);
        assert_eq!(opt.should_cascade("simple"), CascadeStrategy::Cascade);
    }

    #[test]
    fn low_success_rate_favors_direct() {
        let mut opt = CascadeOptimizer::new(100);
        for _ in 0..9 {
            opt.record(cascade_fail("complex"));
        }
        opt.record(cascade_win("complex"));

        let rate = opt.weak_success_rate("complex");
        assert!((rate - 0.1).abs() < f64::EPSILON);
        assert_eq!(opt.should_cascade("complex"), CascadeStrategy::Direct);
    }

    #[test]
    fn unknown_class_defaults() {
        let opt = CascadeOptimizer::new(100);
        let rate = opt.weak_success_rate("unknown");
        assert!((rate - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn window_eviction() {
        let mut opt = CascadeOptimizer::new(5);
        for _ in 0..10 {
            opt.record(cascade_win("test"));
        }
        assert_eq!(opt.observation_count("test"), 5);
    }

    #[test]
    fn expected_utility_no_data() {
        let opt = CascadeOptimizer::new(100);
        let (c, d) = opt.expected_utility("none");
        assert!((c - 0.5).abs() < f64::EPSILON);
        assert!((d - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn strategy_display() {
        assert_eq!(format!("{}", CascadeStrategy::Cascade), "cascade");
        assert_eq!(format!("{}", CascadeStrategy::Direct), "direct");
    }

    #[test]
    fn query_classes_listed() {
        let mut opt = CascadeOptimizer::new(100);
        opt.record(cascade_win("a"));
        opt.record(cascade_win("b"));
        let classes = opt.query_classes();
        assert_eq!(classes.len(), 2);
    }
}
