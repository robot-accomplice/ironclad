use std::collections::{HashMap, VecDeque};
use std::time::Instant;

use aes_gcm::{
    Aes256Gcm,
    aead::{Aead, AeadCore, KeyInit},
};
use chrono::{DateTime, Utc};
use hkdf::Hkdf;
use ironclad_core::{IroncladError, Result, config::A2aConfig};
use serde_json::{Value, json};
use sha2::Sha256;
use std::fmt::Write as _;
use tracing::debug;
use x25519_dalek::{EphemeralSecret, PublicKey};

pub struct A2aSession {
    pub peer_did: String,
    /// ECDH-derived session key for AES-256-GCM; set after key agreement.
    ///
    /// **Callers MUST check `session_key.is_some()` (or use `is_established`)
    /// before calling `encrypt_message` / `decrypt_message`.**  Passing a
    /// zero-initialised key silently produces insecure ciphertext.
    pub session_key: Option<[u8; 32]>,
    pub established_at: DateTime<Utc>,
    pub last_activity: DateTime<Utc>,
}

impl A2aSession {
    /// Returns `true` when the ECDH key agreement has completed and the session
    /// is ready for encrypted communication.
    pub fn is_established(&self) -> bool {
        self.session_key.is_some()
    }
}

const MAX_A2A_SESSIONS: usize = 256;
/// Maximum number of entries in `rate_windows` before stale-entry eviction.
const MAX_RATE_WINDOW_ENTRIES: usize = 1000;

pub struct A2aProtocol {
    pub config: A2aConfig,
    sessions: HashMap<String, A2aSession>,
    rate_windows: HashMap<String, VecDeque<Instant>>,
    seen_nonces: HashMap<String, Instant>,
}

impl A2aProtocol {
    pub fn new(config: A2aConfig) -> Self {
        Self {
            config,
            sessions: HashMap::new(),
            rate_windows: HashMap::new(),
            seen_nonces: HashMap::new(),
        }
    }

    fn evict_expired_sessions(&mut self) {
        let timeout = chrono::Duration::seconds(self.config.session_timeout_seconds as i64);
        let cutoff = Utc::now() - timeout;
        self.sessions.retain(|_, s| s.last_activity > cutoff);
    }

    pub fn insert_session(&mut self, id: String, session: A2aSession) {
        self.evict_expired_sessions();
        if self.sessions.len() >= MAX_A2A_SESSIONS
            && let Some(oldest_key) = self
                .sessions
                .iter()
                .min_by_key(|(_, s)| s.last_activity)
                .map(|(k, _)| k.clone())
        {
            self.sessions.remove(&oldest_key);
        }
        self.sessions.insert(id, session);
    }

    pub fn get_session(&self, id: &str) -> Option<&A2aSession> {
        self.sessions.get(id)
    }

    pub fn get_session_mut(&mut self, id: &str) -> Option<&mut A2aSession> {
        self.sessions.get_mut(id)
    }

    pub fn remove_session(&mut self, id: &str) -> Option<A2aSession> {
        self.sessions.remove(id)
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
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

        // Evict stale rate-window entries to prevent unbounded growth.
        if self.rate_windows.len() > MAX_RATE_WINDOW_ENTRIES {
            let stale_threshold = std::time::Duration::from_secs(3600);
            self.rate_windows.retain(|_, ts| {
                ts.back()
                    .is_some_and(|&last| now.duration_since(last) < stale_threshold)
            });
        }

        let timestamps = self.rate_windows.entry(peer_did.to_string()).or_default();

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

    /// Reject duplicate nonces to prevent replay attacks.
    /// Evicts expired entries before checking.
    pub fn register_nonce(&mut self, nonce: &str) -> Result<()> {
        let now = Instant::now();
        let ttl_seconds = if self.config.nonce_ttl_seconds > 0 {
            self.config.nonce_ttl_seconds
        } else {
            2 * self.config.session_timeout_seconds
        };
        let ttl = std::time::Duration::from_secs(ttl_seconds);

        self.seen_nonces
            .retain(|_, inserted_at| now.duration_since(*inserted_at) < ttl);

        if self.seen_nonces.contains_key(nonce) {
            return Err(IroncladError::A2a(
                "duplicate nonce: replay rejected".into(),
            ));
        }

        self.seen_nonces.insert(nonce.to_string(), now);
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

    /// Generate an ephemeral X25519 keypair for ECDH.
    pub fn generate_keypair() -> (EphemeralSecret, PublicKey) {
        let secret = EphemeralSecret::random_from_rng(rand::rngs::OsRng);
        let public = PublicKey::from(&secret);
        (secret, public)
    }

    /// Derive a 32-byte session key from X25519 ECDH shared secret using HKDF-SHA256.
    pub fn derive_session_key(our_secret: EphemeralSecret, their_public: &PublicKey) -> [u8; 32] {
        let shared = our_secret.diffie_hellman(their_public);
        let h = Hkdf::<Sha256>::new(None, shared.as_bytes());
        let mut key = [0u8; 32];
        // SAFETY: HKDF-SHA256 expand to 32 bytes cannot fail per RFC 5869
        h.expand(b"ironclad-a2a-session", &mut key)
            .expect("HKDF expand to 32 bytes");
        key
    }

    /// Encrypt plaintext with AES-256-GCM; returns 12-byte nonce || ciphertext (including tag).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironclad_channels::a2a::A2aProtocol;
    ///
    /// let key = [0x42u8; 32];
    /// let plaintext = b"secret data";
    /// let ciphertext = A2aProtocol::encrypt_message(&key, plaintext).unwrap();
    /// let decrypted = A2aProtocol::decrypt_message(&key, &ciphertext).unwrap();
    /// assert_eq!(decrypted, plaintext);
    /// ```
    pub fn encrypt_message(key: &[u8; 32], plaintext: &[u8]) -> Result<Vec<u8>> {
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| IroncladError::A2a(format!("AES-GCM key init: {e}")))?;
        let nonce = Aes256Gcm::generate_nonce(rand::rngs::OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| IroncladError::A2a(format!("AES-GCM encrypt: {e}")))?;
        let mut out = nonce.to_vec();
        out.extend_from_slice(&ciphertext);
        Ok(out)
    }

    /// Decrypt ciphertext (format: 12-byte nonce || ciphertext); returns plaintext.
    pub fn decrypt_message(key: &[u8; 32], ciphertext: &[u8]) -> Result<Vec<u8>> {
        if ciphertext.len() < 12 {
            return Err(IroncladError::A2a("ciphertext too short for nonce".into()));
        }
        let (nonce_bytes, ct) = ciphertext.split_at(12);
        let cipher = Aes256Gcm::new_from_slice(key)
            .map_err(|e| IroncladError::A2a(format!("AES-GCM key init: {e}")))?;
        let nonce = aes_gcm::Nonce::from_slice(nonce_bytes);
        cipher
            .decrypt(nonce, ct)
            .map_err(|e| IroncladError::A2a(format!("AES-GCM decrypt: {e}")))
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

    #[test]
    fn key_agreement_produces_matching_session_keys() {
        let (secret_a, public_a) = A2aProtocol::generate_keypair();
        let (secret_b, public_b) = A2aProtocol::generate_keypair();

        let key_a = A2aProtocol::derive_session_key(secret_a, &public_b);
        let key_b = A2aProtocol::derive_session_key(secret_b, &public_a);

        assert_eq!(key_a, key_b, "ECDH session keys must match");
    }

    #[test]
    fn aes256gcm_encrypt_decrypt_roundtrip() {
        let key = [0u8; 32];
        let plaintext = b"hello a2a";
        let ciphertext = A2aProtocol::encrypt_message(&key, plaintext).expect("encrypt");
        assert!(ciphertext.len() > plaintext.len());
        let decrypted = A2aProtocol::decrypt_message(&key, &ciphertext).expect("decrypt");
        assert_eq!(decrypted.as_slice(), plaintext);
    }

    #[test]
    fn tampered_ciphertext_fails_decryption() {
        let key = [0u8; 32];
        let plaintext = b"secret";
        let mut ciphertext = A2aProtocol::encrypt_message(&key, plaintext).expect("encrypt");
        // Tamper with the ciphertext (after the 12-byte nonce).
        if ciphertext.len() > 20 {
            ciphertext[20] = ciphertext[20].wrapping_add(1);
        }
        let err = A2aProtocol::decrypt_message(&key, &ciphertext).unwrap_err();
        assert!(err.to_string().contains("decrypt"));
    }

    #[test]
    fn decrypt_ciphertext_too_short() {
        let key = [0u8; 32];
        let err = A2aProtocol::decrypt_message(&key, &[0u8; 5]).unwrap_err();
        assert!(err.to_string().contains("too short"));
    }

    #[test]
    fn session_is_established() {
        let session = A2aSession {
            peer_did: "did:test:1".into(),
            session_key: Some([42u8; 32]),
            established_at: Utc::now(),
            last_activity: Utc::now(),
        };
        assert!(session.is_established());
    }

    #[test]
    fn session_not_established_without_key() {
        let session = A2aSession {
            peer_did: "did:test:2".into(),
            session_key: None,
            established_at: Utc::now(),
            last_activity: Utc::now(),
        };
        assert!(!session.is_established());
    }

    #[test]
    fn insert_and_get_session() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        let session = A2aSession {
            peer_did: "did:test:abc".into(),
            session_key: None,
            established_at: Utc::now(),
            last_activity: Utc::now(),
        };
        proto.insert_session("s1".into(), session);
        assert_eq!(proto.session_count(), 1);
        assert!(proto.get_session("s1").is_some());
        assert_eq!(proto.get_session("s1").unwrap().peer_did, "did:test:abc");
    }

    #[test]
    fn get_session_mut_updates() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        let session = A2aSession {
            peer_did: "did:test:mut".into(),
            session_key: None,
            established_at: Utc::now(),
            last_activity: Utc::now(),
        };
        proto.insert_session("s1".into(), session);
        let s = proto.get_session_mut("s1").unwrap();
        s.session_key = Some([99u8; 32]);
        assert!(proto.get_session("s1").unwrap().is_established());
    }

    #[test]
    fn remove_session() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        let session = A2aSession {
            peer_did: "did:test:rm".into(),
            session_key: None,
            established_at: Utc::now(),
            last_activity: Utc::now(),
        };
        proto.insert_session("rm1".into(), session);
        assert_eq!(proto.session_count(), 1);
        let removed = proto.remove_session("rm1");
        assert!(removed.is_some());
        assert_eq!(proto.session_count(), 0);
    }

    #[test]
    fn remove_session_nonexistent() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        assert!(proto.remove_session("nope").is_none());
    }

    #[test]
    fn get_session_nonexistent() {
        let proto = A2aProtocol::new(A2aConfig::default());
        assert!(proto.get_session("nope").is_none());
    }

    #[test]
    fn insert_session_evicts_oldest_when_full() {
        let mut proto = A2aProtocol::new(A2aConfig {
            session_timeout_seconds: 99999,
            ..Default::default()
        });
        // Fill to MAX_A2A_SESSIONS
        for i in 0..MAX_A2A_SESSIONS {
            let session = A2aSession {
                peer_did: format!("did:test:{}", i),
                session_key: None,
                established_at: Utc::now(),
                last_activity: Utc::now() + chrono::Duration::seconds(i as i64),
            };
            proto.insert_session(format!("s{}", i), session);
        }
        assert_eq!(proto.session_count(), MAX_A2A_SESSIONS);

        // Insert one more -- should evict oldest
        let new_session = A2aSession {
            peer_did: "did:test:new".into(),
            session_key: None,
            established_at: Utc::now(),
            last_activity: Utc::now() + chrono::Duration::seconds(99999),
        };
        proto.insert_session("new_one".into(), new_session);
        assert!(proto.session_count() <= MAX_A2A_SESSIONS);
        assert!(proto.get_session("new_one").is_some());
    }

    #[test]
    fn validate_message_size_at_boundary() {
        let proto = A2aProtocol::new(A2aConfig {
            max_message_size: 10,
            ..Default::default()
        });
        assert!(proto.validate_message_size(&[0u8; 10]).is_ok());
        assert!(proto.validate_message_size(&[0u8; 11]).is_err());
        assert!(proto.validate_message_size(&[0u8; 0]).is_ok());
    }

    #[test]
    fn verify_hello_missing_type() {
        let hello = json!({"did": "did:test:1", "nonce": "aa"});
        let err = A2aProtocol::verify_hello(&hello).unwrap_err();
        assert!(err.to_string().contains("type"));
    }

    #[test]
    fn verify_hello_wrong_type() {
        let hello = json!({"type": "wrong_type", "did": "did:test:1", "nonce": "aa"});
        let err = A2aProtocol::verify_hello(&hello).unwrap_err();
        assert!(err.to_string().contains("unexpected message type"));
    }

    #[test]
    fn verify_hello_empty_did() {
        let hello = json!({"type": "a2a_hello", "did": "", "nonce": "aa"});
        let err = A2aProtocol::verify_hello(&hello).unwrap_err();
        assert!(err.to_string().contains("empty DID"));
    }

    #[test]
    fn verify_hello_missing_nonce() {
        let hello = json!({"type": "a2a_hello", "did": "did:test:1"});
        let err = A2aProtocol::verify_hello(&hello).unwrap_err();
        assert!(err.to_string().contains("nonce"));
    }

    #[test]
    fn verify_hello_missing_did() {
        let hello = json!({"type": "a2a_hello", "nonce": "aa"});
        let err = A2aProtocol::verify_hello(&hello).unwrap_err();
        assert!(err.to_string().contains("did"));
    }

    #[test]
    fn generate_hello_nonce_hex_encoding() {
        let nonce = &[0xde, 0xad, 0xbe, 0xef];
        let hello = A2aProtocol::generate_hello("did:test:hex", nonce);
        let nonce_str = hello["nonce"].as_str().unwrap();
        assert_eq!(nonce_str, "deadbeef");
    }

    #[test]
    fn generate_hello_has_timestamp() {
        let hello = A2aProtocol::generate_hello("did:test:ts", &[1, 2, 3]);
        assert!(hello.get("timestamp").is_some());
        assert!(hello["timestamp"].is_number());
    }

    #[test]
    fn encrypt_decrypt_empty_plaintext() {
        let key = [1u8; 32];
        let ciphertext = A2aProtocol::encrypt_message(&key, b"").unwrap();
        let decrypted = A2aProtocol::decrypt_message(&key, &ciphertext).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn encrypt_decrypt_large_plaintext() {
        let key = [2u8; 32];
        let plaintext = vec![0xABu8; 10_000];
        let ciphertext = A2aProtocol::encrypt_message(&key, &plaintext).unwrap();
        let decrypted = A2aProtocol::decrypt_message(&key, &ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn wrong_key_fails_decryption() {
        let key1 = [1u8; 32];
        let key2 = [2u8; 32];
        let ciphertext = A2aProtocol::encrypt_message(&key1, b"secret").unwrap();
        assert!(A2aProtocol::decrypt_message(&key2, &ciphertext).is_err());
    }

    #[test]
    fn timestamp_validation_at_boundary() {
        let now = Utc::now().timestamp();
        assert!(A2aProtocol::validate_timestamp(now - 30, 30).is_ok());
        assert!(A2aProtocol::validate_timestamp(now + 30, 30).is_ok());
        assert!(A2aProtocol::validate_timestamp(now - 31, 30).is_err());
        assert!(A2aProtocol::validate_timestamp(now + 31, 30).is_err());
    }

    #[test]
    fn rate_limit_stale_entry_eviction() {
        let mut proto = A2aProtocol::new(A2aConfig {
            rate_limit_per_peer: 100,
            ..Default::default()
        });
        // Populate many rate window entries to trigger eviction path
        for i in 0..MAX_RATE_WINDOW_ENTRIES + 10 {
            let _ = proto.check_rate_limit(&format!("peer-{}", i));
        }
        // Should still work (entries were evicted or managed)
        assert!(proto.check_rate_limit("fresh-peer").is_ok());
    }

    #[test]
    fn session_count_empty() {
        let proto = A2aProtocol::new(A2aConfig::default());
        assert_eq!(proto.session_count(), 0);
    }

    #[test]
    fn validate_message_size_error_message() {
        let proto = A2aProtocol::new(A2aConfig {
            max_message_size: 5,
            ..Default::default()
        });
        let err = proto.validate_message_size(&[0u8; 10]).unwrap_err();
        assert!(err.to_string().contains("10"));
        assert!(err.to_string().contains("5"));
    }

    #[test]
    fn nonce_replay_rejected() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        assert!(proto.register_nonce("nonce-abc").is_ok());
        let err = proto.register_nonce("nonce-abc").unwrap_err();
        assert!(err.to_string().contains("duplicate nonce: replay rejected"));
    }

    #[test]
    fn nonce_accepted_after_manual_expiry() {
        let mut proto = A2aProtocol::new(A2aConfig::default());
        assert!(proto.register_nonce("nonce-xyz").is_ok());
        proto.seen_nonces.clear();
        assert!(proto.register_nonce("nonce-xyz").is_ok());
    }

    #[test]
    fn nonce_eviction_bounds_memory() {
        let mut proto = A2aProtocol::new(A2aConfig {
            nonce_ttl_seconds: 0,
            session_timeout_seconds: 0,
            ..Default::default()
        });
        for i in 0..500 {
            let _ = proto.register_nonce(&format!("n-{i}"));
        }
        // With a TTL of 0, eviction runs on every call and removes expired
        // entries. After one more call the map should contain at most 1 entry
        // (the freshly inserted one).
        let _ = proto.register_nonce("final");
        assert!(proto.seen_nonces.len() <= 1);
    }
}
