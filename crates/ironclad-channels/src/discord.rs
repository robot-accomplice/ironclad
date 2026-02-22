use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};
use tracing::{debug, error, warn};
use uuid::Uuid;

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_MESSAGE_LEN: usize = 2000;

pub struct DiscordAdapter {
    pub token: String,
    pub client: reqwest::Client,
    pub allowed_guild_ids: Vec<String>,
    message_buffer: Arc<Mutex<VecDeque<InboundMessage>>>,
}

impl DiscordAdapter {
    pub fn new(token: String) -> Self {
        Self {
            token,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            allowed_guild_ids: Vec::new(),
            message_buffer: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn with_config(token: String, allowed_guild_ids: Vec<String>) -> Self {
        Self {
            allowed_guild_ids,
            ..Self::new(token)
        }
    }

    pub fn buffer_handle(&self) -> Arc<Mutex<VecDeque<InboundMessage>>> {
        Arc::clone(&self.message_buffer)
    }

    fn is_guild_allowed(&self, guild_id: &str) -> bool {
        self.allowed_guild_ids.is_empty() || self.allowed_guild_ids.iter().any(|g| g == guild_id)
    }

    pub fn push_message(&self, msg: InboundMessage) {
        let mut buf = self
            .message_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        buf.push_back(msg);
    }

    pub fn parse_message_create(&self, data: &Value) -> Result<Option<InboundMessage>> {
        let author = data
            .get("author")
            .ok_or_else(|| IroncladError::Network("missing author in MESSAGE_CREATE".into()))?;

        if author.get("bot").and_then(|b| b.as_bool()).unwrap_or(false) {
            return Ok(None);
        }

        let content = data
            .get("content")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        if content.is_empty() {
            return Ok(None);
        }

        if let Some(guild_id) = data.get("guild_id").and_then(|v| v.as_str())
            && !self.is_guild_allowed(guild_id)
        {
            debug!(guild_id, "ignoring message from disallowed guild");
            return Ok(None);
        }

        let message_id = data
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let sender_id = author
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let channel_id = data
            .get("channel_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        Ok(Some(InboundMessage {
            id: if message_id.is_empty() {
                Uuid::new_v4().to_string()
            } else {
                message_id
            },
            platform: "discord".into(),
            sender_id,
            content,
            timestamp: Utc::now(),
            metadata: Some(json!({ "channel_id": channel_id })),
        }))
    }

    pub async fn send_message(&self, channel_id: &str, content: &str) -> Result<Value> {
        let url = format!("{}/channels/{}/messages", DISCORD_API_BASE, channel_id);
        let body = json!({ "content": content });

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("Discord send failed: {e}")))?;

        self.handle_api_response(resp).await
    }

    pub async fn get_gateway_url(&self) -> Result<String> {
        let url = format!("{}/gateway", DISCORD_API_BASE);
        let resp = self
            .client
            .get(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("get gateway failed: {e}")))?;

        let data = self.handle_api_response(resp).await?;
        data.get("url")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| IroncladError::Network("missing 'url' in gateway response".into()))
    }

    pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
        if text.len() <= max_len {
            return vec![text.to_string()];
        }

        let mut chunks = Vec::new();
        let mut remaining = text;

        while !remaining.is_empty() {
            if remaining.len() <= max_len {
                chunks.push(remaining.to_string());
                break;
            }

            let boundary = &remaining[..max_len];
            let split_at = boundary
                .rfind('\n')
                .or_else(|| boundary.rfind(|c: char| c.is_whitespace()))
                .unwrap_or(max_len);

            let (chunk, rest) = remaining.split_at(split_at);
            chunks.push(chunk.to_string());
            remaining = rest.trim_start_matches('\n').trim_start();
        }

        chunks
    }

    async fn handle_api_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();

        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<f64>().ok())
                .unwrap_or(5.0);
            warn!(retry_after, "Discord rate limit hit");
            return Err(IroncladError::Network(format!(
                "rate limited, retry after {retry_after}s"
            )));
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("response parse error: {e}")))?;

        if !status.is_success() {
            let msg = body
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!(status = %status, error = msg, "Discord API error");
            return Err(IroncladError::Network(format!(
                "Discord API {status}: {msg}"
            )));
        }

        Ok(body)
    }
}

#[async_trait]
impl ChannelAdapter for DiscordAdapter {
    fn platform_name(&self) -> &str {
        "discord"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let mut buf = self
            .message_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Ok(buf.pop_front())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let channel_id = &msg.recipient_id;
        let chunks = Self::chunk_message(&msg.content, MAX_MESSAGE_LEN);

        for chunk in chunks {
            debug!(channel_id, len = chunk.len(), "sending Discord message");
            self.send_message(channel_id, &chunk).await?;
        }

        Ok(())
    }
}

/// Discord WebSocket Gateway connection state.
pub struct GatewayConnection {
    _heartbeat_interval_ms: u64,
    sequence: Arc<Mutex<Option<u64>>>,
    session_id: Arc<Mutex<Option<String>>>,
    _resume_gateway_url: Arc<Mutex<Option<String>>>,
}

impl Default for GatewayConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayConnection {
    pub fn new() -> Self {
        Self {
            _heartbeat_interval_ms: 41250,
            sequence: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            _resume_gateway_url: Arc::new(Mutex::new(None)),
        }
    }

    pub fn sequence(&self) -> Option<u64> {
        *self.sequence.lock().expect("mutex poisoned")
    }

    pub fn set_sequence(&self, seq: Option<u64>) {
        *self.sequence.lock().expect("mutex poisoned") = seq;
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id.lock().expect("mutex poisoned").clone()
    }

    pub fn set_session_id(&self, id: String) {
        *self.session_id.lock().expect("mutex poisoned") = Some(id);
    }
}

impl DiscordAdapter {
    /// Build the Gateway Identify payload.
    pub fn build_identify(&self) -> serde_json::Value {
        serde_json::json!({
            "op": 2,
            "d": {
                "token": self.token,
                "intents": 512 | 1 | 4096,
                "properties": {
                    "os": "linux",
                    "browser": "ironclad",
                    "device": "ironclad"
                }
            }
        })
    }

    /// Build a heartbeat payload.
    pub fn build_heartbeat(&self, sequence: Option<u64>) -> serde_json::Value {
        serde_json::json!({
            "op": 1,
            "d": sequence
        })
    }

    /// Build a Resume payload for reconnection.
    pub fn build_resume(&self, session_id: &str, sequence: u64) -> serde_json::Value {
        serde_json::json!({
            "op": 6,
            "d": {
                "token": self.token,
                "session_id": session_id,
                "seq": sequence
            }
        })
    }

    /// Parse a gateway dispatch event (op=0). Returns the event name and parsed data.
    pub fn parse_dispatch(
        &self,
        payload: &serde_json::Value,
    ) -> Option<(String, serde_json::Value)> {
        let event_name = payload.get("t")?.as_str()?.to_string();
        let data = payload.get("d")?.clone();
        let _seq = payload.get("s").and_then(|v| v.as_u64());

        Some((event_name, data))
    }

    /// Determine the gateway opcode from a received message.
    pub fn gateway_opcode(payload: &serde_json::Value) -> Option<u64> {
        payload.get("op")?.as_u64()
    }

    /// Extract the heartbeat interval from a Hello (op=10) payload.
    pub fn extract_heartbeat_interval(payload: &serde_json::Value) -> Option<u64> {
        payload.get("d")?.get("heartbeat_interval")?.as_u64()
    }

    /// Check if a gateway close code is resumable.
    pub fn is_resumable_close(code: u16) -> bool {
        matches!(code, 4000 | 4001 | 4002 | 4003 | 4005 | 4007 | 4008 | 4009)
    }

    /// Check if a gateway close code is fatal (should not reconnect).
    pub fn is_fatal_close(code: u16) -> bool {
        matches!(code, 4004 | 4010 | 4011 | 4012 | 4013 | 4014)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_adapter_defaults() {
        let adapter = DiscordAdapter::new("test-token".into());
        assert_eq!(adapter.token, "test-token");
        assert!(adapter.allowed_guild_ids.is_empty());
    }

    #[test]
    fn with_config_sets_guilds() {
        let adapter =
            DiscordAdapter::with_config("tok".into(), vec!["guild1".into(), "guild2".into()]);
        assert_eq!(adapter.allowed_guild_ids.len(), 2);
    }

    #[test]
    fn guild_allowed_empty_means_all() {
        let adapter = DiscordAdapter::new("tok".into());
        assert!(adapter.is_guild_allowed("any_guild"));
    }

    #[test]
    fn guild_allowed_filters() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["g1".into()]);
        assert!(adapter.is_guild_allowed("g1"));
        assert!(!adapter.is_guild_allowed("g2"));
    }

    #[test]
    fn parse_message_create_valid() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "msg123",
            "channel_id": "ch456",
            "guild_id": "g789",
            "author": { "id": "user1", "username": "testuser" },
            "content": "hello discord"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
        let msg = result.unwrap();
        assert_eq!(msg.platform, "discord");
        assert_eq!(msg.id, "msg123");
        assert_eq!(msg.sender_id, "user1");
        assert_eq!(msg.content, "hello discord");
    }

    #[test]
    fn parse_message_create_skips_bots() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "msg1",
            "channel_id": "ch1",
            "author": { "id": "bot1", "bot": true },
            "content": "bot message"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_message_create_skips_empty() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "msg1",
            "channel_id": "ch1",
            "author": { "id": "user1" },
            "content": ""
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_message_create_filters_guild() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["allowed".into()]);
        let data = json!({
            "id": "msg1",
            "channel_id": "ch1",
            "guild_id": "not_allowed",
            "author": { "id": "user1" },
            "content": "filtered"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn push_and_recv_message() {
        let adapter = DiscordAdapter::new("tok".into());
        let msg = InboundMessage {
            id: "m1".into(),
            platform: "discord".into(),
            sender_id: "u1".into(),
            content: "buffered".into(),
            timestamp: Utc::now(),
            metadata: None,
        };
        adapter.push_message(msg);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(adapter.recv()).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "buffered");

        let result2 = rt.block_on(adapter.recv()).unwrap();
        assert!(result2.is_none());
    }

    #[test]
    fn chunk_message_short() {
        let chunks = DiscordAdapter::chunk_message("short", 2000);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "short");
    }

    #[test]
    fn chunk_message_long() {
        let text = "word ".repeat(500);
        let chunks = DiscordAdapter::chunk_message(text.trim(), 100);
        for chunk in &chunks {
            assert!(chunk.len() <= 100);
        }
    }

    #[test]
    fn chunk_message_prefers_newline() {
        let text = "line one\nline two which is a bit longer\nline three";
        let chunks = DiscordAdapter::chunk_message(text, 25);
        assert!(chunks[0].ends_with("one"));
    }

    #[test]
    fn buffer_handle_returns_shared_buffer() {
        let adapter = DiscordAdapter::new("tok".into());
        let handle = adapter.buffer_handle();
        adapter.push_message(InboundMessage {
            id: "x".into(),
            platform: "discord".into(),
            sender_id: "u".into(),
            content: "via handle".into(),
            timestamp: Utc::now(),
            metadata: None,
        });
        let buf = handle.lock().unwrap();
        assert_eq!(buf.len(), 1);
        assert_eq!(buf[0].content, "via handle");
    }

    #[test]
    fn parse_message_create_missing_author() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({"id": "m1", "channel_id": "c1", "content": "no author"});
        assert!(adapter.parse_message_create(&data).is_err());
    }

    #[test]
    fn parse_message_create_missing_id_generates_uuid() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "channel_id": "c1",
            "author": {"id": "u1"},
            "content": "no msg id"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn parse_message_create_no_guild_id_passes_through() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["restricted".into()]);
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "u1"},
            "content": "dm message"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn parse_message_create_metadata_has_channel_id() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "m1",
            "channel_id": "ch999",
            "author": {"id": "u1"},
            "content": "test"
        });
        let msg = adapter.parse_message_create(&data).unwrap().unwrap();
        assert_eq!(msg.metadata.unwrap()["channel_id"], "ch999");
    }

    #[test]
    fn chunk_message_empty() {
        let chunks = DiscordAdapter::chunk_message("", 100);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_message_exact_boundary() {
        let text = "a".repeat(100);
        let chunks = DiscordAdapter::chunk_message(&text, 100);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_message_no_whitespace_hard_split() {
        let text = "a".repeat(50);
        let chunks = DiscordAdapter::chunk_message(&text, 20);
        for chunk in &chunks {
            assert!(chunk.len() <= 20);
        }
    }

    #[test]
    fn platform_name_is_discord() {
        let adapter = DiscordAdapter::new("tok".into());
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(async { adapter.platform_name() }), "discord");
    }

    #[test]
    fn build_identify_has_token_and_intents() {
        let adapter = DiscordAdapter::new("test-bot-token".into());
        let identify = adapter.build_identify();
        assert_eq!(identify["op"], 2);
        assert_eq!(identify["d"]["token"], "test-bot-token");
        let intents = identify["d"]["intents"].as_u64().unwrap();
        assert!(intents & 512 != 0, "should have GUILD_MESSAGES intent");
    }

    #[test]
    fn build_heartbeat_with_sequence() {
        let adapter = DiscordAdapter::new("tok".into());
        let hb = adapter.build_heartbeat(Some(42));
        assert_eq!(hb["op"], 1);
        assert_eq!(hb["d"], 42);
    }

    #[test]
    fn build_heartbeat_null_sequence() {
        let adapter = DiscordAdapter::new("tok".into());
        let hb = adapter.build_heartbeat(None);
        assert_eq!(hb["op"], 1);
        assert!(hb["d"].is_null());
    }

    #[test]
    fn build_resume_payload() {
        let adapter = DiscordAdapter::new("tok".into());
        let resume = adapter.build_resume("session-123", 99);
        assert_eq!(resume["op"], 6);
        assert_eq!(resume["d"]["session_id"], "session-123");
        assert_eq!(resume["d"]["seq"], 99);
    }

    #[test]
    fn gateway_opcode_extracts_op() {
        let hello = serde_json::json!({"op": 10, "d": {"heartbeat_interval": 41250}});
        assert_eq!(DiscordAdapter::gateway_opcode(&hello), Some(10));
    }

    #[test]
    fn extract_heartbeat_interval_from_hello() {
        let hello = serde_json::json!({"op": 10, "d": {"heartbeat_interval": 41250}});
        assert_eq!(
            DiscordAdapter::extract_heartbeat_interval(&hello),
            Some(41250)
        );
    }

    #[test]
    fn resumable_close_codes() {
        assert!(DiscordAdapter::is_resumable_close(4000));
        assert!(DiscordAdapter::is_resumable_close(4009));
        assert!(!DiscordAdapter::is_resumable_close(4004));
    }

    #[test]
    fn fatal_close_codes() {
        assert!(DiscordAdapter::is_fatal_close(4004));
        assert!(DiscordAdapter::is_fatal_close(4014));
        assert!(!DiscordAdapter::is_fatal_close(4000));
    }

    #[test]
    fn parse_dispatch_extracts_event() {
        let adapter = DiscordAdapter::new("tok".into());
        let dispatch = serde_json::json!({
            "op": 0,
            "s": 42,
            "t": "MESSAGE_CREATE",
            "d": {"content": "hello", "author": {"id": "123", "bot": false}}
        });
        let (name, data) = adapter.parse_dispatch(&dispatch).unwrap();
        assert_eq!(name, "MESSAGE_CREATE");
        assert_eq!(data["content"], "hello");
    }

    #[test]
    fn gateway_connection_state() {
        let conn = GatewayConnection::new();
        assert!(conn.sequence().is_none());
        assert!(conn.session_id().is_none());

        conn.set_sequence(Some(42));
        assert_eq!(conn.sequence(), Some(42));

        conn.set_session_id("test-session".to_string());
        assert_eq!(conn.session_id().as_deref(), Some("test-session"));
    }
}
