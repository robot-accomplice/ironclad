//! # ironclad-channels
//!
//! Channel adapters for user-facing chat platforms and the zero-trust
//! agent-to-agent (A2A) communication protocol. All adapters implement the
//! [`ChannelAdapter`] trait for unified message handling.
//!
//! ## Key Types
//!
//! - [`ChannelAdapter`] -- Async trait: `recv()`, `send()`, `platform_name()`
//! - [`InboundMessage`] -- Normalized inbound message from any platform
//! - [`OutboundMessage`] -- Normalized outbound message for any platform
//!
//! ## Modules
//!
//! - `telegram` -- Telegram Bot API (long-poll + webhook, Markdown V2)
//! - `whatsapp` -- WhatsApp Cloud API (webhook, message templates)
//! - `discord` -- Discord Gateway + REST API (slash commands, rich embeds)
//! - `signal` -- Signal Protocol via signal-cli daemon (JSON-RPC)
//! - `web` -- WebSocket interface (axum, JSON frames, ping/pong)
//! - `voice` -- Voice channel (WebRTC, STT, TTS)
//! - `email` -- Email adapter (IMAP listener + SMTP sender)
//! - `a2a` -- Zero-trust A2A protocol (ECDH key exchange, AES-256-GCM)
//! - `router` -- Multi-channel message routing and dispatch
//! - `delivery` -- Outbound delivery queue with retry logic
//! - `filter` -- Addressability filter (per-channel routing rules)

pub mod a2a;
pub mod delivery;
pub mod discord;
pub mod email;
pub mod filter;
pub mod media;
pub mod router;
pub mod signal;
pub mod telegram;
pub mod voice;
pub mod web;
pub mod whatsapp;

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use ironclad_core::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ── Multimodal attachment types ─────────────────────────────────────────

/// Classification of a media attachment received from any channel.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Image,
    Audio,
    Video,
    Document,
}

impl MediaType {
    /// Infer media type from a MIME content-type string.
    pub fn from_content_type(ct: &str) -> Self {
        let ct_lower = ct.to_ascii_lowercase();
        if ct_lower.starts_with("image/") {
            Self::Image
        } else if ct_lower.starts_with("audio/") {
            Self::Audio
        } else if ct_lower.starts_with("video/") {
            Self::Video
        } else {
            Self::Document
        }
    }
}

/// A media attachment received from a channel adapter.
///
/// Stored as JSON inside `InboundMessage.metadata["attachments"]` for full
/// backward compatibility — no changes to trait signatures or struct fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaAttachment {
    pub media_type: MediaType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub content_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caption: Option<String>,
}

/// Maximum allowed length for the `platform` field on [`InboundMessage`].
const MAX_PLATFORM_LEN: usize = 64;

/// Strip control characters and truncate to `MAX_PLATFORM_LEN` bytes.
/// Callers constructing an [`InboundMessage`] should pass the platform name
/// through this function to ensure it contains only printable characters and
/// stays within a reasonable length.
pub fn sanitize_platform(raw: &str) -> String {
    let cleaned: String = raw.chars().filter(|c| !c.is_control()).collect();
    if cleaned.len() <= MAX_PLATFORM_LEN {
        cleaned
    } else {
        // Truncate to MAX_PLATFORM_LEN bytes at a char boundary
        let mut end = MAX_PLATFORM_LEN;
        while end > 0 && !cleaned.is_char_boundary(end) {
            end -= 1;
        }
        cleaned[..end].to_string()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub id: String,
    pub platform: String,
    pub sender_id: String,
    pub content: String,
    pub timestamp: DateTime<Utc>,
    pub metadata: Option<Value>,
}

impl InboundMessage {
    /// Sanitize fields that accept untrusted input.
    /// Currently normalizes `platform` (strips control chars, caps length).
    pub fn sanitize(&mut self) {
        self.platform = sanitize_platform(&self.platform);
    }
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

    #[test]
    fn sanitize_platform_strips_control_chars() {
        assert_eq!(sanitize_platform("telegram\n<script>"), "telegram<script>");
        assert_eq!(sanitize_platform("ok\x00bad\x1F"), "okbad");
    }

    #[test]
    fn sanitize_platform_truncates_long_input() {
        let long = "a".repeat(200);
        assert_eq!(sanitize_platform(&long).len(), MAX_PLATFORM_LEN);
    }

    #[test]
    fn sanitize_platform_passes_clean_input() {
        assert_eq!(sanitize_platform("whatsapp"), "whatsapp");
        assert_eq!(sanitize_platform(""), "");
    }

    #[test]
    fn inbound_message_sanitize_method() {
        let mut msg = InboundMessage {
            id: "s-1".into(),
            platform: "bad\x00name\nhere".into(),
            sender_id: "u1".into(),
            content: "hi".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        msg.sanitize();
        assert_eq!(msg.platform, "badnamehere");
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

    #[test]
    fn sanitize_platform_only_control_chars() {
        assert_eq!(sanitize_platform("\x00\x01\x02\n\r\t"), "");
    }

    #[test]
    fn sanitize_platform_mixed_control_and_printable() {
        assert_eq!(sanitize_platform("te\x00le\ngr\x01am"), "telegram");
    }

    #[test]
    fn sanitize_platform_exact_max_len() {
        let exact = "a".repeat(MAX_PLATFORM_LEN);
        assert_eq!(sanitize_platform(&exact).len(), MAX_PLATFORM_LEN);
    }

    #[test]
    fn sanitize_platform_one_over_max_len() {
        let over = "a".repeat(MAX_PLATFORM_LEN + 1);
        assert_eq!(sanitize_platform(&over).len(), MAX_PLATFORM_LEN);
    }

    #[test]
    fn inbound_message_sanitize_long_platform() {
        let mut msg = InboundMessage {
            id: "s-2".into(),
            platform: "x".repeat(200),
            sender_id: "u1".into(),
            content: "hi".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        msg.sanitize();
        assert_eq!(msg.platform.len(), MAX_PLATFORM_LEN);
    }

    #[test]
    fn outbound_message_with_metadata() {
        let msg = OutboundMessage {
            content: "reply".into(),
            recipient_id: "user-1".into(),
            metadata: Some(serde_json::json!({"thread_id": "t1"})),
        };
        let json = serde_json::to_string(&msg).unwrap();
        let decoded: OutboundMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.metadata.unwrap()["thread_id"], "t1");
    }

    #[test]
    fn inbound_message_clone() {
        let msg = InboundMessage {
            id: "c-1".into(),
            platform: "test".into(),
            sender_id: "u1".into(),
            content: "cloneable".into(),
            timestamp: Utc::now(),
            metadata: Some(serde_json::json!({"key": "val"})),
        };
        let cloned = msg.clone();
        assert_eq!(cloned.id, msg.id);
        assert_eq!(cloned.content, msg.content);
        assert_eq!(cloned.metadata, msg.metadata);
    }

    #[test]
    fn inbound_message_debug() {
        let msg = InboundMessage {
            id: "d-1".into(),
            platform: "test".into(),
            sender_id: "u1".into(),
            content: "debug".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        let debug = format!("{:?}", msg);
        assert!(debug.contains("d-1"));
        assert!(debug.contains("debug"));
    }

    #[test]
    fn outbound_message_clone() {
        let msg = OutboundMessage {
            content: "out".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        let cloned = msg.clone();
        assert_eq!(cloned.content, "out");
        assert_eq!(cloned.recipient_id, "r1");
    }
}
