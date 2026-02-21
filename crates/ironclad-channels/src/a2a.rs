use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use chrono::{DateTime, Utc};
use ironclad_core::{IroncladError, Result, config::A2aConfig};
use serde_json::{Value, json};
use std::fmt::Write as _;
use tracing::debug;

pub struct A2aSession {
    pub peer_did: String,
    pub session_key: Vec<u8>,
    pub established_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}

pub struct A2aProtocol {
    pub config: A2aConfig,
    pub sessions: HashMap<String, A2aSession>,
    rate_windows: HashMap<String, VecDeque<Instant>>,
}

impl A2aProtocol {
    pub fn new(config: A2aConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
            rate_windows: HashMap::new(),
        }
    }

    /// Reject messages that exceed the configured maximum size.
    pub fn validate_message_size(&self, msg: &[u8]) -> Result<()> {
        if msg.len() > self.config.max_message_size {
            return Err(IroncladError::A2a(format!(
                "message size {} exceeds max {}",
                msg.len(),
                self.config.max_message_size
            )));
        }
        Ok(())
    }

    /// Sliding-window rate limiter per peer DID.
    /// Allows up to `rate_limit_per_peer` requests per 60-second window.
    pub fn check_rate_limit(&mut self, peer_did: &str) -> Result<()> {
        let limit = self.config.rate_limit_per_peer;
        if limit == 0 {
            return Ok(());
        }

        let now = Instant::now();
        let window = std::time::Duration::from_secs(60);

        let timestamps = self
            .rate_windows
            .entry(peer_did.to_string())
            .or_default();

        while let Some(&front) = timestamps.front() {
            if now.duration_since(front) > window {
                timestamps.pop_front();
            } else {
                break;
            }
        }

        if timestamps.len() >= limit as usize {
            debug!(peer = %peer_did, count = timestamps.len(), limit, "rate limit exceeded");
            return Err(IroncladError::A2a(format!(
                "rate limit exceeded for peer {peer_did}: {limit} requests per 60s"
            )));
        }

        timestamps.push_back(now);
        Ok(())
    }

    /// Reject timestamps that drift too far from the current time.
    pub fn validate_timestamp(timestamp: i64, max_drift_seconds: u64) -> Result<()> {
        let now = Utc::now().timestamp();
        let drift = (now - timestamp).unsigned_abs();
        if drift > max_drift_seconds {
            return Err(IroncladError::A2a(format!(
                "timestamp drift {drift}s exceeds max {max_drift_seconds}s"
            )));
        }
        Ok(())
    }

    /// Create a hello handshake message for A2A session establishment.
    pub fn generate_hello(our_did: &str, nonce: &[u8]) -> Value {
        json!({
            "type": "a2a_hello",
            "did": our_did,
            "nonce": nonce.iter().fold(String::new(), |mut s, b| { let _ = write!(s, "{b:02x}"); s }),
            "timestamp": Utc::now().timestamp(),
        })
    }

    /// Extract and validate the peer DID from a hello handshake message.
    pub fn verify_hello(hello: &Value) -> Result<String> {
        let msg_type = hello
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IroncladError::A2a("missing 'type' in hello".into()))?;

        if msg_type != "a2a_hello" {
            return Err(IroncladError::A2a(format!(
                "unexpected message type: {msg_type}"
            )));
        }

        let did = hello
            .get("did")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IroncladError::A2a("missing 'did' in hello".into()))?;

        if did.is_empty() {
            return Err(IroncladError::A2a("empty DID in hello".into()));
        }

        hello
            .get("nonce")
            .and_then(|v| v.as_str())
            .ok_or_else(|| IroncladError::A2a("missing 'nonce' in hello".into()))?;

        Ok(did.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_size_validation() {
        let proto = A2aProtocol::new(A2aConfig {
            max_message_size: 100,
            ..Default::default()
        });

        assert!(proto.validate_message_size(&[0u8; 50]).is_ok());
        assert!(proto.validate_message_size(&[0u8; 100]).is_ok());
        assert!(proto.validate_message_size(&[0u8; 101]).is_err());
    }

    #[test]
    fn timestamp_validation_fresh_and_stale() {
        let now = Utc::now().timestamp();

        assert!(A2aProtocol::validate_timestamp(now, 30).is_ok());
        assert!(A2aProtocol::validate_timestamp(now - 10, 30).is_ok());

        assert!(A2aProtocol::validate_timestamp(now - 300, 30).is_err());
        assert!(A2aProtocol::validate_timestamp(now + 300, 30).is_err());
    }

    #[test]
    fn hello_generation_and_verification() {
        let nonce = b"random_nonce_bytes";
        let hello = A2aProtocol::generate_hello("did:ironclad:abc123", nonce);

        assert_eq!(hello["type"], "a2a_hello");
        assert_eq!(hello["did"], "did:ironclad:abc123");
        assert!(hello.get("nonce").is_some());
        assert!(hello.get("timestamp").is_some());

        let peer_did = A2aProtocol::verify_hello(&hello).unwrap();
        assert_eq!(peer_did, "did:ironclad:abc123");

        let bad_hello = json!({"type": "wrong", "did": "x", "nonce": "aa"});
        assert!(A2aProtocol::verify_hello(&bad_hello).is_err());

        let missing_did = json!({"type": "a2a_hello", "nonce": "aa"});
        assert!(A2aProtocol::verify_hello(&missing_did).is_err());
    }

    #[test]
    fn rate_limit_allows_within_threshold() {
        let mut proto = A2aProtocol::new(A2aConfig {
            rate_limit_per_peer: 3,
            ..Default::default()
        });

        assert!(proto.check_rate_limit("peer-1").is_ok());
        assert!(proto.check_rate_limit("peer-1").is_ok());
        assert!(proto.check_rate_limit("peer-1").is_ok());
    }

    #[test]
    fn rate_limit_blocks_excess() {
        let mut proto = A2aProtocol::new(A2aConfig {
            rate_limit_per_peer: 2,
            ..Default::default()
        });

        assert!(proto.check_rate_limit("peer-1").is_ok());
        assert!(proto.check_rate_limit("peer-1").is_ok());
        let err = proto.check_rate_limit("peer-1").unwrap_err();
        assert!(err.to_string().contains("rate limit exceeded"));
    }

    #[test]
    fn rate_limit_per_peer_isolation() {
        let mut proto = A2aProtocol::new(A2aConfig {
            rate_limit_per_peer: 1,
            ..Default::default()
        });

        assert!(proto.check_rate_limit("peer-a").is_ok());
        assert!(proto.check_rate_limit("peer-b").is_ok());

        assert!(proto.check_rate_limit("peer-a").is_err());
        assert!(proto.check_rate_limit("peer-b").is_err());
    }

    #[test]
    fn rate_limit_zero_means_unlimited() {
        let mut proto = A2aProtocol::new(A2aConfig {
            rate_limit_per_peer: 0,
            ..Default::default()
        });

        for _ in 0..100 {
            assert!(proto.check_rate_limit("peer-1").is_ok());
        }
    }
}
