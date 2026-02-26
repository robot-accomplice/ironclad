use std::sync::Arc;
use std::task::{Context, Poll};

use subtle::ConstantTimeEq;

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use futures_util::future::BoxFuture;
use tower::{Layer, Service};

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
    // Only allow query-string token for WebSocket upgrade path
    if req.uri().path() == "/ws"
        && let Some(query) = req.uri().query()
    {
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=')
                && k == "token"
                && !v.is_empty()
            {
                return Some(v.to_string());
            }
        }
    }
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
    Response::builder()
        .status(StatusCode::UNAUTHORIZED)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
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
            if let Some(ref expected) = key {
                let path = req.uri().path();
                if !is_exempt(path) {
                    match extract_api_key(&req) {
                        Some(provided)
                            if bool::from(provided.as_bytes().ct_eq(expected.as_bytes())) => {}
                        _ => return Ok(unauthorized_response()),
                    }
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
    fn extract_token_from_query() {
        let req = Request::builder()
            .uri("/ws?token=query-key-456")
            .body(Body::empty())
            .unwrap();
        assert_eq!(extract_api_key(&req).as_deref(), Some("query-key-456"));
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
