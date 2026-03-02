//! Abuse signal aggregation and enforcement.
//!
//! The `AbuseTracker` correlates rate-limit hits, policy violations, and
//! anomalous patterns across actors, origins, and channels. It produces
//! graduated enforcement actions (allow / slowdown / quarantine) and
//! persists an audit trail to the `abuse_events` table.
//!
//! Lives in `AppState` as `Arc<RwLock<AbuseTracker>>` so both API and
//! channel entry points share the same abuse view.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tracing::{info, warn};

/// Action the system should take for a given request.
#[derive(Debug, Clone, PartialEq)]
pub enum AbuseAction {
    /// Request proceeds normally.
    Allow,
    /// Request proceeds after an artificial delay.
    Slowdown(Duration),
    /// Request is rejected outright.
    Quarantine(Duration),
}

/// Categories of abuse signals fed into the tracker.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SignalType {
    /// HTTP/channel rate limit was hit or nearly hit.
    RateBurst,
    /// PolicyEngine denied a tool call.
    PolicyViolation,
    /// Repeated identical or near-identical messages.
    RepetitionSpam,
    /// Rapid session creation without meaningful interaction.
    SessionChurn,
    /// Requests targeting admin/sensitive endpoints.
    SensitiveProbe,
}

impl SignalType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RateBurst => "rate_burst",
            Self::PolicyViolation => "policy_violation",
            Self::RepetitionSpam => "repetition_spam",
            Self::SessionChurn => "session_churn",
            Self::SensitiveProbe => "sensitive_probe",
        }
    }
}

/// Per-actor abuse signal accumulator with time-decay.
#[derive(Debug, Clone)]
struct ActorSignals {
    /// (signal_type, timestamp) pairs — oldest pruned on evaluate.
    events: Vec<(SignalType, Instant)>,
    /// Currently active quarantine expiry, if any.
    quarantine_until: Option<Instant>,
    /// Currently active slowdown expiry, if any.
    slowdown_until: Option<Instant>,
}

impl ActorSignals {
    fn new() -> Self {
        Self {
            events: Vec::new(),
            quarantine_until: None,
            slowdown_until: None,
        }
    }

    fn prune_stale(&mut self, window: Duration) {
        let cutoff = Instant::now() - window;
        self.events.retain(|(_, ts)| *ts > cutoff);
    }
}

/// Configurable thresholds for abuse scoring.
#[derive(Debug, Clone)]
pub struct AbuseConfig {
    /// Time window over which signals are aggregated.
    pub window: Duration,
    /// Score threshold above which slowdown is applied.
    pub slowdown_threshold: f64,
    /// Score threshold above which quarantine is applied.
    pub quarantine_threshold: f64,
    /// Duration of slowdown penalty.
    pub slowdown_duration: Duration,
    /// Duration of quarantine penalty.
    pub quarantine_duration: Duration,
    /// Maximum tracked actors (LRU eviction of oldest inactive).
    pub max_tracked_actors: usize,
}

impl Default for AbuseConfig {
    fn default() -> Self {
        Self {
            window: Duration::from_secs(300),
            slowdown_threshold: 0.5,
            quarantine_threshold: 0.8,
            slowdown_duration: Duration::from_secs(5),
            quarantine_duration: Duration::from_secs(60),
            max_tracked_actors: 10_000,
        }
    }
}

/// Correlates abuse signals across actors and produces enforcement actions.
#[derive(Debug)]
pub struct AbuseTracker {
    config: AbuseConfig,
    actors: HashMap<String, ActorSignals>,
}

impl AbuseTracker {
    pub fn new(config: AbuseConfig) -> Self {
        Self {
            config,
            actors: HashMap::new(),
        }
    }

    /// Record an abuse signal for the given actor.
    pub fn record_signal(&mut self, actor_id: &str, signal: SignalType) {
        let entry = self
            .actors
            .entry(actor_id.to_string())
            .or_insert_with(ActorSignals::new);
        entry.events.push((signal, Instant::now()));

        // Evict oldest actors if we exceed the cap
        if self.actors.len() > self.config.max_tracked_actors {
            self.evict_oldest();
        }
    }

    /// Evaluate the current abuse score for an actor and return the
    /// appropriate enforcement action.
    pub fn evaluate(&mut self, actor_id: &str) -> (AbuseAction, f64) {
        let entry = match self.actors.get_mut(actor_id) {
            Some(e) => e,
            None => return (AbuseAction::Allow, 0.0),
        };

        // Check active quarantine
        if let Some(until) = entry.quarantine_until {
            if Instant::now() < until {
                return (AbuseAction::Quarantine(until - Instant::now()), 1.0);
            }
            entry.quarantine_until = None;
        }

        // Check active slowdown
        if let Some(until) = entry.slowdown_until {
            if Instant::now() < until {
                return (
                    AbuseAction::Slowdown(until - Instant::now()),
                    self.config.slowdown_threshold,
                );
            }
            entry.slowdown_until = None;
        }

        // Prune stale signals and score
        entry.prune_stale(self.config.window);
        let score = Self::compute_score(&entry.events);

        if score >= self.config.quarantine_threshold {
            let until = Instant::now() + self.config.quarantine_duration;
            entry.quarantine_until = Some(until);
            warn!(actor = %actor_id, score, "abuse quarantine triggered");
            (
                AbuseAction::Quarantine(self.config.quarantine_duration),
                score,
            )
        } else if score >= self.config.slowdown_threshold {
            let until = Instant::now() + self.config.slowdown_duration;
            entry.slowdown_until = Some(until);
            info!(actor = %actor_id, score, "abuse slowdown triggered");
            (AbuseAction::Slowdown(self.config.slowdown_duration), score)
        } else {
            (AbuseAction::Allow, score)
        }
    }

    /// Compute a 0.0–1.0 abuse score from recent signals.
    ///
    /// Signal weights:
    /// - RateBurst: 0.15 each
    /// - PolicyViolation: 0.25 each
    /// - RepetitionSpam: 0.10 each
    /// - SessionChurn: 0.10 each
    /// - SensitiveProbe: 0.30 each
    ///
    /// Score is clamped to [0.0, 1.0].
    fn compute_score(events: &[(SignalType, Instant)]) -> f64 {
        let mut total: f64 = 0.0;
        for (sig, _) in events {
            total += match sig {
                SignalType::RateBurst => 0.15,
                SignalType::PolicyViolation => 0.25,
                SignalType::RepetitionSpam => 0.10,
                SignalType::SessionChurn => 0.10,
                SignalType::SensitiveProbe => 0.30,
            };
        }
        total.min(1.0)
    }

    /// Remove the oldest inactive actor to stay within cap.
    fn evict_oldest(&mut self) {
        let oldest = self
            .actors
            .iter()
            .filter_map(|(id, sig)| sig.events.last().map(|(_, ts)| (id.clone(), *ts)))
            .min_by_key(|(_, ts)| *ts);
        if let Some((id, _)) = oldest {
            self.actors.remove(&id);
        }
    }

    /// Operator-visible summary of tracked actors and their scores.
    pub fn snapshot(&mut self) -> Vec<(String, f64, usize)> {
        let window = self.config.window;
        self.actors
            .iter_mut()
            .map(|(id, sig)| {
                sig.prune_stale(window);
                let score = Self::compute_score(&sig.events);
                (id.clone(), score, sig.events.len())
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> AbuseConfig {
        AbuseConfig {
            window: Duration::from_secs(60),
            slowdown_threshold: 0.4,
            quarantine_threshold: 0.7,
            slowdown_duration: Duration::from_millis(100),
            quarantine_duration: Duration::from_millis(500),
            max_tracked_actors: 100,
        }
    }

    #[test]
    fn clean_actor_is_allowed() {
        let mut tracker = AbuseTracker::new(test_config());
        let (action, score) = tracker.evaluate("clean-user");
        assert_eq!(action, AbuseAction::Allow);
        assert!((score - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn single_signal_below_threshold() {
        let mut tracker = AbuseTracker::new(test_config());
        tracker.record_signal("user-1", SignalType::RateBurst);
        let (action, score) = tracker.evaluate("user-1");
        assert_eq!(action, AbuseAction::Allow);
        assert!((score - 0.15).abs() < f64::EPSILON);
    }

    #[test]
    fn accumulated_signals_trigger_slowdown() {
        let mut tracker = AbuseTracker::new(test_config());
        // 3 × RateBurst = 0.45 → above slowdown (0.4), below quarantine (0.7)
        for _ in 0..3 {
            tracker.record_signal("user-2", SignalType::RateBurst);
        }
        let (action, score) = tracker.evaluate("user-2");
        assert!(matches!(action, AbuseAction::Slowdown(_)));
        assert!((score - 0.45).abs() < f64::EPSILON);
    }

    #[test]
    fn severe_signals_trigger_quarantine() {
        let mut tracker = AbuseTracker::new(test_config());
        // SensitiveProbe(0.30) + PolicyViolation(0.25) + RateBurst(0.15) = 0.70
        tracker.record_signal("attacker", SignalType::SensitiveProbe);
        tracker.record_signal("attacker", SignalType::PolicyViolation);
        tracker.record_signal("attacker", SignalType::RateBurst);
        let (action, score) = tracker.evaluate("attacker");
        assert!(matches!(action, AbuseAction::Quarantine(_)));
        assert!((score - 0.70).abs() < f64::EPSILON);
    }

    #[test]
    fn score_clamps_to_one() {
        let mut tracker = AbuseTracker::new(test_config());
        for _ in 0..20 {
            tracker.record_signal("spammer", SignalType::SensitiveProbe);
        }
        let (_, score) = tracker.evaluate("spammer");
        assert!((score - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn eviction_respects_cap() {
        let cfg = AbuseConfig {
            max_tracked_actors: 3,
            ..test_config()
        };
        let mut tracker = AbuseTracker::new(cfg);
        for i in 0..5 {
            tracker.record_signal(&format!("actor-{i}"), SignalType::RateBurst);
        }
        assert!(tracker.actors.len() <= 3);
    }

    #[test]
    fn snapshot_returns_all_tracked() {
        let mut tracker = AbuseTracker::new(test_config());
        tracker.record_signal("a", SignalType::RateBurst);
        tracker.record_signal("b", SignalType::PolicyViolation);
        let snap = tracker.snapshot();
        assert_eq!(snap.len(), 2);
    }
}
