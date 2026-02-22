//! Global API rate limiting (fixed window, Clone-friendly for axum Router).

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use futures_util::future::BoxFuture;
use tokio::sync::Mutex;
use tower::{Layer, Service};

/// Fixed-window rate limit state: at most `capacity` requests per `window`.
#[derive(Clone)]
pub struct GlobalRateLimitLayer {
    state: Arc<Mutex<RateLimitState>>,
    capacity: u64,
    per_ip_capacity: u64,
    window: Duration,
}

struct RateLimitState {
    count: u64,
    window_start: Instant,
    per_ip: HashMap<IpAddr, (u64, Instant)>,
}

impl GlobalRateLimitLayer {
    /// Allow at most `capacity` requests per `window` globally, and `per_ip` per IP.
    pub fn new(capacity: u64, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimitState {
                count: 0,
                window_start: Instant::now(),
                per_ip: HashMap::new(),
            })),
            capacity,
            per_ip_capacity: 300,
            window,
        }
    }
}

impl<S> Layer<S> for GlobalRateLimitLayer {
    type Service = GlobalRateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GlobalRateLimitService {
            inner,
            state: self.state.clone(),
            capacity: self.capacity,
            per_ip_capacity: self.per_ip_capacity,
            window: self.window,
        }
    }
}

#[derive(Clone)]
pub struct GlobalRateLimitService<S> {
    inner: S,
    state: Arc<Mutex<RateLimitState>>,
    capacity: u64,
    per_ip_capacity: u64,
    window: Duration,
}

fn too_many_requests_response() -> Response<Body> {
    let body = serde_json::json!({
        "error": "rate_limit_exceeded",
        "message": "Too many requests, please try again later"
    });
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("content-type", "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap()
}

impl<S> Service<Request<Body>> for GlobalRateLimitService<S>
where
    S: Service<Request<Body>, Response = Response<Body>> + Clone + Send + 'static,
    S::Future: Send + 'static,
{
    type Response = Response<Body>;
    type Error = S::Error;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(
        &mut self,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let mut inner = self.inner.clone();
        let state = self.state.clone();
        let capacity = self.capacity;
        let per_ip_capacity = self.per_ip_capacity;
        let window = self.window;

        let ip: IpAddr = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.split(',').next())
            .and_then(|s| s.trim().parse().ok())
            .or_else(|| {
                req.headers()
                    .get("x-real-ip")
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.trim().parse().ok())
            })
            .unwrap_or_else(|| IpAddr::from([127, 0, 0, 1]));

        Box::pin(async move {
            let now = Instant::now();
            let mut guard = state.lock().await;
            if now.duration_since(guard.window_start) >= window {
                guard.window_start = now;
                guard.count = 0;
            }
            if guard.count >= capacity {
                return Ok(too_many_requests_response());
            }
            guard.count += 1;

            let per_ip_cap = per_ip_capacity;
            let ip_entry = guard.per_ip.entry(ip).or_insert((0, now));
            if now.duration_since(ip_entry.1) >= window {
                *ip_entry = (0, now);
            }
            if ip_entry.0 >= per_ip_cap {
                return Ok(too_many_requests_response());
            }
            ip_entry.0 += 1;

            drop(guard);

            inner.call(req).await
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::{Service, ServiceExt};

    fn dummy_service() -> axum::routing::Router {
        axum::routing::Router::new().route("/", axum::routing::get(|| async { "ok" }))
    }

    #[tokio::test]
    async fn allows_requests_within_capacity() {
        let layer = GlobalRateLimitLayer::new(5, Duration::from_secs(60));
        let mut svc = layer.layer(dummy_service().into_service());
        for _ in 0..5 {
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            let resp = svc.ready().await.unwrap().call(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
    }

    #[tokio::test]
    async fn returns_429_when_capacity_exceeded() {
        let layer = GlobalRateLimitLayer::new(2, Duration::from_secs(60));
        let mut svc = layer.layer(dummy_service().into_service());
        for _ in 0..2 {
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            let _ = svc.ready().await.unwrap().call(req).await.unwrap();
        }
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn window_resets_after_expiry() {
        let layer = GlobalRateLimitLayer::new(1, Duration::from_millis(50));
        let mut svc = layer.layer(dummy_service().into_service());
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        tokio::time::sleep(Duration::from_millis(60)).await;
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn per_ip_limits_enforced() {
        let layer = GlobalRateLimitLayer::new(1000, Duration::from_secs(60));
        let mut svc = layer.layer(dummy_service().into_service());
        for _ in 0..300 {
            let req = Request::builder()
                .uri("/")
                .header("x-forwarded-for", "1.2.3.4")
                .body(Body::empty())
                .unwrap();
            let resp = svc.ready().await.unwrap().call(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let req = Request::builder()
            .uri("/")
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let req = Request::builder()
            .uri("/")
            .header("x-forwarded-for", "5.6.7.8")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
