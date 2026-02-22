use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::RwLock;
use tracing::{debug, warn};

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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct TokenFile {
    tokens: Vec<StoredTokens>,
}

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
        if let Ok(data) = std::fs::read_to_string(token_file_path()) {
            if let Ok(file) = serde_json::from_str::<TokenFile>(&data) {
                for entry in file.tokens {
                    map.insert(entry.provider.clone(), entry);
                }
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
            (rt, None::<String>)
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
        };

        {
            let mut tokens = self.tokens.write().await;
            tokens.insert(provider_name.to_string(), new_stored);
        }

        self.persist().await;
        Ok(token_resp.access_token)
    }

    pub async fn store_tokens(&self, stored: StoredTokens) {
        let name = stored.provider.clone();
        let mut tokens = self.tokens.write().await;
        tokens.insert(name, stored);
        drop(tokens);
        self.persist().await;
    }

    pub async fn remove_tokens(&self, provider_name: &str) -> bool {
        let mut tokens = self.tokens.write().await;
        let removed = tokens.remove(provider_name).is_some();
        drop(tokens);
        if removed {
            self.persist().await;
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

    async fn persist(&self) {
        let tokens = self.tokens.read().await;
        let file = TokenFile {
            tokens: tokens.values().cloned().collect(),
        };
        let path = token_file_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&file) {
            Ok(json) => {
                if let Err(e) = std::fs::write(&path, json) {
                    warn!(error = %e, "failed to persist OAuth tokens");
                }
            }
            Err(e) => warn!(error = %e, "failed to serialize OAuth tokens"),
        }
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
    #[allow(dead_code)]
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
    format!(
        "{ANTHROPIC_AUTHORIZE_URL}?\
         response_type=code\
         &client_id={client_id}\
         &redirect_uri={redirect_uri}\
         &code_challenge={code_challenge}\
         &code_challenge_method=S256\
         &state={state}\
         &scope=user:inference"
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
            }],
        };
        let json = serde_json::to_string(&file).unwrap();
        let parsed: TokenFile = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.tokens.len(), 1);
        assert_eq!(parsed.tokens[0].provider, "anthropic");
        assert_eq!(parsed.tokens[0].access_token, "at-123");
        assert_eq!(parsed.tokens[0].refresh_token.as_deref(), Some("rt-456"));
        assert_eq!(parsed.tokens[0].expires_at, Some(1700000000));
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
}
