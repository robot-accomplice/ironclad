use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json;
use uuid::Uuid;

use crate::{InboundMessage, OutboundMessage};

pub struct WebSocketChannel {
    pub connections: Vec<String>,
}

impl WebSocketChannel {
    pub fn new() -> Self {
        Self {
            connections: Vec::new(),
        }
    }

    /// Serialize an `OutboundMessage` to a JSON string for WebSocket transmission.
    pub fn format_ws_message(msg: &OutboundMessage) -> String {
        serde_json::to_string(msg).expect("OutboundMessage is always serializable")
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
}

impl Default for WebSocketChannel {
    fn default() -> Self {
        Self::new()
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
        let json_str = WebSocketChannel::format_ws_message(&outbound);
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
}
