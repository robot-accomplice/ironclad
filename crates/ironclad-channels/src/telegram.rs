use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};
use tracing::{debug, warn, error};
use uuid::Uuid;

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

pub struct TelegramAdapter {
    pub token: String,
    pub client: reqwest::Client,
    pub last_update_id: Arc<Mutex<i64>>,
    pub poll_timeout: u64,
    pub allowed_chat_ids: Vec<i64>,
}

impl TelegramAdapter {
    pub fn new(token: String) -> Self {
        Self {
            token,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(60))
                .build()
                .unwrap_or_default(),
            last_update_id: Arc::new(Mutex::new(0)),
            poll_timeout: 30,
            allowed_chat_ids: Vec::new(),
        }
    }

    pub fn with_config(token: String, poll_timeout: u64, allowed_chat_ids: Vec<i64>) -> Self {
        Self {
            poll_timeout,
            allowed_chat_ids,
            ..Self::new(token)
        }
    }

    fn api_url(&self, method: &str) -> String {
        format!("https://api.telegram.org/bot{}/{}", self.token, method)
    }

    fn is_chat_allowed(&self, chat_id: i64) -> bool {
        self.allowed_chat_ids.is_empty() || self.allowed_chat_ids.contains(&chat_id)
    }

    pub fn parse_inbound(update: &Value) -> Result<InboundMessage> {
        let message = update
            .get("message")
            .ok_or_else(|| IroncladError::Network("missing 'message' in update".into()))?;

        let text = message
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();

        let from = message
            .get("from")
            .ok_or_else(|| IroncladError::Network("missing 'from' in message".into()))?;

        let sender_id = from
            .get("id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .ok_or_else(|| IroncladError::Network("missing 'from.id'".into()))?;

        let message_id = message
            .get("message_id")
            .and_then(|v| v.as_i64())
            .map(|id| id.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        Ok(InboundMessage {
            id: message_id,
            platform: "telegram".into(),
            sender_id,
            content: text,
            timestamp: Utc::now(),
            metadata: Some(update.clone()),
        })
    }

    pub fn format_outbound(msg: &OutboundMessage) -> Value {
        json!({
            "chat_id": msg.recipient_id,
            "text": msg.content,
        })
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
                .rfind(|c: char| c.is_whitespace())
                .unwrap_or(max_len);

            let (chunk, rest) = remaining.split_at(split_at);
            chunks.push(chunk.to_string());
            remaining = rest.trim_start();
        }

        chunks
    }

    pub async fn register_webhook(&self, url: &str) -> Result<()> {
        let api_url = self.api_url("setWebhook");
        let body = json!({ "url": url });
        let resp = self.client.post(&api_url).json(&body).send().await
            .map_err(|e| IroncladError::Network(format!("setWebhook failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!("setWebhook error: {text}")));
        }
        debug!("Telegram webhook registered: {url}");
        Ok(())
    }

    pub async fn delete_webhook(&self) -> Result<()> {
        let api_url = self.api_url("deleteWebhook");
        let resp = self.client.post(&api_url).send().await
            .map_err(|e| IroncladError::Network(format!("deleteWebhook failed: {e}")))?;

        if !resp.status().is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!("deleteWebhook error: {text}")));
        }
        debug!("Telegram webhook deleted");
        Ok(())
    }

    pub fn process_webhook_update(&self, body: &Value) -> Result<Option<InboundMessage>> {
        if let Some(update_id) = body.get("update_id").and_then(|v| v.as_i64()) {
            let mut last = self.last_update_id.lock().expect("mutex poisoned");
            if update_id > *last {
                *last = update_id;
            }
        }

        if body.get("message").is_none() {
            return Ok(None);
        }

        if let Some(chat_id) = body.pointer("/message/chat/id").and_then(|v| v.as_i64()) {
            if !self.is_chat_allowed(chat_id) {
                debug!(chat_id, "ignoring message from disallowed chat");
                return Ok(None);
            }
        }

        Self::parse_inbound(body).map(Some)
    }

    async fn handle_api_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();

        if status.as_u16() == 429 {
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(5);
            warn!(retry_after, "Telegram rate limit hit");
            return Err(IroncladError::Network(format!(
                "rate limited, retry after {retry_after}s"
            )));
        }

        let body: Value = resp.json().await
            .map_err(|e| IroncladError::Network(format!("response parse error: {e}")))?;

        if !status.is_success() {
            let desc = body.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!(status = %status, description = desc, "Telegram API error");
            return Err(IroncladError::Network(format!("Telegram API {status}: {desc}")));
        }

        Ok(body)
    }
}

#[async_trait]
impl ChannelAdapter for TelegramAdapter {
    fn platform_name(&self) -> &str {
        "telegram"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let offset = {
            let last = self.last_update_id.lock().expect("mutex poisoned");
            *last + 1
        };

        let url = self.api_url("getUpdates");
        let body = json!({
            "offset": offset,
            "timeout": self.poll_timeout,
            "allowed_updates": ["message"],
        });

        debug!(offset, "polling Telegram getUpdates");

        let resp = self.client.post(&url).json(&body).send().await
            .map_err(|e| IroncladError::Network(format!("getUpdates failed: {e}")))?;

        let data = self.handle_api_response(resp).await?;

        let updates = data.get("result")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        if updates.is_empty() {
            return Ok(None);
        }

        let update = &updates[0];
        if let Some(uid) = update.get("update_id").and_then(|v| v.as_i64()) {
            let mut last = self.last_update_id.lock().expect("mutex poisoned");
            *last = uid;
        }

        if let Some(chat_id) = update.pointer("/message/chat/id").and_then(|v| v.as_i64()) {
            if !self.is_chat_allowed(chat_id) {
                debug!(chat_id, "ignoring message from disallowed chat");
                return Ok(None);
            }
        }

        if update.get("message").is_none() {
            return Ok(None);
        }

        Self::parse_inbound(update).map(Some)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let url = self.api_url("sendMessage");
        let chunks = Self::chunk_message(&msg.content, 4096);

        for chunk in chunks {
            let body = json!({
                "chat_id": msg.recipient_id,
                "text": chunk,
            });

            debug!(chat_id = %msg.recipient_id, len = chunk.len(), "sending Telegram message");

            let resp = self.client.post(&url).json(&body).send().await
                .map_err(|e| IroncladError::Network(format!("sendMessage failed: {e}")))?;

            self.handle_api_response(resp).await?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inbound_from_fixture() {
        let update = json!({
            "update_id": 123456,
            "message": {
                "message_id": 42,
                "from": {
                    "id": 98765,
                    "is_bot": false,
                    "first_name": "Duncan"
                },
                "chat": {
                    "id": 98765,
                    "type": "private"
                },
                "text": "Hello from Telegram"
            }
        });

        let msg = TelegramAdapter::parse_inbound(&update).unwrap();
        assert_eq!(msg.platform, "telegram");
        assert_eq!(msg.sender_id, "98765");
        assert_eq!(msg.id, "42");
        assert_eq!(msg.content, "Hello from Telegram");
        assert!(msg.metadata.is_some());
    }

    #[test]
    fn format_outbound_produces_valid_json() {
        let msg = OutboundMessage {
            content: "Hi there".into(),
            recipient_id: "12345".into(),
            metadata: None,
        };
        let payload = TelegramAdapter::format_outbound(&msg);
        assert_eq!(payload["chat_id"], "12345");
        assert_eq!(payload["text"], "Hi there");
    }

    #[test]
    fn chunk_message_splits_on_word_boundary() {
        let text = "hello world this is a test of the chunking system";
        let chunks = TelegramAdapter::chunk_message(text, 20);

        for chunk in &chunks {
            assert!(chunk.len() <= 20, "chunk too long: {}", chunk);
        }

        let rejoined = chunks.join(" ");
        assert_eq!(rejoined, text);
    }

    #[test]
    fn new_adapter_defaults() {
        let adapter = TelegramAdapter::new("test-token".into());
        assert_eq!(adapter.token, "test-token");
        assert_eq!(adapter.poll_timeout, 30);
        assert!(adapter.allowed_chat_ids.is_empty());
    }

    #[test]
    fn with_config_sets_fields() {
        let adapter = TelegramAdapter::with_config("tok".into(), 60, vec![111, 222]);
        assert_eq!(adapter.poll_timeout, 60);
        assert_eq!(adapter.allowed_chat_ids, vec![111, 222]);
    }

    #[test]
    fn chat_allowed_empty_means_all() {
        let adapter = TelegramAdapter::new("tok".into());
        assert!(adapter.is_chat_allowed(12345));
    }

    #[test]
    fn chat_allowed_filters() {
        let adapter = TelegramAdapter::with_config("tok".into(), 30, vec![100, 200]);
        assert!(adapter.is_chat_allowed(100));
        assert!(adapter.is_chat_allowed(200));
        assert!(!adapter.is_chat_allowed(300));
    }

    #[test]
    fn process_webhook_update_valid() {
        let adapter = TelegramAdapter::new("tok".into());
        let update = json!({
            "update_id": 999,
            "message": {
                "message_id": 1,
                "from": { "id": 42 },
                "chat": { "id": 42, "type": "private" },
                "text": "webhook msg"
            }
        });
        let result = adapter.process_webhook_update(&update).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "webhook msg");
    }

    #[test]
    fn process_webhook_update_no_message() {
        let adapter = TelegramAdapter::new("tok".into());
        let update = json!({ "update_id": 100, "edited_message": {} });
        let result = adapter.process_webhook_update(&update).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn process_webhook_update_disallowed_chat() {
        let adapter = TelegramAdapter::with_config("tok".into(), 30, vec![100]);
        let update = json!({
            "update_id": 101,
            "message": {
                "message_id": 1,
                "from": { "id": 999 },
                "chat": { "id": 999, "type": "private" },
                "text": "blocked"
            }
        });
        let result = adapter.process_webhook_update(&update).unwrap();
        assert!(result.is_none());
    }
}
