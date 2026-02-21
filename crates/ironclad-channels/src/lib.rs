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
}
