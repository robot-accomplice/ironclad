pub mod a2a;
pub mod delivery;
pub mod discord;
pub mod router;
pub mod telegram;
pub mod web;
pub mod whatsapp;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclad_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub platform: String,
    pub sender_id: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub content: String,
    pub recipient_id: String,
    pub metadata: Option<Value>,
}

#[async_trait]
pub trait ChannelAdapter: Send + Sync {
    fn platform_name(&self) -> &str;
    async fn recv(&self) -> Result<Option<InboundMessage>>;
    async fn send(&self, msg: OutboundMessage) -> Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_message_roundtrip() {
        let msg = InboundMessage {
            id: "msg-1".into(),
            platform: "test".into(),
            sender_id: "user-42".into(),
            content: "hello".into(),
            timestamp: Utc::now(),
            metadata: Some(serde_json::json!({"key": "value"})),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.id, "msg-1");
        assert_eq!(decoded.platform, "test");
        assert_eq!(decoded.content, "hello");
    }

    #[test]
    fn outbound_message_serialization() {
        let msg = OutboundMessage {
            content: "response".into(),
            recipient_id: "user-42".into(),
            metadata: None,
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: OutboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.content, "response");
        assert_eq!(decoded.recipient_id, "user-42");
        assert!(decoded.metadata.is_none());
    }

    // 9C: Edge cases — oversized message, empty message, special chars in platform
    #[test]
    fn inbound_message_oversized_content() {
        let large = "x".repeat(11_000);
        let msg = InboundMessage {
            id: "big-1".into(),
            platform: "telegram".into(),
            sender_id: "u1".into(),
            content: large.clone(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert_eq!(msg.content.len(), 11_000);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.content.len(), 11_000);
    }

    #[test]
    fn inbound_message_empty_content() {
        let msg = InboundMessage {
            id: "empty-1".into(),
            platform: "discord".into(),
            sender_id: "u1".into(),
            content: String::new(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert!(msg.content.is_empty());
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert!(decoded.content.is_empty());
    }

    #[test]
    fn inbound_message_special_chars_in_platform() {
        let msg = InboundMessage {
            id: "spec-1".into(),
            platform: "telegram\n<script>".into(),
            sender_id: "u1".into(),
            content: "hi".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert!(msg.platform.contains('\n'));
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.platform, "telegram\n<script>");
    }

    // Phase 4K: Oversized message (>100KB) handled gracefully
    #[test]
    fn inbound_message_oversized_100kb_handled_gracefully() {
        let oversized = "x".repeat(100 * 1024 + 1);
        let msg = InboundMessage {
            id: "oversized-1".into(),
            platform: "web".into(),
            sender_id: "u1".into(),
            content: oversized.clone(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert!(msg.content.len() > 100 * 1024);
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.content.len(), msg.content.len());
    }

    // Phase 4K: Empty message platform name works
    #[test]
    fn inbound_message_empty_platform_name_works() {
        let msg = InboundMessage {
            id: "ep-1".into(),
            platform: String::new(),
            sender_id: "u1".into(),
            content: "hello".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        assert!(msg.platform.is_empty());
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: InboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.platform, "");
        assert_eq!(decoded.content, "hello");
    }
}
