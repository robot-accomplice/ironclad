use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json;
use tokio::sync::{broadcast, mpsc, Mutex};
use uuid::Uuid;

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

/// Sink for pushing inbound WebSocket messages into the channel (e.g. from the WS handler).
#[derive(Clone)]
pub struct WebSocketChannelSink {
    inbound_tx: mpsc::Sender<InboundMessage>,
}

impl WebSocketChannelSink {
    /// Push an inbound message into the channel. Returns an error if the receiver is dropped.
    pub async fn push(&self, msg: InboundMessage) -> Result<()> {
        self.inbound_tx
            .send(msg)
            .await
            .map_err(|e| IroncladError::Network(format!("WebSocket channel send: {e}")))
    }
}

/// WebSocket channel adapter: recv from mpsc, send via broadcast.
pub struct WebSocketChannel {
    recv_rx: Mutex<mpsc::Receiver<InboundMessage>>,
    send_tx: broadcast::Sender<OutboundMessage>,
}

impl WebSocketChannel {
    /// Creates a new WebSocket channel and a sink for pushing inbound messages.
    /// The returned `Arc<WebSocketChannel>` implements `ChannelAdapter` and can be registered
    /// with `ChannelRouter`. The sink should be given to the WebSocket handler to push
    /// parsed inbound messages.
    pub fn new() -> (Arc<Self>, WebSocketChannelSink) {
        let (inbound_tx, inbound_rx) = mpsc::channel(256);
        let (send_tx, _) = broadcast::channel(64);
        let channel = Arc::new(Self {
            recv_rx: Mutex::new(inbound_rx),
            send_tx,
        });
        let sink = WebSocketChannelSink { inbound_tx };
        (channel, sink)
    }

    /// Serialize an `OutboundMessage` to a JSON string for WebSocket transmission.
    pub fn format_ws_message(msg: &OutboundMessage) -> Result<String> {
        serde_json::to_string(msg).map_err(|e| IroncladError::Channel(format!("failed to serialize: {e}")))
    }

    /// Parse a raw JSON string from a WebSocket frame into an `InboundMessage`.
    pub fn parse_ws_message(raw: &str) -> Result<InboundMessage> {
        let v: serde_json::Value = serde_json::from_str(raw)
            .map_err(|e| IroncladError::Network(format!("invalid WebSocket JSON: {e}")))?;

        let id = v
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let sender_id = v
            .get("sender_id")
            .and_then(|v| v.as_str())
            .unwrap_or("anonymous")
            .to_string();

        let content = v
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(InboundMessage {
            id,
            platform: "web".into(),
            sender_id,
            content,
            timestamp: Utc::now(),
            metadata: v.get("metadata").cloned(),
        })
    }

    /// Subscribe to outbound messages (e.g. for a WebSocket connection). Returns a receiver
    /// that will receive copies of all messages sent via `send()`.
    pub fn subscribe(&self) -> broadcast::Receiver<OutboundMessage> {
        self.send_tx.subscribe()
    }
}

#[async_trait]
impl ChannelAdapter for WebSocketChannel {
    fn platform_name(&self) -> &str {
        "web"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let mut rx = self.recv_rx.lock().await;
        let msg = rx.recv().await;
        Ok(msg)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        self.send_tx
            .send(msg)
            .map_err(|e| IroncladError::Network(format!("WebSocket broadcast: {e}")))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_and_parse_roundtrip() {
        let outbound = OutboundMessage {
            content: "round-trip test".into(),
            recipient_id: "user-7".into(),
            metadata: None,
        };
        let json_str = WebSocketChannel::format_ws_message(&outbound).unwrap();
        assert!(json_str.contains("round-trip test"));
        assert!(json_str.contains("user-7"));

        let raw = serde_json::json!({
            "id": "msg-1",
            "sender_id": "user-7",
            "content": "round-trip test",
        });
        let inbound = WebSocketChannel::parse_ws_message(&raw.to_string()).unwrap();
        assert_eq!(inbound.content, "round-trip test");
        assert_eq!(inbound.sender_id, "user-7");
        assert_eq!(inbound.platform, "web");
    }

    #[test]
    fn parse_ws_message_with_metadata() {
        let raw = serde_json::json!({
            "id": "msg-99",
            "sender_id": "admin",
            "content": "with meta",
            "metadata": { "source": "dashboard" }
        });
        let msg = WebSocketChannel::parse_ws_message(&raw.to_string()).unwrap();
        assert_eq!(msg.id, "msg-99");
        assert!(msg.metadata.is_some());
        assert_eq!(msg.metadata.unwrap()["source"], "dashboard");
    }

    #[test]
    fn parse_ws_message_rejects_invalid_json() {
        let result = WebSocketChannel::parse_ws_message("not json {{{");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn websocket_adapter_send_recv_roundtrip() {
        let (channel, sink) = WebSocketChannel::new();

        // Subscribe before sending so we don't miss the message
        let mut sub = channel.subscribe();

        // Push an inbound message via sink
        let inbound = InboundMessage {
            id: "id-1".into(),
            platform: "web".into(),
            sender_id: "user-1".into(),
            content: "hello from client".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        sink.push(inbound.clone()).await.unwrap();

        // recv() should get it
        let received = channel.recv().await.unwrap();
        assert!(received.is_some());
        let received = received.unwrap();
        assert_eq!(received.content, "hello from client");
        assert_eq!(received.sender_id, "user-1");

        // send() should broadcast to subscriber
        let outbound = OutboundMessage {
            content: "reply from server".into(),
            recipient_id: "user-1".into(),
            metadata: None,
        };
        channel.send(outbound.clone()).await.unwrap();
        let from_sub = sub.recv().await.unwrap();
        assert_eq!(from_sub.content, "reply from server");
        assert_eq!(from_sub.recipient_id, "user-1");
    }
}
