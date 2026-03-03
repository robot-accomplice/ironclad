//! Global API rate limiting (fixed window, Clone-friendly for axum Router).

use std::collections::HashMap;
use std::hash::Hash;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{Request, Response, StatusCode};
use futures_util::future::BoxFuture;
use tokio::sync::Mutex;
use tower::{Layer, Service};

/// Hard cap on distinct tracked IPs/actors within a window.
/// Requests from new IPs beyond this limit are immediately rate-limited
/// to prevent unbounded memory growth during distributed floods.
const MAX_DISTINCT_IPS: usize = 10_000;
const MAX_DISTINCT_ACTORS: usize = 5_000;

/// Fixed-window rate limit state: at most `capacity` requests per `window`.
#[derive(Clone)]
pub struct GlobalRateLimitLayer {
    state: Arc<Mutex<RateLimitState>>,
    capacity: u64,
    per_ip_capacity: u64,
    per_actor_capacity: u64,
    window: Duration,
    trusted_proxy_cidrs: Vec<IpCidr>,
}

struct RateLimitState {
    count: u64,
    window_start: Instant,
    per_ip: HashMap<IpAddr, (u64, Instant)>,
    per_actor: HashMap<String, (u64, Instant)>,
    throttled_per_ip: HashMap<IpAddr, u64>,
    throttled_per_actor: HashMap<String, u64>,
    throttled_global: u64,
}

#[derive(Clone, Debug)]
struct IpCidr {
    network: IpAddr,
    prefix_len: u8,
}

impl GlobalRateLimitLayer {
    /// Allow at most `capacity` requests per `window` globally, and `per_ip` per IP.
    pub fn new(capacity: u64, window: Duration) -> Self {
        Self {
            state: Arc::new(Mutex::new(RateLimitState {
                count: 0,
                window_start: Instant::now(),
                per_ip: HashMap::new(),
                per_actor: HashMap::new(),
                throttled_per_ip: HashMap::new(),
                throttled_per_actor: HashMap::new(),
                throttled_global: 0,
            })),
            capacity,
            per_ip_capacity: 300,
            per_actor_capacity: 200,
            window,
            trusted_proxy_cidrs: Vec::new(),
        }
    }

    pub fn with_per_ip_capacity(mut self, per_ip_capacity: u64) -> Self {
        self.per_ip_capacity = per_ip_capacity;
        self
    }

    pub fn with_per_actor_capacity(mut self, per_actor_capacity: u64) -> Self {
        self.per_actor_capacity = per_actor_capacity;
        self
    }

    pub fn with_trusted_proxy_cidrs(mut self, cidrs: &[String]) -> Self {
        self.trusted_proxy_cidrs = cidrs
            .iter()
            .filter_map(|c| IpCidr::parse(c))
            .collect::<Vec<_>>();
        self
    }

    fn evict_stale<K>(counter: &mut HashMap<K, (u64, Instant)>, window: Duration)
    where
        K: Eq + Hash,
    {
        let now = Instant::now();
        counter.retain(|_, (_, start)| now.duration_since(*start) < window);
    }

    /// Snapshot current throttle statistics for admin observability.
    ///
    /// Returns counts of throttled requests per-IP, per-actor, and globally
    /// within the current window, plus top offenders (up to 10 each).
    pub async fn snapshot(&self) -> ThrottleSnapshot {
        let guard = self.state.lock().await;

        let mut top_ips: Vec<_> = guard
            .throttled_per_ip
            .iter()
            .map(|(ip, &count)| (ip.to_string(), count))
            .collect();
        top_ips.sort_by(|a, b| b.1.cmp(&a.1));
        top_ips.truncate(10);

        let mut top_actors: Vec<_> = guard
            .throttled_per_actor
            .iter()
            .map(|(actor, &count)| (actor.clone(), count))
            .collect();
        top_actors.sort_by(|a, b| b.1.cmp(&a.1));
        top_actors.truncate(10);

        ThrottleSnapshot {
            window_secs: self.window.as_secs(),
            global_count: guard.count,
            global_capacity: self.capacity,
            per_ip_capacity: self.per_ip_capacity,
            per_actor_capacity: self.per_actor_capacity,
            throttled_global: guard.throttled_global,
            active_ips: guard.per_ip.len(),
            active_actors: guard.per_actor.len(),
            top_throttled_ips: top_ips,
            top_throttled_actors: top_actors,
        }
    }
}

/// Snapshot of current throttle counters for observability.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ThrottleSnapshot {
    pub window_secs: u64,
    pub global_count: u64,
    pub global_capacity: u64,
    pub per_ip_capacity: u64,
    pub per_actor_capacity: u64,
    pub throttled_global: u64,
    pub active_ips: usize,
    pub active_actors: usize,
    pub top_throttled_ips: Vec<(String, u64)>,
    pub top_throttled_actors: Vec<(String, u64)>,
}

impl<S> Layer<S> for GlobalRateLimitLayer {
    type Service = GlobalRateLimitService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        GlobalRateLimitService {
            inner,
            state: self.state.clone(),
            capacity: self.capacity,
            per_ip_capacity: self.per_ip_capacity,
            per_actor_capacity: self.per_actor_capacity,
            window: self.window,
            trusted_proxy_cidrs: self.trusted_proxy_cidrs.clone(),
        }
    }
}

#[derive(Clone)]
pub struct GlobalRateLimitService<S> {
    inner: S,
    state: Arc<Mutex<RateLimitState>>,
    capacity: u64,
    per_ip_capacity: u64,
    per_actor_capacity: u64,
    window: Duration,
    trusted_proxy_cidrs: Vec<IpCidr>,
}

fn too_many_requests_response() -> Response<Body> {
    let body = serde_json::json!({
        "error": "rate_limit_exceeded",
        "message": "Too many requests, please try again later"
    });
    Response::builder()
        .status(StatusCode::TOO_MANY_REQUESTS)
        .header("content-type", "application/json")
        .body(Body::from(
            serde_json::to_vec(&body).expect("static error body serialization"),
        ))
        .expect("error response construction")
}

fn stable_token_fingerprint(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(raw.as_bytes());
    // 8 bytes (64 bits) is plenty for rate-limit dedup — collision-resistant
    // enough for bucket identity while keeping map keys small.
    hex::encode(&hash[..8])
}

fn extract_actor_id(req: &Request<Body>) -> Option<String> {
    let principal = crate::auth::extract_auth_principal(req);
    if let Some(v) = req.headers().get("x-api-key")
        && let Ok(raw) = v.to_str()
        && !raw.is_empty()
    {
        return Some(format!("api_key:{}", stable_token_fingerprint(raw)));
    }
    if let Some(v) = req.headers().get("authorization")
        && let Ok(raw) = v.to_str()
        && let Some(token) = raw.strip_prefix("Bearer ")
        && !token.is_empty()
    {
        return Some(format!("bearer:{}", stable_token_fingerprint(token)));
    }
    // x-user-id header is intentionally NOT used as an actor identity here.
    // It is unauthenticated and would allow rate-limit bypass by cycling IDs.
    principal
}

fn parse_ip(s: &str) -> Option<IpAddr> {
    s.trim().parse().ok()
}

fn forwarded_ip(req: &Request<Body>) -> Option<IpAddr> {
    req.headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split(',').next())
        .and_then(parse_ip)
}

fn real_ip(req: &Request<Body>) -> Option<IpAddr> {
    req.headers()
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .and_then(parse_ip)
}

fn trust_forwarded_headers(proxy_ip: IpAddr, trusted_proxy_cidrs: &[IpCidr]) -> bool {
    trusted_proxy_cidrs
        .iter()
        .any(|cidr| cidr.contains(proxy_ip))
}

fn resolve_client_ip(req: &Request<Body>, trusted_proxy_cidrs: &[IpCidr]) -> IpAddr {
    let forwarded = forwarded_ip(req);
    let real = real_ip(req);

    if let (Some(client_ip), Some(proxy_ip)) = (forwarded, real)
        && trust_forwarded_headers(proxy_ip, trusted_proxy_cidrs)
    {
        return client_ip;
    }

    if let Some(proxy_ip) = real {
        return proxy_ip;
    }

    // Fall back to the actual TCP peer address from ConnectInfo rather than
    // hardcoding 127.0.0.1, which would lump all headerless clients into
    // a single rate-limit bucket.
    use axum::extract::ConnectInfo;
    use std::net::SocketAddr;
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip())
        .unwrap_or(IpAddr::from([127, 0, 0, 1]))
}

impl IpCidr {
    fn parse(raw: &str) -> Option<Self> {
        let (ip, prefix) = raw.split_once('/')?;
        let network = ip.parse::<IpAddr>().ok()?;
        let prefix_len = prefix.parse::<u8>().ok()?;
        let max = match network {
            IpAddr::V4(_) => 32,
            IpAddr::V6(_) => 128,
        };
        if prefix_len > max {
            return None;
        }
        Some(Self {
            network,
            prefix_len,
        })
    }

    fn contains(&self, ip: IpAddr) -> bool {
        match (self.network, ip) {
            (IpAddr::V4(net), IpAddr::V4(candidate)) => {
                cidr_match_v4(net, candidate, self.prefix_len)
            }
            (IpAddr::V6(net), IpAddr::V6(candidate)) => {
                cidr_match_v6(net, candidate, self.prefix_len)
            }
            _ => false,
        }
    }
}

fn cidr_match_v4(network: Ipv4Addr, candidate: Ipv4Addr, prefix_len: u8) -> bool {
    let mask = if prefix_len == 0 {
        0
    } else {
        u32::MAX << (32 - prefix_len)
    };
    (u32::from(network) & mask) == (u32::from(candidate) & mask)
}

fn cidr_match_v6(network: Ipv6Addr, candidate: Ipv6Addr, prefix_len: u8) -> bool {
    let net = u128::from_be_bytes(network.octets());
    let cand = u128::from_be_bytes(candidate.octets());
    let mask = if prefix_len == 0 {
        0
    } else {
        u128::MAX << (128 - prefix_len)
    };
    (net & mask) == (cand & mask)
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
        let per_actor_capacity = self.per_actor_capacity;
        let window = self.window;
        let trusted_proxy_cidrs = self.trusted_proxy_cidrs.clone();
        let ip = resolve_client_ip(&req, &trusted_proxy_cidrs);
        let actor = extract_actor_id(&req);

        Box::pin(async move {
            let now = Instant::now();
            let mut guard = state.lock().await;
            if now.duration_since(guard.window_start) >= window {
                guard.window_start = now;
                guard.count = 0;
                GlobalRateLimitLayer::evict_stale(&mut guard.per_ip, window);
                GlobalRateLimitLayer::evict_stale(&mut guard.per_actor, window);
                guard.throttled_per_ip.clear();
                guard.throttled_per_actor.clear();
                guard.throttled_global = 0;
            }
            if guard.count >= capacity {
                guard.throttled_global += 1;
                return Ok(too_many_requests_response());
            }

            // Check per-IP limit.
            let per_ip_cap = per_ip_capacity;
            if !guard.per_ip.contains_key(&ip) && guard.per_ip.len() >= MAX_DISTINCT_IPS {
                return Ok(too_many_requests_response());
            }
            let ip_entry = guard.per_ip.entry(ip).or_insert((0, now));
            if now.duration_since(ip_entry.1) >= window {
                *ip_entry = (0, now);
            }
            if ip_entry.0 >= per_ip_cap {
                *guard.throttled_per_ip.entry(ip).or_insert(0) += 1;
                return Ok(too_many_requests_response());
            }
            ip_entry.0 += 1;

            // Check per-actor limit.
            if let Some(ref actor_id) = actor {
                if !guard.per_actor.contains_key(actor_id)
                    && guard.per_actor.len() >= MAX_DISTINCT_ACTORS
                {
                    return Ok(too_many_requests_response());
                }
                let actor_entry = guard.per_actor.entry(actor_id.clone()).or_insert((0, now));
                if now.duration_since(actor_entry.1) >= window {
                    *actor_entry = (0, now);
                }
                if actor_entry.0 >= per_actor_capacity {
                    *guard
                        .throttled_per_actor
                        .entry(actor_id.clone())
                        .or_insert(0) += 1;
                    return Ok(too_many_requests_response());
                }
                actor_entry.0 += 1;
            }

            // All per-IP/per-actor checks passed — now increment global counter.
            guard.count += 1;

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
                .header("x-real-ip", "1.2.3.4")
                .body(Body::empty())
                .unwrap();
            let resp = svc.ready().await.unwrap().call(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let req = Request::builder()
            .uri("/")
            .header("x-real-ip", "1.2.3.4")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
        let req = Request::builder()
            .uri("/")
            .header("x-real-ip", "5.6.7.8")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[test]
    fn cidr_parse_and_contains() {
        let cidr = IpCidr::parse("10.0.0.0/8").expect("cidr");
        assert!(cidr.contains("10.1.2.3".parse().unwrap()));
        assert!(!cidr.contains("11.1.2.3".parse().unwrap()));
    }

    #[test]
    fn trusted_proxy_resolution_prefers_forwarded_when_proxy_trusted() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-real-ip", "10.0.0.5")
            .body(Body::empty())
            .unwrap();
        let cidr = IpCidr::parse("10.0.0.0/8").unwrap();
        let ip = resolve_client_ip(&req, &[cidr]);
        assert_eq!(ip, "1.2.3.4".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn untrusted_proxy_resolution_uses_direct_ip() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .header("x-real-ip", "198.51.100.2")
            .body(Body::empty())
            .unwrap();
        let ip = resolve_client_ip(&req, &[]);
        assert_eq!(ip, "198.51.100.2".parse::<IpAddr>().unwrap());
    }

    #[test]
    fn forwarded_header_without_trusted_proxy_is_ignored() {
        let req = Request::builder()
            .header("x-forwarded-for", "1.2.3.4")
            .body(Body::empty())
            .unwrap();
        let ip = resolve_client_ip(&req, &[]);
        assert_eq!(ip, "127.0.0.1".parse::<IpAddr>().unwrap());
    }

    #[tokio::test]
    async fn actor_limits_enforced() {
        let layer =
            GlobalRateLimitLayer::new(1000, Duration::from_secs(60)).with_per_actor_capacity(2);
        let mut svc = layer.layer(dummy_service().into_service());
        for _ in 0..2 {
            let req = Request::builder()
                .uri("/")
                .header("authorization", "Bearer actor-token")
                .body(Body::empty())
                .unwrap();
            let resp = svc.ready().await.unwrap().call(req).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
        }
        let req = Request::builder()
            .uri("/")
            .header("authorization", "Bearer actor-token")
            .body(Body::empty())
            .unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    }

    #[tokio::test]
    async fn snapshot_reflects_throttle_state() {
        let layer = GlobalRateLimitLayer::new(2, Duration::from_secs(60));
        let mut svc = layer.layer(dummy_service().into_service());

        // Exhaust global capacity.
        for _ in 0..2 {
            let req = Request::builder().uri("/").body(Body::empty()).unwrap();
            let _ = svc.ready().await.unwrap().call(req).await.unwrap();
        }
        // This should be throttled.
        let req = Request::builder().uri("/").body(Body::empty()).unwrap();
        let resp = svc.ready().await.unwrap().call(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);

        let snap = layer.snapshot().await;
        assert_eq!(snap.global_count, 2);
        assert_eq!(snap.global_capacity, 2);
        assert!(
            snap.throttled_global >= 1,
            "should record ≥1 throttled global"
        );
        assert_eq!(snap.window_secs, 60);
    }
}
