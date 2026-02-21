use std::collections::HashMap;
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::format::UnifiedMessage;

pub struct DedupTracker {
    tracked: HashMap<String, Instant>,
    ttl: Duration,
}

impl DedupTracker {
    pub fn new(ttl_seconds: u64) -> Self {
        Self {
            tracked: HashMap::new(),
            ttl: Duration::from_secs(ttl_seconds),
        }
    }

    pub fn fingerprint(model: &str, messages: &[UnifiedMessage]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(model.as_bytes());
        for msg in messages {
            hasher.update(msg.role.as_bytes());
            hasher.update(msg.content.as_bytes());
        }
        format!("{:x}", hasher.finalize())
    }

    /// Returns `true` if the request is unique (not already tracked), `false` if duplicate.
    /// Evicts expired entries before checking.
    pub fn check_and_track(&mut self, fingerprint: &str) -> bool {
        self.evict_expired();

        if self.tracked.contains_key(fingerprint) {
            return false;
        }

        self.tracked.insert(fingerprint.to_string(), Instant::now());
        true
    }

    pub fn release(&mut self, fingerprint: &str) {
        self.tracked.remove(fingerprint);
    }

    fn evict_expired(&mut self) {
        let ttl = self.ttl;
        self.tracked
            .retain(|_, tracked_at| tracked_at.elapsed() < ttl);
    }
}

impl Default for DedupTracker {
    fn default() -> Self {
        Self::new(120)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unique_request_tracked() {
        let mut tracker = DedupTracker::new(120);
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        let fp = DedupTracker::fingerprint("gpt-4o", &msgs);

        assert!(tracker.check_and_track(&fp));
    }

    #[test]
    fn duplicate_detected() {
        let mut tracker = DedupTracker::new(120);
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        let fp = DedupTracker::fingerprint("gpt-4o", &msgs);

        assert!(tracker.check_and_track(&fp));
        assert!(!tracker.check_and_track(&fp));
    }

    #[test]
    fn expired_entries_evicted() {
        let mut tracker = DedupTracker::new(0); // 0s TTL = immediate expiry
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        let fp = DedupTracker::fingerprint("gpt-4o", &msgs);

        assert!(tracker.check_and_track(&fp));
        std::thread::sleep(Duration::from_millis(10));
        assert!(tracker.check_and_track(&fp)); // should be evicted and re-trackable
    }

    #[test]
    fn release_allows_retrack() {
        let mut tracker = DedupTracker::new(120);
        let fp = DedupTracker::fingerprint(
            "claude",
            &[UnifiedMessage {
                role: "user".into(),
                content: "test".into(),
            }],
        );

        assert!(tracker.check_and_track(&fp));
        assert!(!tracker.check_and_track(&fp));
        tracker.release(&fp);
        assert!(tracker.check_and_track(&fp));
    }

    #[test]
    fn different_models_different_fingerprints() {
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "hello".into(),
        }];
        let fp1 = DedupTracker::fingerprint("gpt-4o", &msgs);
        let fp2 = DedupTracker::fingerprint("claude-sonnet", &msgs);
        assert_ne!(fp1, fp2);
    }
}
