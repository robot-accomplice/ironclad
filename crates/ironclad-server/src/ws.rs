use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use serde::Deserialize;
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;

use crate::ws_ticket::TicketStore;

#[derive(Clone)]
pub struct EventBus {
    tx: broadcast::Sender<String>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    pub fn publish(&self, event: String) {
        if let Err(e) = self.tx.send(event) {
            tracing::debug!(error = %e, "EventBus publish: no active subscribers");
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

#[derive(Deserialize)]
struct WsQuery {
    ticket: Option<String>,
}

/// Returns an axum GET route handler that upgrades the connection to WebSocket.
///
/// Authentication is handled inside this handler (not by the global API-key
/// middleware) because the `/ws` route lives outside the authed router group.
/// Accepts either:
///   - `x-api-key` / `Authorization: Bearer …` header (programmatic clients)
///   - `?ticket=wst_…` query param (short-lived, single-use ticket from `POST /api/ws-ticket`)
pub fn ws_route(
    bus: EventBus,
    tickets: TicketStore,
    api_key: Option<String>,
) -> axum::routing::MethodRouter {
    let api_key: Option<Arc<str>> = api_key.map(|k| Arc::from(k.as_str()));

    let handler =
        move |ws: WebSocketUpgrade,
              headers: axum::http::HeaderMap,
              axum::extract::Query(query): axum::extract::Query<WsQuery>| {
            let bus = bus.clone();
            let tickets = tickets.clone();
            let api_key = api_key.clone();
            async move {
                if !ws_authenticate(&headers, &query, &tickets, api_key.as_deref()) {
                    return (StatusCode::UNAUTHORIZED, "Valid API key or ticket required")
                        .into_response();
                }
                ws.on_upgrade(move |socket| handle_socket(socket, bus))
                    .into_response()
            }
        };
    axum::routing::get(handler)
}

/// Check WebSocket auth: header first, then ticket, then reject.
fn ws_authenticate(
    headers: &axum::http::HeaderMap,
    query: &WsQuery,
    tickets: &TicketStore,
    api_key: Option<&str>,
) -> bool {
    // If no API key is configured, allow all connections (local dev mode)
    let Some(expected) = api_key else {
        return true;
    };

    // 1. Check x-api-key header
    if let Some(val) = headers.get("x-api-key")
        && let Ok(provided) = val.to_str()
        && bool::from(provided.as_bytes().ct_eq(expected.as_bytes()))
    {
        return true;
    }

    // 2. Check Authorization: Bearer header
    if let Some(val) = headers.get("authorization")
        && let Ok(s) = val.to_str()
        && let Some(token) = s.strip_prefix("Bearer ")
        && bool::from(token.as_bytes().ct_eq(expected.as_bytes()))
    {
        return true;
    }

    // 3. Check ticket query param (single-use, short-lived)
    if let Some(ref ticket) = query.ticket
        && tickets.redeem(ticket)
    {
        return true;
    }

    false
}

async fn handle_socket(mut socket: WebSocket, bus: EventBus) {
    let mut rx = bus.subscribe();

    // Send a welcome message
    let welcome = serde_json::json!({
        "type": "connected",
        "version": env!("CARGO_PKG_VERSION"),
        "timestamp": chrono::Utc::now().to_rfc3339(),
    });
    if let Err(e) = socket.send(Message::Text(welcome.to_string().into())).await {
        tracing::debug!(error = %e, "WebSocket welcome send failed");
        return;
    }

    // Forward events from the bus to the WebSocket client
    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    Ok(event) => {
                        if socket.send(Message::Text(event.into())).await.is_err() {
                            break; // client disconnected
                        }
                    }
                    Err(broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "WebSocket subscriber lagged, skipping lost events");
                        continue;
                    }
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Limit inbound message size to prevent memory amplification
                        if text.len() > 4096 {
                            tracing::warn!(len = text.len(), "WebSocket message exceeds 4KiB limit, closing");
                            break;
                        }
                        let resp = serde_json::json!({ "type": "ack" });
                        if let Err(e) = socket.send(Message::Text(resp.to_string().into())).await {
                            tracing::debug!(error = %e, "WebSocket ack send failed");
                            break;
                        }
                    }
                    Some(Ok(Message::Ping(data))) => {
                        if let Err(e) = socket.send(Message::Pong(data)).await {
                            tracing::debug!(error = %e, "WebSocket pong send failed");
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    _ => {}
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn publish_and_receive() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.publish("hello".to_string());
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn subscriber_receives_all_events() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();

        bus.publish("event-1".to_string());
        bus.publish("event-2".to_string());
        bus.publish("event-3".to_string());

        let m1 = rx.recv().await.unwrap();
        let m2 = rx.recv().await.unwrap();
        let m3 = rx.recv().await.unwrap();

        assert_eq!(m1, "event-1");
        assert_eq!(m2, "event-2");
        assert_eq!(m3, "event-3");
    }

    #[tokio::test]
    async fn multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();

        bus.publish("shared".to_string());

        assert_eq!(rx1.recv().await.unwrap(), "shared");
        assert_eq!(rx2.recv().await.unwrap(), "shared");
    }

    #[test]
    fn publish_without_subscribers_does_not_panic() {
        let bus = EventBus::new(4);
        bus.publish("orphan".to_string());
    }

    #[test]
    fn ws_route_returns_method_router() {
        let bus = EventBus::new(256);
        let tickets = TicketStore::new();
        let _router = super::ws_route(bus, tickets, None);
    }

    #[tokio::test]
    async fn event_bus_publish_subscribe() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        bus.publish("hello".to_string());
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, "hello");
    }

    #[tokio::test]
    async fn event_bus_multiple_subscribers() {
        let bus = EventBus::new(16);
        let mut rx1 = bus.subscribe();
        let mut rx2 = bus.subscribe();
        bus.publish("event1".to_string());
        assert_eq!(rx1.recv().await.unwrap(), "event1");
        assert_eq!(rx2.recv().await.unwrap(), "event1");
    }

    #[test]
    fn event_bus_dropped_subscriber_does_not_block() {
        let bus = EventBus::new(16);
        let _rx = bus.subscribe();
        drop(_rx);
        bus.publish("should not block".to_string());
    }

    #[tokio::test]
    async fn bus_clone_shares_channel() {
        let bus1 = EventBus::new(16);
        let bus2 = bus1.clone();
        let mut rx = bus1.subscribe();

        bus2.publish("from-clone".to_string());
        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, "from-clone");
    }

    #[tokio::test]
    async fn subscriber_after_publish_misses_earlier_events() {
        let bus = EventBus::new(16);
        bus.publish("before-subscribe".to_string());

        let mut rx = bus.subscribe();
        bus.publish("after-subscribe".to_string());

        let msg = rx.recv().await.unwrap();
        assert_eq!(msg, "after-subscribe");
    }

    #[test]
    fn capacity_overflow_does_not_panic() {
        let bus = EventBus::new(2);
        let _rx = bus.subscribe();
        for i in 0..10 {
            bus.publish(format!("event-{i}"));
        }
    }

    #[tokio::test]
    async fn publish_json_event_roundtrip() {
        let bus = EventBus::new(16);
        let mut rx = bus.subscribe();
        let event = serde_json::json!({"type": "inference", "model": "gpt-4", "tokens": 42});
        bus.publish(event.to_string());
        let msg = rx.recv().await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(parsed["type"], "inference");
        assert_eq!(parsed["tokens"], 42);
    }

    #[tokio::test]
    async fn multiple_publishes_order_preserved() {
        let bus = EventBus::new(64);
        let mut rx = bus.subscribe();
        for i in 0..50 {
            bus.publish(format!("msg-{i}"));
        }
        for i in 0..50 {
            let msg = rx.recv().await.unwrap();
            assert_eq!(msg, format!("msg-{i}"));
        }
    }

    #[tokio::test]
    async fn concurrent_publishers() {
        let bus = EventBus::new(256);
        let mut rx = bus.subscribe();
        let bus1 = bus.clone();
        let bus2 = bus.clone();

        let h1 = tokio::spawn(async move {
            for i in 0..10 {
                bus1.publish(format!("a-{i}"));
            }
        });
        let h2 = tokio::spawn(async move {
            for i in 0..10 {
                bus2.publish(format!("b-{i}"));
            }
        });

        h1.await.unwrap();
        h2.await.unwrap();

        let mut count = 0;
        while let Ok(msg) =
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv()).await
        {
            msg.unwrap();
            count += 1;
        }
        assert_eq!(count, 20);
    }

    #[test]
    fn ws_route_builds_without_panic() {
        let bus = EventBus::new(4);
        let tickets = TicketStore::new();
        let router = axum::Router::new().route("/ws", super::ws_route(bus, tickets, None));
        let _app = router.into_make_service();
    }

    // ── WebSocket authentication tests ────────────────────────────

    #[test]
    fn ws_auth_no_key_configured_allows_all() {
        let headers = axum::http::HeaderMap::new();
        let query = WsQuery { ticket: None };
        let tickets = TicketStore::new();
        assert!(ws_authenticate(&headers, &query, &tickets, None));
    }

    #[test]
    fn ws_auth_header_x_api_key() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "test-key".parse().unwrap());
        let query = WsQuery { ticket: None };
        let tickets = TicketStore::new();
        assert!(ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_header_bearer() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("authorization", "Bearer test-key".parse().unwrap());
        let query = WsQuery { ticket: None };
        let tickets = TicketStore::new();
        assert!(ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_valid_ticket() {
        let headers = axum::http::HeaderMap::new();
        let tickets = TicketStore::new();
        let ticket = tickets.issue();
        let query = WsQuery {
            ticket: Some(ticket),
        };
        assert!(ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_invalid_ticket_rejected() {
        let headers = axum::http::HeaderMap::new();
        let tickets = TicketStore::new();
        let query = WsQuery {
            ticket: Some("wst_invalid".to_string()),
        };
        assert!(!ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_no_credentials_rejected() {
        let headers = axum::http::HeaderMap::new();
        let query = WsQuery { ticket: None };
        let tickets = TicketStore::new();
        assert!(!ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_wrong_key_rejected() {
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-api-key", "wrong-key".parse().unwrap());
        let query = WsQuery { ticket: None };
        let tickets = TicketStore::new();
        assert!(!ws_authenticate(
            &headers,
            &query,
            &tickets,
            Some("test-key")
        ));
    }

    #[test]
    fn ws_auth_ticket_single_use() {
        let headers = axum::http::HeaderMap::new();
        let tickets = TicketStore::new();
        let ticket = tickets.issue();
        let query1 = WsQuery {
            ticket: Some(ticket.clone()),
        };
        assert!(ws_authenticate(
            &headers,
            &query1,
            &tickets,
            Some("test-key")
        ));
        let query2 = WsQuery {
            ticket: Some(ticket),
        };
        assert!(!ws_authenticate(
            &headers,
            &query2,
            &tickets,
            Some("test-key")
        ));
    }
}
