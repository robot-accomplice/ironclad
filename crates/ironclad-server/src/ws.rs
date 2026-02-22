use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use tokio::sync::broadcast;

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
        let _ = self.tx.send(event);
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }
}

/// Returns an axum GET route handler that upgrades the connection to WebSocket and
/// forwards EventBus events to the client. The handler captures `bus` by value (clone).
pub fn ws_route(bus: EventBus) -> axum::routing::MethodRouter {
    let handler = move |ws: WebSocketUpgrade| {
        let bus = bus.clone();
        async move { ws.on_upgrade(move |socket| handle_socket(socket, bus)) }
    };
    axum::routing::get(handler)
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
                    Err(_) => break,
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Text(text))) => {
                        // Echo back or handle client messages
                        let received: String = text.to_string();
                        let resp = serde_json::json!({
                            "type": "ack",
                            "received": received,
                        });
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
        let _router = super::ws_route(bus);
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
        while let Ok(msg) = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            rx.recv(),
        )
        .await
        {
            msg.unwrap();
            count += 1;
        }
        assert_eq!(count, 20);
    }

    #[test]
    fn ws_route_builds_without_panic() {
        let bus = EventBus::new(4);
        let router = axum::Router::new().route("/ws", super::ws_route(bus));
        let _app = router.into_make_service();
    }
}
