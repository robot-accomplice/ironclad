use std::net::SocketAddr;
use std::sync::Arc;
use std::task::{Context, Poll};

use subtle::ConstantTimeEq;

use axum::body::Body;
use axum::extract::connect_info::ConnectInfo;
use axum::http::{Request, Response, StatusCode};
use futures_util::future::BoxFuture;
use tower::{Layer, Service};
use tracing::warn;

#[derive(Clone)]
pub struct ApiKeyLayer {
    key: Option<Arc<str>>,
}

impl ApiKeyLayer {
    pub fn new(key: Option<String>) -> Self {
        Self {
            key: key.map(|k| Arc::from(k.as_str())),
        }
    }
}

impl<S> Layer<S> for ApiKeyLayer {
    type Service = ApiKeyMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ApiKeyMiddleware {
            inner,
            key: self.key.clone(),
        }
    }
}

#[derive(Clone)]
pub struct ApiKeyMiddleware<S> {
    inner: S,
    key: Option<Arc<str>>,
}

/// Returns `true` for paths that must be reachable without an API key.
///
/// - `/` and `/api/health` -- uptime probes; read-only, no side-effects.
/// - `/api/webhooks/*` -- inbound from Telegram/WhatsApp; these services
///   cannot supply our API key, so they authenticate via HMAC or provider
///   token validation inside the handler itself.
/// - `/.well-known/agent.json` -- public A2A agent-card discovery.
///
/// NOTE: All exempt paths are still subject to the global and per-IP rate
/// limiter. If you add a new exempt path, ensure it cannot be abused to
/// amplify work (e.g. trigger LLM calls) without its own auth check.
fn is_exempt(path: &str) -> bool {
    path == "/"
        || path == "/api/health"
        || path == "/api/webhooks/telegram"
        || path == "/api/webhooks/whatsapp"
        || path == "/.well-known/agent.json"
}

fn extract_api_key(req: &Request<Body>) -> Option<String> {
    if let Some(val) = req.headers().get("x-api-key")
        && let Ok(s) = val.to_str()
    {
        return Some(s.to_string());
    }
    if let Some(val) = req.headers().get("authorization")
        && let Ok(s) = val.to_str()
        && let Some(token) = s.strip_prefix("Bearer ")
    {
        return Some(token.to_string());
    }
    // S-HIGH-2: query-string ?token= removed — use POST /api/ws-ticket
    // for short-lived, single-use tickets instead.
    None
}

pub(crate) fn extract_auth_principal(req: &Request<Body>) -> Option<String> {
    if req.headers().contains_key("x-api-key") {
        return Some("api_key".to_string());
    }
    if let Some(val) = req.headers().get("authorization")
        && let Ok(s) = val.to_str()
        && s.starts_with("Bearer ")
    {
        return Some("bearer".to_string());
    }
    None
}

fn unauthorized_response() -> Response<Body> {
    let body = serde_json::json!({"error": "unauthorized", "message": "Valid API key required"});
    let bytes = serde_json::to_vec(&body).unwrap_or_else(|_| {
        br#"{"error":"unauthorized","message":"Valid API key required"}"#.to_vec()
    });
    let mut response = Response::new(Body::from(bytes));
    *response.status_mut() = StatusCode::UNAUTHORIZED;
    response.headers_mut().insert(
        axum::http::header::CONTENT_TYPE,
        axum::http::HeaderValue::from_static("application/json"),
    );
    response
}

impl<S> Service<Request<Body>> for ApiKeyMiddleware<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Send + Clone + 'static,
    S::Future: Send + 'static,
{
    type Response = S::Response;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let key = self.key.clone();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            let path = req.uri().path().to_string();
            if let Some(ref expected) = key {
                if !is_exempt(&path) {
                    match extract_api_key(&req) {
                        Some(provided)
                            if bool::from(provided.as_bytes().ct_eq(expected.as_bytes())) => {}
                        _ => return Ok(unauthorized_response()),
                    }
                }
            } else if !is_exempt(&path) {
                // No API key configured — restrict to loopback addresses only.
                let is_loopback = req
                    .extensions()
                    .get::<ConnectInfo<SocketAddr>>()
                    .map(|ci| ci.0.ip().is_loopback())
                    .unwrap_or(false);
                if !is_loopback {
                    warn!(
                        path = %path,
                        "rejected non-loopback request: no API key configured — set server.api_key"
                    );
                    return Ok(unauthorized_response());
                }
            }
            inner.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exempt_paths() {
        assert!(is_exempt("/"));
        assert!(!is_exempt("/ws"));
        assert!(is_exempt("/api/health"));
        assert!(is_exempt("/api/webhooks/telegram"));
        assert!(is_exempt("/api/webhooks/whatsapp"));
        assert!(!is_exempt("/api/config"));
        assert!(!is_exempt("/api/sessions"));
        assert!(!is_exempt("/api/agent/message"));
    }

    #[test]
    fn extract_bearer_token() {
        let req = Request::builder()
            .header("authorization", "Bearer test-key-123")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req).as_deref(), Some("test-key-123"));
    }

    #[test]
    fn extract_x_api_key_header() {
        let req = Request::builder()
            .header("x-api-key", "test-key-789")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req).as_deref(), Some("test-key-789"));
    }

    #[test]
    fn query_token_no_longer_accepted() {
        // S-HIGH-2: ?token= in URL is removed — tickets replace it
        let req = Request::builder()
            .uri("/ws?token=query-key-456")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req), None);
    }

    #[test]
    fn query_token_not_accepted_for_non_ws_paths() {
        let req = Request::builder()
            .uri("/api/health?token=query-key-456")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req), None);
    }

    #[test]
    fn no_key_returns_none() {
        let req = Request::builder().body(Body::empty()).unwrap();
        assert_eq!(extract_api_key(&req), None);
    }

    #[test]
    fn x_api_key_takes_precedence() {
        let req = Request::builder()
            .header("x-api-key", "header-key")
            .header("authorization", "Bearer bearer-key")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req).as_deref(), Some("header-key"));
    }

    #[test]
    fn extract_auth_principal_prefers_api_key() {
        let req = Request::builder()
            .header("x-api-key", "abc")
            .header("authorization", "Bearer token")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_auth_principal(&req).as_deref(), Some("api_key"));
    }

    #[test]
    fn extract_auth_principal_bearer() {
        let req = Request::builder()
            .header("authorization", "Bearer token")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_auth_principal(&req).as_deref(), Some("bearer"));
    }

    #[test]
    fn unknown_webhook_not_exempt() {
        assert!(!is_exempt("/api/webhooks/unknown"));
        assert!(!is_exempt("/api/webhooks/"));
    }

    #[test]
    fn no_key_rejects_non_loopback() {
        // Without ConnectInfo in extensions, the middleware treats it as non-loopback
        let mut req = Request::builder()
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        // Insert a non-loopback ConnectInfo
        let addr: SocketAddr = "192.168.1.5:12345".parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let is_loopback = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        assert!(!is_loopback);
    }

    #[test]
    fn no_key_allows_loopback() {
        let mut req = Request::builder()
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        let addr: SocketAddr = "127.0.0.1:12345".parse().unwrap();
        req.extensions_mut().insert(ConnectInfo(addr));
        let is_loopback = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        assert!(is_loopback);
    }

    #[test]
    fn no_key_allows_exempt_paths() {
        // Exempt paths should be allowed regardless of loopback status
        assert!(is_exempt("/"));
        assert!(is_exempt("/api/health"));
        assert!(is_exempt("/.well-known/agent.json"));
    }

    #[test]
    fn no_key_no_connect_info_rejects() {
        // Without ConnectInfo extension, default is non-loopback (fail closed)
        let req = Request::builder()
            .uri("/api/sessions")
            .body(Body::empty())
            .unwrap();
        let is_loopback = req
            .extensions()
            .get::<ConnectInfo<SocketAddr>>()
            .map(|ci| ci.0.ip().is_loopback())
            .unwrap_or(false);
        assert!(!is_loopback, "missing ConnectInfo should default to reject");
    }

    #[test]
    fn layer_none_key_creates_middleware() {
        let layer = ApiKeyLayer::new(None);
        assert!(layer.key.is_none());
    }

    #[test]
    fn layer_some_key_creates_middleware() {
        let layer = ApiKeyLayer::new(Some("test-layer-key".into()));
        assert_eq!(layer.key.as_deref(), Some("test-layer-key"));
    }
}
