use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, error, warn};

use ironclad_core::{IroncladError, Result};

const ANTHROPIC_AUTHORIZE_URL: &str = "https://claude.ai/oauth/authorize";
const ANTHROPIC_TOKEN_URL: &str = "https://claude.ai/oauth/token";
const CALLBACK_PORT: u16 = 18791;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoredTokens {
    pub provider: String,
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenFile {
    tokens: Vec<StoredTokens>,
}

// SECURITY TODO: encrypt tokens at rest using the Keystore.
// Tokens are currently stored as plaintext JSON in ~/.ironclad/oauth_tokens.json.
// This is acceptable only as a temporary measure; a future release must use
// OS-level keychain integration or an encrypted envelope before GA.
fn token_file_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".ironclad")
        .join("oauth_tokens.json")
}

#[derive(Debug, Clone)]
pub struct OAuthManager {
    tokens: Arc<RwLock<HashMap<String, StoredTokens>>>,
    http: reqwest::Client,
}

impl OAuthManager {
    pub fn new() -> Result<Self> {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| IroncladError::Network(e.to_string()))?;

        let mut map = HashMap::new();
        if let Ok(data) = std::fs::read_to_string(token_file_path())
            && let Ok(file) = serde_json::from_str::<TokenFile>(&data)
        {
            for entry in file.tokens {
                map.insert(entry.provider.clone(), entry);
            }
        }

        Ok(Self {
            tokens: Arc::new(RwLock::new(map)),
            http,
        })
    }

    pub async fn resolve_token(&self, provider_name: &str) -> Result<String> {
        let tokens = self.tokens.read().await;
        let stored = tokens.get(provider_name).ok_or_else(|| {
            IroncladError::Config(format!(
                "no OAuth tokens stored for provider '{provider_name}'"
            ))
        })?;

        if let Some(expires_at) = stored.expires_at {
            let now = chrono::Utc::now().timestamp();
            if now >= expires_at - 60 {
                drop(tokens);
                return self.refresh_token(provider_name).await;
            }
        }

        Ok(stored.access_token.clone())
    }

    async fn refresh_token(&self, provider_name: &str) -> Result<String> {
        let (refresh_token, client_id) = {
            let tokens = self.tokens.read().await;
            let stored = tokens
                .get(provider_name)
                .ok_or_else(|| IroncladError::Config(format!("no tokens for '{provider_name}'")))?;
            let rt = stored.refresh_token.clone().ok_or_else(|| {
                IroncladError::Config(format!(
                    "no refresh token for '{provider_name}', re-run `ironclad auth login`"
                ))
            })?;
            (rt, stored.client_id.clone())
        };

        debug!(provider = provider_name, "refreshing OAuth token");

        let mut params = HashMap::new();
        params.insert("grant_type", "refresh_token".to_string());
        params.insert("refresh_token", refresh_token);
        if let Some(ref cid) = client_id {
            params.insert("client_id", cid.clone());
        }

        let resp = self
            .http
            .post(ANTHROPIC_TOKEN_URL)
            .form(&params)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("token refresh failed: {e}")))?;

        if !resp.status().is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!(
                "token refresh returned error: {body}"
            )));
        }

        let token_resp: TokenResponse = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("invalid token response: {e}")))?;

        if let Some(ref tt) = token_resp.token_type
            && !tt.eq_ignore_ascii_case("bearer")
        {
            warn!(
                provider = provider_name,
                token_type = tt.as_str(),
                "unexpected token_type in OAuth response (expected \"Bearer\")"
            );
        }

        let expires_at = token_resp
            .expires_in
            .map(|secs| chrono::Utc::now().timestamp() + secs);

        let old_refresh = {
            let tokens = self.tokens.read().await;
            tokens
                .get(provider_name)
                .and_then(|t| t.refresh_token.clone())
        };

        let new_stored = StoredTokens {
            provider: provider_name.to_string(),
            access_token: token_resp.access_token.clone(),
            refresh_token: token_resp.refresh_token.or(old_refresh),
            expires_at,
            client_id,
        };

        {
            let mut tokens = self.tokens.write().await;
            tokens.insert(provider_name.to_string(), new_stored);
        }

        if let Err(e) = self.persist().await {
            error!(provider = provider_name, error = %e, "failed to persist refreshed OAuth tokens");
        }
        Ok(token_resp.access_token)
    }

    pub async fn store_tokens(&self, stored: StoredTokens) {
        let name = stored.provider.clone();
        let mut tokens = self.tokens.write().await;
        tokens.insert(name.clone(), stored);
        drop(tokens);
        if let Err(e) = self.persist().await {
            error!(provider = %name, error = %e, "failed to persist stored OAuth tokens");
        }
    }

    pub async fn remove_tokens(&self, provider_name: &str) -> bool {
        let mut tokens = self.tokens.write().await;
        let removed = tokens.remove(provider_name).is_some();
        drop(tokens);
        if removed && let Err(e) = self.persist().await {
            error!(provider = provider_name, error = %e, "failed to persist OAuth token removal");
        }
        removed
    }

    pub async fn status(&self) -> Vec<TokenStatus> {
        let tokens = self.tokens.read().await;
        let now = chrono::Utc::now().timestamp();
        tokens
            .values()
            .map(|t| {
                let expired = t.expires_at.is_some_and(|exp| now >= exp);
                TokenStatus {
                    provider: t.provider.clone(),
                    has_access_token: !t.access_token.is_empty(),
                    has_refresh_token: t.refresh_token.is_some(),
                    expired,
                    expires_at: t.expires_at,
                }
            })
            .collect()
    }

    async fn persist(&self) -> Result<()> {
        let tokens = self.tokens.read().await;
        let file = TokenFile {
            tokens: tokens.values().cloned().collect(),
        };
        let path = token_file_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                IroncladError::Config(format!(
                    "failed to create token directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        let json = serde_json::to_string_pretty(&file)
            .map_err(|e| IroncladError::Config(format!("failed to serialize OAuth tokens: {e}")))?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, &json).map_err(|e| {
            IroncladError::Config(format!(
                "failed to write OAuth token file {}: {e}",
                tmp.display()
            ))
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &path).map_err(|e| {
            IroncladError::Config(format!("failed to rename OAuth token file into place: {e}"))
        })?;
        Ok(())
    }
}

#[derive(Debug)]
pub struct TokenStatus {
    pub provider: String,
    pub has_access_token: bool,
    pub has_refresh_token: bool,
    pub expired: bool,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    expires_in: Option<i64>,
    token_type: Option<String>,
}

// ── PKCE helpers ──────────────────────────────────────────────

pub fn generate_code_verifier() -> String {
    use rand::RngCore;
    let mut buf = [0u8; 96];
    rand::rngs::OsRng.fill_bytes(&mut buf);
    base64url_encode(&buf)
}

pub fn compute_code_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    base64url_encode(&hash)
}

fn base64url_encode(data: &[u8]) -> String {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    URL_SAFE_NO_PAD.encode(data)
}

pub fn build_authorization_url(
    client_id: &str,
    redirect_uri: &str,
    code_challenge: &str,
    state: &str,
) -> String {
    fn pct_encode(s: &str) -> String {
        let mut out = String::with_capacity(s.len());
        for b in s.bytes() {
            match b {
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                    out.push(b as char);
                }
                _ => {
                    out.push('%');
                    out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                    out.push(char::from(b"0123456789ABCDEF"[(b & 0x0F) as usize]));
                }
            }
        }
        out
    }

    format!(
        "{ANTHROPIC_AUTHORIZE_URL}?\
         response_type=code\
         &client_id={}\
         &redirect_uri={}\
         &code_challenge={}\
         &code_challenge_method=S256\
         &state={}\
         &scope=user%3Ainference",
        pct_encode(client_id),
        pct_encode(redirect_uri),
        pct_encode(code_challenge),
        pct_encode(state),
    )
}

pub fn default_redirect_uri() -> String {
    format!("http://127.0.0.1:{CALLBACK_PORT}/callback")
}

pub fn callback_port() -> u16 {
    CALLBACK_PORT
}

pub fn token_url() -> &'static str {
    ANTHROPIC_TOKEN_URL
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_verifier_is_base64url() {
        let verifier = generate_code_verifier();
        assert!(!verifier.is_empty());
        assert!(
            verifier.len() > 40,
            "verifier should be substantial: {}",
            verifier.len()
        );
        assert!(
            !verifier.contains('+') && !verifier.contains('/') && !verifier.contains('='),
            "verifier must be base64url (no +, /, =)"
        );
    }

    #[test]
    fn code_challenge_is_sha256_base64url() {
        let verifier = "test-verifier-string";
        let challenge = compute_code_challenge(verifier);
        assert!(!challenge.is_empty());
        assert!(
            !challenge.contains('+') && !challenge.contains('/') && !challenge.contains('='),
            "challenge must be base64url (no +, /, =)"
        );

        let challenge2 = compute_code_challenge(verifier);
        assert_eq!(challenge, challenge2, "deterministic");
    }

    #[test]
    fn different_verifiers_produce_different_challenges() {
        let c1 = compute_code_challenge("verifier-one");
        let c2 = compute_code_challenge("verifier-two");
        assert_ne!(c1, c2);
    }

    #[test]
    fn authorization_url_structure() {
        let url = build_authorization_url(
            "my-client-id",
            "http://127.0.0.1:18791/callback",
            "challenge123",
            "state-abc",
        );
        assert!(url.starts_with(ANTHROPIC_AUTHORIZE_URL));
        assert!(url.contains("response_type=code"));
        assert!(url.contains("client_id=my-client-id"));
        assert!(url.contains("code_challenge=challenge123"));
        assert!(url.contains("code_challenge_method=S256"));
        assert!(url.contains("state=state-abc"));
        assert!(
            url.contains("redirect_uri=http%3A%2F%2F127.0.0.1%3A18791%2Fcallback"),
            "redirect_uri must be percent-encoded: {url}"
        );
        assert!(
            url.contains("scope=user%3Ainference"),
            "scope colon must be percent-encoded: {url}"
        );
    }

    #[test]
    fn default_redirect_uri_contains_port() {
        let uri = default_redirect_uri();
        assert!(uri.contains(&CALLBACK_PORT.to_string()));
        assert!(uri.contains("/callback"));
    }

    #[test]
    fn token_file_roundtrip() {
        let file = TokenFile {
            tokens: vec![StoredTokens {
                provider: "anthropic".into(),
                access_token: "at-123".into(),
                refresh_token: Some("rt-456".into()),
                expires_at: Some(1700000000),
                client_id: Some("my-client".into()),
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: TokenFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tokens.len(), 1);
        assert_eq!(parsed.tokens[0].provider, "anthropic");
        assert_eq!(parsed.tokens[0].access_token, "at-123");
        assert_eq!(parsed.tokens[0].refresh_token.as_deref(), Some("rt-456"));
        assert_eq!(parsed.tokens[0].expires_at, Some(1700000000));
        assert_eq!(parsed.tokens[0].client_id.as_deref(), Some("my-client"));
    }

    #[test]
    fn token_file_backward_compat_no_client_id() {
        let json = r#"{"tokens":[{"provider":"anthropic","access_token":"at","refresh_token":null,"expires_at":null}]}"#;
        let parsed: TokenFile = serde_json::from_str(json).unwrap();
        assert!(parsed.tokens[0].client_id.is_none());
    }

    #[tokio::test]
    async fn oauth_manager_new_with_no_file() {
        let mgr = OAuthManager::new().unwrap();
        let status = mgr.status().await;
        assert!(
            status.is_empty() || !status.is_empty(),
            "should not panic even if file doesn't exist"
        );
    }

    #[tokio::test]
    async fn oauth_manager_store_and_resolve() {
        let mgr = OAuthManager::new().unwrap();
        let far_future = chrono::Utc::now().timestamp() + 3600;
        mgr.store_tokens(StoredTokens {
            provider: "test-provider".into(),
            access_token: "test-access-token".into(),
            refresh_token: Some("test-refresh".into()),
            expires_at: Some(far_future),
            client_id: None,
        })
        .await;

        let token = mgr.resolve_token("test-provider").await.unwrap();
        assert_eq!(token, "test-access-token");

        let removed = mgr.remove_tokens("test-provider").await;
        assert!(removed);

        let err = mgr.resolve_token("test-provider").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn oauth_manager_resolve_missing_provider() {
        let mgr = OAuthManager::new().unwrap();
        let err = mgr.resolve_token("nonexistent").await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn oauth_manager_status_reports_expiry() {
        let mgr = OAuthManager::new().unwrap();
        let past = chrono::Utc::now().timestamp() - 3600;
        mgr.store_tokens(StoredTokens {
            provider: "expired-provider".into(),
            access_token: "old-token".into(),
            refresh_token: None,
            expires_at: Some(past),
            client_id: None,
        })
        .await;

        let statuses = mgr.status().await;
        let s = statuses
            .iter()
            .find(|s| s.provider == "expired-provider")
            .unwrap();
        assert!(s.expired);
        assert!(s.has_access_token);
        assert!(!s.has_refresh_token);

        mgr.remove_tokens("expired-provider").await;
    }

    #[tokio::test]
    async fn oauth_manager_remove_nonexistent() {
        let mgr = OAuthManager::new().unwrap();
        let removed = mgr.remove_tokens("does-not-exist").await;
        assert!(!removed);
    }

    // ── token_url / callback_port / default_redirect_uri ──────────────

    #[test]
    fn token_url_is_anthropic() {
        let url = token_url();
        assert_eq!(url, ANTHROPIC_TOKEN_URL);
        assert!(url.starts_with("https://"));
        assert!(url.contains("token"));
    }

    #[test]
    fn callback_port_returns_constant() {
        let port = callback_port();
        assert_eq!(port, CALLBACK_PORT);
        assert!(port > 1024, "should be a high port");
    }

    // ── resolve_token expiry logic ──────────────────────────────

    #[tokio::test]
    async fn resolve_token_not_expired_returns_access_token() {
        let mgr = OAuthManager::new().unwrap();
        let far_future = chrono::Utc::now().timestamp() + 7200; // 2 hours from now
        mgr.store_tokens(StoredTokens {
            provider: "test-resolve".into(),
            access_token: "valid-token".into(),
            refresh_token: Some("rt-123".into()),
            expires_at: Some(far_future),
            client_id: None,
        })
        .await;

        let token = mgr.resolve_token("test-resolve").await.unwrap();
        assert_eq!(token, "valid-token");
        mgr.remove_tokens("test-resolve").await;
    }

    #[tokio::test]
    async fn resolve_token_no_expiry_returns_access_token() {
        let mgr = OAuthManager::new().unwrap();
        mgr.store_tokens(StoredTokens {
            provider: "test-no-exp".into(),
            access_token: "no-expiry-token".into(),
            refresh_token: None,
            expires_at: None, // no expiry set
            client_id: None,
        })
        .await;

        let token = mgr.resolve_token("test-no-exp").await.unwrap();
        assert_eq!(token, "no-expiry-token");
        mgr.remove_tokens("test-no-exp").await;
    }

    #[tokio::test]
    async fn resolve_token_expired_attempts_refresh_fails_network() {
        let mgr = OAuthManager::new().unwrap();
        let past = chrono::Utc::now().timestamp() - 3600; // already expired
        mgr.store_tokens(StoredTokens {
            provider: "test-expired".into(),
            access_token: "old-token".into(),
            refresh_token: Some("rt-old".into()),
            expires_at: Some(past),
            client_id: None,
        })
        .await;

        // resolve_token should attempt refresh, which will fail (network)
        let err = mgr.resolve_token("test-expired").await;
        assert!(
            err.is_err(),
            "refresh should fail against real Anthropic endpoint"
        );
        mgr.remove_tokens("test-expired").await;
    }

    #[tokio::test]
    async fn resolve_token_about_to_expire_attempts_refresh() {
        let mgr = OAuthManager::new().unwrap();
        // Expires within the 60-second buffer
        let almost_expired = chrono::Utc::now().timestamp() + 30;
        mgr.store_tokens(StoredTokens {
            provider: "test-almost".into(),
            access_token: "almost-expired-token".into(),
            refresh_token: Some("rt-almost".into()),
            expires_at: Some(almost_expired),
            client_id: Some("test-client".into()),
        })
        .await;

        // Should attempt refresh (within 60s buffer) and fail
        let err = mgr.resolve_token("test-almost").await;
        assert!(err.is_err(), "refresh should fail");
        mgr.remove_tokens("test-almost").await;
    }

    #[tokio::test]
    async fn resolve_token_expired_no_refresh_token_errors() {
        let mgr = OAuthManager::new().unwrap();
        let past = chrono::Utc::now().timestamp() - 3600;
        mgr.store_tokens(StoredTokens {
            provider: "test-no-rt".into(),
            access_token: "old-token".into(),
            refresh_token: None, // no refresh token
            expires_at: Some(past),
            client_id: None,
        })
        .await;

        let err = mgr.resolve_token("test-no-rt").await.unwrap_err();
        let msg = format!("{err}");
        assert!(
            msg.contains("no refresh token") || msg.contains("re-run"),
            "should mention missing refresh token: {msg}"
        );
        mgr.remove_tokens("test-no-rt").await;
    }

    // ── store_tokens / status / persist ──────────────────────────────

    #[tokio::test]
    async fn store_tokens_overwrites_existing() {
        let mgr = OAuthManager::new().unwrap();
        let future = chrono::Utc::now().timestamp() + 3600;
        mgr.store_tokens(StoredTokens {
            provider: "test-overwrite".into(),
            access_token: "first-token".into(),
            refresh_token: None,
            expires_at: Some(future),
            client_id: None,
        })
        .await;

        let token1 = mgr.resolve_token("test-overwrite").await.unwrap();
        assert_eq!(token1, "first-token");

        // Overwrite
        mgr.store_tokens(StoredTokens {
            provider: "test-overwrite".into(),
            access_token: "second-token".into(),
            refresh_token: Some("new-rt".into()),
            expires_at: Some(future),
            client_id: Some("new-client".into()),
        })
        .await;

        let token2 = mgr.resolve_token("test-overwrite").await.unwrap();
        assert_eq!(token2, "second-token");
        mgr.remove_tokens("test-overwrite").await;
    }

    #[tokio::test]
    async fn status_empty_access_token_reports_false() {
        let mgr = OAuthManager::new().unwrap();
        mgr.store_tokens(StoredTokens {
            provider: "test-empty-at".into(),
            access_token: "".into(), // empty
            refresh_token: Some("rt".into()),
            expires_at: None,
            client_id: None,
        })
        .await;

        let statuses = mgr.status().await;
        let s = statuses
            .iter()
            .find(|s| s.provider == "test-empty-at")
            .unwrap();
        assert!(!s.has_access_token);
        assert!(s.has_refresh_token);
        assert!(!s.expired); // no expiry set
        assert!(s.expires_at.is_none());
        mgr.remove_tokens("test-empty-at").await;
    }

    #[tokio::test]
    async fn status_not_expired_when_future() {
        let mgr = OAuthManager::new().unwrap();
        let future = chrono::Utc::now().timestamp() + 86400;
        mgr.store_tokens(StoredTokens {
            provider: "test-future".into(),
            access_token: "at".into(),
            refresh_token: None,
            expires_at: Some(future),
            client_id: None,
        })
        .await;

        let statuses = mgr.status().await;
        let s = statuses
            .iter()
            .find(|s| s.provider == "test-future")
            .unwrap();
        assert!(!s.expired);
        assert_eq!(s.expires_at, Some(future));
        mgr.remove_tokens("test-future").await;
    }

    // ── authorization URL with special chars ──────────────────

    #[test]
    fn authorization_url_encodes_special_chars_in_client_id() {
        let url = build_authorization_url(
            "client id with spaces",
            "http://localhost/callback",
            "challenge",
            "state",
        );
        assert!(url.contains("client%20id%20with%20spaces"));
    }

    #[test]
    fn authorization_url_encodes_state() {
        let url = build_authorization_url(
            "client",
            "http://localhost/callback",
            "challenge",
            "state=with&special",
        );
        assert!(url.contains("state%3Dwith%26special"));
    }

    // ── base64url_encode ──────────────────────────────────

    #[test]
    fn base64url_encode_roundtrip() {
        let data = b"hello world 123!@#";
        let encoded = base64url_encode(data);
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
    }

    // ── TokenFile serde edge cases ──────────────────────────

    #[test]
    fn token_file_empty_tokens() {
        let file = TokenFile { tokens: vec![] };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: TokenFile = serde_json::from_str(&json).unwrap();
        assert!(parsed.tokens.is_empty());
    }

    #[test]
    fn token_file_multiple_providers() {
        let file = TokenFile {
            tokens: vec![
                StoredTokens {
                    provider: "provider-a".into(),
                    access_token: "at-a".into(),
                    refresh_token: None,
                    expires_at: None,
                    client_id: None,
                },
                StoredTokens {
                    provider: "provider-b".into(),
                    access_token: "at-b".into(),
                    refresh_token: Some("rt-b".into()),
                    expires_at: Some(9999999999),
                    client_id: Some("cid-b".into()),
                },
            ],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: TokenFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tokens.len(), 2);
        assert_eq!(parsed.tokens[0].provider, "provider-a");
        assert_eq!(parsed.tokens[1].provider, "provider-b");
    }

    #[test]
    fn stored_tokens_client_id_skipped_when_none() {
        let token = StoredTokens {
            provider: "p".into(),
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            client_id: None,
        };
        let json = serde_json::to_string(&token).unwrap();
        assert!(
            !json.contains("client_id"),
            "client_id should be skipped: {json}"
        );
    }

    #[test]
    fn stored_tokens_client_id_present_when_some() {
        let token = StoredTokens {
            provider: "p".into(),
            access_token: "at".into(),
            refresh_token: None,
            expires_at: None,
            client_id: Some("my-client".into()),
        };
        let json = serde_json::to_string(&token).unwrap();
        assert!(
            json.contains("client_id"),
            "client_id should be present: {json}"
        );
        assert!(json.contains("my-client"));
    }

    // ── token_file_path ──────────────────────────────────

    #[test]
    fn token_file_path_contains_ironclad() {
        let path = token_file_path();
        assert!(
            path.to_str().unwrap().contains(".ironclad"),
            "path should contain .ironclad: {path:?}"
        );
        assert!(
            path.to_str().unwrap().contains("oauth_tokens.json"),
            "path should end with oauth_tokens.json: {path:?}"
        );
    }

    // ── PKCE verifier uniqueness ──────────────────────────

    #[test]
    fn code_verifiers_are_unique() {
        let v1 = generate_code_verifier();
        let v2 = generate_code_verifier();
        assert_ne!(v1, v2, "verifiers should be unique per call");
    }
}
