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
    let _ = socket.send(Message::Text(welcome.to_string().into())).await;

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
                        let _ = socket.send(Message::Text(resp.to_string().into())).await;
                    }
                    Some(Ok(Message::Ping(data))) => {
                        let _ = socket.send(Message::Pong(data)).await;
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
        // MethodRouter is constructed and can be used with .route("/ws", router)
    }
}
