use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};
use tracing::{debug, warn, error};
use uuid::Uuid;

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

pub struct WhatsAppAdapter {
    pub token: String,
    pub phone_number_id: String,
    pub verify_token: String,
    pub client: reqwest::Client,
    pub allowed_numbers: Vec<String>,
    pub api_version: String,
    /// App secret for webhook X-Hub-Signature-256 verification (HMAC-SHA256).
    pub app_secret: Option<String>,
}

impl WhatsAppAdapter {
    pub fn new(token: String, phone_number_id: String) -> Self {
        Self {
            token,
            phone_number_id,
            verify_token: String::new(),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            allowed_numbers: Vec::new(),
            api_version: "v21.0".into(),
            app_secret: None,
        }
    }

    pub fn with_config(
        token: String,
        phone_number_id: String,
        verify_token: String,
        allowed_numbers: Vec<String>,
        app_secret: Option<String>,
    ) -> Self {
        Self {
            verify_token,
            allowed_numbers,
            app_secret,
            ..Self::new(token, phone_number_id)
        }
    }

    fn api_url(&self, endpoint: &str) -> String {
        format!(
            "https://graph.facebook.com/{}/{}/{}",
            self.api_version, self.phone_number_id, endpoint
        )
    }

    fn is_sender_allowed(&self, sender: &str) -> bool {
        self.allowed_numbers.is_empty() || self.allowed_numbers.iter().any(|n| n == sender)
    }

    pub fn verify_webhook_challenge(
        &self,
        mode: &str,
        token: &str,
        challenge: &str,
    ) -> Result<String> {
        if mode != "subscribe" {
            return Err(IroncladError::Network("invalid hub.mode".into()));
        }
        if token != self.verify_token {
            return Err(IroncladError::Network("verify token mismatch".into()));
        }
        Ok(challenge.to_string())
    }

    pub fn parse_inbound(webhook: &Value) -> Result<InboundMessage> {
        let entry = webhook
            .get("entry")
            .and_then(|e| e.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| IroncladError::Network("missing 'entry' array".into()))?;

        let changes = entry
            .get("changes")
            .and_then(|c| c.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| IroncladError::Network("missing 'changes' array".into()))?;

        let value = changes
            .get("value")
            .ok_or_else(|| IroncladError::Network("missing 'value' in change".into()))?;

        let messages = value
            .get("messages")
            .and_then(|m| m.as_array())
            .and_then(|a| a.first())
            .ok_or_else(|| IroncladError::Network("missing 'messages' array".into()))?;

        let sender_id = messages
            .get("from")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();

        let message_id = messages
            .get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| Uuid::new_v4().to_string());

        let msg_type = messages
            .get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("text");

        let content = match msg_type {
            "text" => messages
                .get("text")
                .and_then(|t| t.get("body"))
                .and_then(|b| b.as_str())
                .unwrap_or("")
                .to_string(),
            "image" | "video" | "audio" | "document" => {
                let caption = messages
                    .get(msg_type)
                    .and_then(|m| m.get("caption"))
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
                let media_id = messages
                    .get(msg_type)
                    .and_then(|m| m.get("id"))
                    .and_then(|i| i.as_str())
                    .unwrap_or("unknown");
                format!("[{msg_type}:{media_id}] {caption}")
            }
            other => format!("[unsupported message type: {other}]"),
        };

        Ok(InboundMessage {
            id: message_id,
            platform: "whatsapp".into(),
            sender_id,
            content,
            timestamp: Utc::now(),
            metadata: Some(webhook.clone()),
        })
    }

    pub fn process_webhook(&self, body: &Value) -> Result<Option<InboundMessage>> {
        if body.get("entry").is_none() {
            return Ok(None);
        }

        let msg = Self::parse_inbound(body)?;

        if !self.is_sender_allowed(&msg.sender_id) {
            debug!(sender = %msg.sender_id, "ignoring message from disallowed number");
            return Ok(None);
        }

        Ok(Some(msg))
    }

    pub fn format_outbound(msg: &OutboundMessage) -> Value {
        json!({
            "messaging_product": "whatsapp",
            "to": msg.recipient_id,
            "type": "text",
            "text": {
                "body": msg.content,
            }
        })
    }

    pub fn format_media_outbound(
        recipient: &str,
        media_type: &str,
        media_url: &str,
        caption: Option<&str>,
    ) -> Value {
        let mut media_obj = json!({ "link": media_url });
        if let Some(cap) = caption {
            media_obj.as_object_mut().unwrap().insert("caption".into(), json!(cap));
        }
        let mut payload = serde_json::Map::new();
        payload.insert("messaging_product".into(), json!("whatsapp"));
        payload.insert("to".into(), json!(recipient));
        payload.insert("type".into(), json!(media_type));
        payload.insert(media_type.to_string(), media_obj);
        Value::Object(payload)
    }

    pub async fn mark_as_read(&self, message_id: &str) -> Result<()> {
        let url = self.api_url("messages");
        let body = json!({
            "messaging_product": "whatsapp",
            "status": "read",
            "message_id": message_id,
        });

        self.client.post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("mark_as_read failed: {e}")))?;

        Ok(())
    }

    async fn handle_api_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();

        if status.as_u16() == 429 {
            warn!("WhatsApp rate limit hit");
            return Err(IroncladError::Network("rate limited".into()));
        }

        let body: Value = resp.json().await
            .map_err(|e| IroncladError::Network(format!("response parse error: {e}")))?;

        if !status.is_success() {
            let err_msg = body.pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!(status = %status, error = err_msg, "WhatsApp API error");
            return Err(IroncladError::Network(format!("WhatsApp API {status}: {err_msg}")));
        }

        Ok(body)
    }
}

#[async_trait]
impl ChannelAdapter for WhatsAppAdapter {
    fn platform_name(&self) -> &str {
        "whatsapp"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        debug!("WhatsApp uses webhook push; recv returns None (use process_webhook instead)");
        Ok(None)
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let url = self.api_url("messages");
        let body = Self::format_outbound(&msg);

        debug!(to = %msg.recipient_id, "sending WhatsApp message");

        let resp = self.client.post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("send message failed: {e}")))?;

        self.handle_api_response(resp).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_inbound_from_webhook_fixture() {
        let webhook = json!({
            "object": "whatsapp_business_account",
            "entry": [{
                "id": "BIZ_ID",
                "changes": [{
                    "value": {
                        "messaging_product": "whatsapp",
                        "metadata": {
                            "display_phone_number": "15551234567",
                            "phone_number_id": "PHONE_ID"
                        },
                        "contacts": [{
                            "profile": { "name": "Duncan" },
                            "wa_id": "15559876543"
                        }],
                        "messages": [{
                            "from": "15559876543",
                            "id": "wamid.abc123",
                            "timestamp": "1677777777",
                            "text": { "body": "Hello from WhatsApp" },
                            "type": "text"
                        }]
                    },
                    "field": "messages"
                }]
            }]
        });

        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert_eq!(msg.platform, "whatsapp");
        assert_eq!(msg.sender_id, "15559876543");
        assert_eq!(msg.id, "wamid.abc123");
        assert_eq!(msg.content, "Hello from WhatsApp");
    }

    #[test]
    fn format_outbound_produces_valid_json() {
        let msg = OutboundMessage {
            content: "Reply text".into(),
            recipient_id: "15559876543".into(),
            metadata: None,
        };
        let payload = WhatsAppAdapter::format_outbound(&msg);
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["to"], "15559876543");
        assert_eq!(payload["type"], "text");
        assert_eq!(payload["text"]["body"], "Reply text");
    }

    #[test]
    fn new_adapter_defaults() {
        let adapter = WhatsAppAdapter::new("tok".into(), "phone123".into());
        assert_eq!(adapter.token, "tok");
        assert_eq!(adapter.phone_number_id, "phone123");
        assert_eq!(adapter.api_version, "v21.0");
        assert!(adapter.allowed_numbers.is_empty());
    }

    #[test]
    fn with_config_sets_fields() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "phone".into(),
            "verify_secret".into(),
            vec!["15551234567".into()],
            None,
        );
        assert_eq!(adapter.verify_token, "verify_secret");
        assert_eq!(adapter.allowed_numbers, vec!["15551234567"]);
    }

    #[test]
    fn verify_webhook_challenge_success() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(), "ph".into(), "my_verify".into(), vec![], None,
        );
        let result = adapter.verify_webhook_challenge("subscribe", "my_verify", "challenge123");
        assert_eq!(result.unwrap(), "challenge123");
    }

    #[test]
    fn verify_webhook_challenge_wrong_mode() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(), "ph".into(), "my_verify".into(), vec![], None,
        );
        let result = adapter.verify_webhook_challenge("unsubscribe", "my_verify", "c");
        assert!(result.is_err());
    }

    #[test]
    fn verify_webhook_challenge_wrong_token() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(), "ph".into(), "my_verify".into(), vec![], None,
        );
        let result = adapter.verify_webhook_challenge("subscribe", "wrong", "c");
        assert!(result.is_err());
    }

    #[test]
    fn sender_allowed_empty_means_all() {
        let adapter = WhatsAppAdapter::new("tok".into(), "ph".into());
        assert!(adapter.is_sender_allowed("any_number"));
    }

    #[test]
    fn sender_allowed_filters() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(), "ph".into(), "v".into(),
            vec!["111".into(), "222".into()],
            None,
        );
        assert!(adapter.is_sender_allowed("111"));
        assert!(!adapter.is_sender_allowed("333"));
    }

    #[test]
    fn process_webhook_valid() {
        let adapter = WhatsAppAdapter::new("tok".into(), "ph".into());
        let webhook = json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "555",
                            "id": "wam1",
                            "type": "text",
                            "text": { "body": "hello" }
                        }]
                    }
                }]
            }]
        });
        let result = adapter.process_webhook(&webhook).unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "hello");
    }

    #[test]
    fn process_webhook_no_entry() {
        let adapter = WhatsAppAdapter::new("tok".into(), "ph".into());
        let result = adapter.process_webhook(&json!({})).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn process_webhook_disallowed_sender() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(), "ph".into(), "v".into(),
            vec!["allowed_only".into()],
            None,
        );
        let webhook = json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "not_allowed",
                            "id": "w1",
                            "type": "text",
                            "text": { "body": "nope" }
                        }]
                    }
                }]
            }]
        });
        let result = adapter.process_webhook(&webhook).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn parse_image_message() {
        let webhook = json!({
            "entry": [{
                "changes": [{
                    "value": {
                        "messages": [{
                            "from": "555",
                            "id": "w2",
                            "type": "image",
                            "image": {
                                "id": "img123",
                                "caption": "look at this"
                            }
                        }]
                    }
                }]
            }]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("image:img123"));
        assert!(msg.content.contains("look at this"));
    }

    #[test]
    fn format_media_outbound_with_caption() {
        let payload = WhatsAppAdapter::format_media_outbound(
            "555", "image", "https://example.com/img.png", Some("caption"),
        );
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["type"], "image");
        assert_eq!(payload["to"], "555");
    }
}
