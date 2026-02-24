use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct ProviderCapacityStats {
    pub tpm_limit: Option<u64>,
    pub rpm_limit: Option<u64>,
    pub tokens_used: u64,
    pub requests_used: u64,
    pub token_utilization: f64,
    pub request_utilization: f64,
    pub headroom: f64,
    pub near_capacity: bool,
}

/// Tracks per-provider token and request throughput using a sliding window.
#[derive(Debug)]
pub struct CapacityTracker {
    providers: Arc<Mutex<HashMap<String, ProviderCapacity>>>,
    window: Duration,
}

#[derive(Debug)]
struct ProviderCapacity {
    tpm_limit: Option<u64>,
    rpm_limit: Option<u64>,
    token_events: Vec<(Instant, u64)>,
    request_events: Vec<Instant>,
}

impl ProviderCapacity {
    fn new(tpm_limit: Option<u64>, rpm_limit: Option<u64>) -> Self {
        Self {
            tpm_limit,
            rpm_limit,
            token_events: Vec::new(),
            request_events: Vec::new(),
        }
    }

    fn prune(&mut self, cutoff: Instant) {
        self.token_events.retain(|(t, _)| *t >= cutoff);
        self.request_events.retain(|t| *t >= cutoff);
    }

    fn tokens_in_window(&self, cutoff: Instant) -> u64 {
        self.token_events
            .iter()
            .filter(|(t, _)| *t >= cutoff)
            .map(|(_, count)| *count)
            .sum()
    }

    fn requests_in_window(&self, cutoff: Instant) -> u64 {
        self.request_events.iter().filter(|t| **t >= cutoff).count() as u64
    }
}

impl CapacityTracker {
    pub fn new(window_seconds: u64) -> Self {
        Self {
            providers: Arc::new(Mutex::new(HashMap::new())),
            window: Duration::from_secs(window_seconds),
        }
    }

    /// Register a provider with its capacity limits.
    pub fn register(&self, name: &str, tpm_limit: Option<u64>, rpm_limit: Option<u64>) {
        let mut providers = self.providers.lock().expect("mutex poisoned");
        providers.insert(
            name.to_string(),
            ProviderCapacity::new(tpm_limit, rpm_limit),
        );
    }

    /// Record a completed request with token usage.
    pub fn record(&self, provider: &str, tokens: u64) {
        let mut providers = self.providers.lock().expect("mutex poisoned");
        if let Some(cap) = providers.get_mut(provider) {
            let now = Instant::now();
            cap.token_events.push((now, tokens));
            cap.request_events.push(now);
            let cutoff = now - self.window;
            cap.prune(cutoff);
        }
    }

    /// Returns a headroom score from 0.0 (saturated) to 1.0 (idle) for a provider.
    pub fn headroom(&self, provider: &str) -> f64 {
        let providers = self.providers.lock().expect("mutex poisoned");
        let cap = match providers.get(provider) {
            Some(c) => c,
            None => return 1.0,
        };

        let cutoff = Instant::now() - self.window;

        let tpm_headroom = match cap.tpm_limit {
            Some(limit) if limit > 0 => {
                let used = cap.tokens_in_window(cutoff);
                1.0 - (used as f64 / limit as f64).min(1.0)
            }
            _ => 1.0,
        };

        let rpm_headroom = match cap.rpm_limit {
            Some(limit) if limit > 0 => {
                let used = cap.requests_in_window(cutoff);
                1.0 - (used as f64 / limit as f64).min(1.0)
            }
            _ => 1.0,
        };

        tpm_headroom.min(rpm_headroom)
    }

    /// Returns true if a provider is above 90% utilization.
    pub fn is_near_capacity(&self, provider: &str) -> bool {
        self.headroom(provider) < 0.1
    }

    /// Returns true when a provider is under sustained load pressure.
    ///
    /// Sustained pressure means utilization is high and non-trivial traffic has
    /// been observed in the active window (avoids tripping on sparse samples).
    pub fn is_sustained_hot(&self, provider: &str) -> bool {
        self.stats(provider).is_some_and(|s| {
            let high_pressure = s.token_utilization >= 0.9 || s.request_utilization >= 0.9;
            let enough_samples = s.requests_used >= 3 || s.tokens_used >= 1024;
            high_pressure && enough_samples
        })
    }

    pub fn stats(&self, provider: &str) -> Option<ProviderCapacityStats> {
        let mut providers = self.providers.lock().expect("mutex poisoned");
        let cap = providers.get_mut(provider)?;
        let cutoff = Instant::now() - self.window;
        cap.prune(cutoff);
        let tokens_used = cap.tokens_in_window(cutoff);
        let requests_used = cap.requests_in_window(cutoff);
        let token_utilization = match cap.tpm_limit {
            Some(limit) if limit > 0 => (tokens_used as f64 / limit as f64).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let request_utilization = match cap.rpm_limit {
            Some(limit) if limit > 0 => (requests_used as f64 / limit as f64).clamp(0.0, 1.0),
            _ => 0.0,
        };
        let headroom = (1.0 - token_utilization)
            .min(1.0 - request_utilization)
            .clamp(0.0, 1.0);
        Some(ProviderCapacityStats {
            tpm_limit: cap.tpm_limit,
            rpm_limit: cap.rpm_limit,
            tokens_used,
            requests_used,
            token_utilization,
            request_utilization,
            headroom,
            near_capacity: headroom < 0.1,
        })
    }

    pub fn list_stats(&self) -> Vec<(String, ProviderCapacityStats)> {
        let names: Vec<String> = {
            let providers = self.providers.lock().expect("mutex poisoned");
            providers.keys().cloned().collect()
        };
        names
            .into_iter()
            .filter_map(|name| self.stats(&name).map(|stats| (name, stats)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_tracker_has_full_headroom() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(100_000), Some(60));
        assert_eq!(tracker.headroom("openai"), 1.0);
    }

    #[test]
    fn unknown_provider_full_headroom() {
        let tracker = CapacityTracker::new(60);
        assert_eq!(tracker.headroom("unknown"), 1.0);
    }

    #[test]
    fn record_reduces_headroom() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(1000), Some(100));
        tracker.record("openai", 500);
        let h = tracker.headroom("openai");
        assert!(h < 1.0);
        assert!(h > 0.0);
        assert!((h - 0.5).abs() < 0.01);
    }

    #[test]
    fn saturated_provider_zero_headroom() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(100), Some(10));
        for _ in 0..10 {
            tracker.record("openai", 10);
        }
        let h = tracker.headroom("openai");
        assert!(h <= 0.01, "should be near zero: {h}");
    }

    #[test]
    fn no_limits_means_full_headroom() {
        let tracker = CapacityTracker::new(60);
        tracker.register("local", None, None);
        tracker.record("local", 999_999);
        assert_eq!(tracker.headroom("local"), 1.0);
    }

    #[test]
    fn is_near_capacity_true_when_saturated() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(100), None);
        tracker.record("openai", 95);
        assert!(tracker.is_near_capacity("openai"));
    }

    #[test]
    fn is_near_capacity_false_when_idle() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(100_000), None);
        tracker.record("openai", 100);
        assert!(!tracker.is_near_capacity("openai"));
    }

    #[test]
    fn rpm_limits_headroom() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", None, Some(10));
        for _ in 0..5 {
            tracker.record("openai", 0);
        }
        let h = tracker.headroom("openai");
        assert!(
            (h - 0.5).abs() < 0.01,
            "5/10 requests = 50% used, headroom should be ~0.5: {h}"
        );
    }

    #[test]
    fn min_of_tpm_and_rpm() {
        let tracker = CapacityTracker::new(60);
        tracker.register("openai", Some(1000), Some(10));
        for _ in 0..9 {
            tracker.record("openai", 11);
        }
        let h = tracker.headroom("openai");
        assert!(h < 0.2, "should be constrained by RPM: {h}");
    }
}
