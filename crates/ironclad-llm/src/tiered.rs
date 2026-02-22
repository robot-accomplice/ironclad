use tracing::debug;

/// Evaluates whether a local model's response is confident enough to skip cloud escalation.
#[derive(Debug)]
pub struct ConfidenceEvaluator {
    floor: f64,
}

impl ConfidenceEvaluator {
    pub fn new(confidence_floor: f64) -> Self {
        Self {
            floor: confidence_floor,
        }
    }

    /// Score response confidence based on multiple heuristics.
    /// Returns a value in [0.0, 1.0].
    pub fn evaluate(&self, response: &str, latency_ms: u64) -> f64 {
        if response.is_empty() {
            debug!(
                len = 0_usize,
                final_score = 0.1,
                "confidence evaluation: empty response"
            );
            return 0.1;
        }

        let mut score = 0.0;
        let mut signals = 0;

        let length_score = match response.len() {
            0..=10 => 0.2,
            11..=50 => 0.5,
            51..=200 => 0.7,
            201..=1000 => 0.85,
            _ => 0.9,
        };
        score += length_score;
        signals += 1;

        let hedging_phrases = [
            "i'm not sure",
            "i don't know",
            "it's unclear",
            "i cannot",
            "i can't",
            "uncertain",
            "might be",
            "possibly",
            "perhaps",
            "i think maybe",
        ];
        let lower = response.to_lowercase();
        let hedge_count = hedging_phrases
            .iter()
            .filter(|p| lower.contains(*p))
            .count();
        let hedge_score = match hedge_count {
            0 => 1.0,
            1 => 0.6,
            2 => 0.3,
            _ => 0.1,
        };
        score += hedge_score;
        signals += 1;

        let latency_score = if latency_ms < 200 {
            0.9
        } else if latency_ms < 2000 {
            0.7
        } else if latency_ms < 5000 {
            0.5
        } else {
            0.3
        };
        score += latency_score;
        signals += 1;

        let ends_well = response.ends_with('.')
            || response.ends_with('!')
            || response.ends_with('?')
            || response.ends_with("```")
            || response.ends_with('\n');
        let structure_score = if ends_well { 0.8 } else { 0.4 };
        score += structure_score;
        signals += 1;

        let final_score = score / signals as f64;
        debug!(
            len = response.len(),
            hedges = hedge_count,
            latency_ms,
            ends_well,
            final_score,
            "confidence evaluation"
        );
        final_score
    }

    /// Whether the response is confident enough to accept without escalation.
    pub fn is_confident(&self, response: &str, latency_ms: u64) -> bool {
        self.evaluate(response, latency_ms) >= self.floor
    }

    pub fn floor(&self) -> f64 {
        self.floor
    }
}

/// Outcome of a tiered inference attempt.
#[derive(Debug, Clone)]
pub struct TieredResult {
    pub response: String,
    pub tier_used: InferenceTier,
    pub confidence: f64,
    pub escalated: bool,
}

/// Which tier produced the final response.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InferenceTier {
    Cache,
    Local,
    Cloud,
}

impl std::fmt::Display for InferenceTier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InferenceTier::Cache => write!(f, "cache"),
            InferenceTier::Local => write!(f, "local"),
            InferenceTier::Cloud => write!(f, "cloud"),
        }
    }
}

/// Tracks escalation statistics for monitoring and tuning.
#[derive(Debug, Default)]
pub struct EscalationTracker {
    pub cache_hits: u64,
    pub local_accepted: u64,
    pub local_escalated: u64,
    pub cloud_direct: u64,
}

impl EscalationTracker {
    pub fn record(&mut self, tier: InferenceTier, escalated: bool) {
        match tier {
            InferenceTier::Cache => self.cache_hits += 1,
            InferenceTier::Local => {
                if escalated {
                    self.local_escalated += 1;
                } else {
                    self.local_accepted += 1;
                }
            }
            InferenceTier::Cloud => self.cloud_direct += 1,
        }
    }

    pub fn total(&self) -> u64 {
        self.cache_hits + self.local_accepted + self.local_escalated + self.cloud_direct
    }

    pub fn local_acceptance_rate(&self) -> f64 {
        let local_total = self.local_accepted + self.local_escalated;
        if local_total == 0 {
            return 0.0;
        }
        self.local_accepted as f64 / local_total as f64
    }

    pub fn cache_hit_rate(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.cache_hits as f64 / total as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evaluator_high_confidence() {
        let eval = ConfidenceEvaluator::new(0.6);
        let response =
            "The answer to your question is 42. This is a well-known mathematical constant.";
        let confidence = eval.evaluate(response, 100);
        assert!(
            confidence > 0.7,
            "detailed response should have high confidence: {confidence}"
        );
        assert!(eval.is_confident(response, 100));
    }

    #[test]
    fn evaluator_low_confidence_hedging() {
        let eval = ConfidenceEvaluator::new(0.6);
        let response = "I'm not sure, but perhaps it might be 42? I think maybe.";
        let confidence = eval.evaluate(response, 300);
        assert!(
            confidence < 0.6,
            "hedging response should have low confidence: {confidence}"
        );
        assert!(!eval.is_confident(response, 300));
    }

    #[test]
    fn evaluator_low_confidence_short() {
        let eval = ConfidenceEvaluator::new(0.6);
        let response = "IDK";
        let confidence = eval.evaluate(response, 5000);
        assert!(
            confidence < 0.5,
            "short + slow should be low confidence: {confidence}"
        );
    }

    #[test]
    fn evaluator_empty_response() {
        let eval = ConfidenceEvaluator::new(0.6);
        let confidence = eval.evaluate("", 100);
        assert!(confidence < 0.6, "empty should be low: {confidence}");
    }

    #[test]
    fn evaluator_floor_accessor() {
        let eval = ConfidenceEvaluator::new(0.75);
        assert!((eval.floor() - 0.75).abs() < f64::EPSILON);
    }

    #[test]
    fn escalation_tracker_basic() {
        let mut tracker = EscalationTracker::default();
        tracker.record(InferenceTier::Cache, false);
        tracker.record(InferenceTier::Cache, false);
        tracker.record(InferenceTier::Local, false);
        tracker.record(InferenceTier::Local, true);
        tracker.record(InferenceTier::Cloud, false);

        assert_eq!(tracker.total(), 5);
        assert_eq!(tracker.cache_hits, 2);
        assert_eq!(tracker.local_accepted, 1);
        assert_eq!(tracker.local_escalated, 1);
        assert_eq!(tracker.cloud_direct, 1);
    }

    #[test]
    fn escalation_tracker_rates() {
        let mut tracker = EscalationTracker::default();
        for _ in 0..7 {
            tracker.record(InferenceTier::Local, false);
        }
        for _ in 0..3 {
            tracker.record(InferenceTier::Local, true);
        }

        let rate = tracker.local_acceptance_rate();
        assert!((rate - 0.7).abs() < f64::EPSILON, "7/10 = 0.7, got {rate}");
    }

    #[test]
    fn escalation_tracker_empty_rates() {
        let tracker = EscalationTracker::default();
        assert!((tracker.local_acceptance_rate() - 0.0).abs() < f64::EPSILON);
        assert!((tracker.cache_hit_rate() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tiered_result_display() {
        assert_eq!(format!("{}", InferenceTier::Cache), "cache");
        assert_eq!(format!("{}", InferenceTier::Local), "local");
        assert_eq!(format!("{}", InferenceTier::Cloud), "cloud");
    }

    #[test]
    fn escalation_tracker_cache_hit_rate() {
        let mut tracker = EscalationTracker::default();
        for _ in 0..3 {
            tracker.record(InferenceTier::Cache, false);
        }
        for _ in 0..7 {
            tracker.record(InferenceTier::Local, false);
        }
        let rate = tracker.cache_hit_rate();
        assert!((rate - 0.3).abs() < f64::EPSILON, "3/10 = 0.3, got {rate}");
    }
}
