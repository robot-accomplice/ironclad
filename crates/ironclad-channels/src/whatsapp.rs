use async_trait::async_trait;
use chrono::Utc;
use hmac::{Hmac, Mac};
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};
use sha2::Sha256;
use tracing::{debug, error, warn};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

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
                .expect("HTTP client initialization - check TLS certificates"),
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

    /// Verifies the X-Hub-Signature-256 HMAC from Meta webhook payloads.
    /// Returns Ok(()) if verification passes, Err if the signature is invalid.
    /// Returns Err if no app_secret is configured -- operators must set it.
    pub fn verify_webhook_signature(
        &self,
        raw_body: &[u8],
        signature_header: Option<&str>,
    ) -> Result<()> {
        let secret = match &self.app_secret {
            Some(s) if !s.is_empty() => s,
            _ => {
                error!("WhatsApp app_secret not configured; rejecting webhook for security");
                return Err(IroncladError::Channel(
                    "WhatsApp webhook signature verification requires app_secret configuration"
                        .into(),
                ));
            }
        };

        let sig_header = signature_header
            .ok_or_else(|| IroncladError::Network("missing X-Hub-Signature-256 header".into()))?;

        let hex_sig = sig_header.strip_prefix("sha256=").ok_or_else(|| {
            IroncladError::Network("X-Hub-Signature-256 header missing sha256= prefix".into())
        })?;

        let expected_bytes = hex::decode(hex_sig).map_err(|e| {
            IroncladError::Network(format!("invalid hex in X-Hub-Signature-256: {e}"))
        })?;

        let mut mac = HmacSha256::new_from_slice(secret.as_bytes())
            .map_err(|e| IroncladError::Network(format!("HMAC init failed: {e}")))?;
        mac.update(raw_body);

        mac.verify_slice(&expected_bytes)
            .map_err(|_| IroncladError::Network("webhook signature verification failed".into()))?;

        debug!("WhatsApp webhook signature verified");
        Ok(())
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
            media_obj
                .as_object_mut()
                .unwrap()
                .insert("caption".into(), json!(cap));
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

        self.client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("mark_as_read failed: {e}")))?;

        Ok(())
    }

    /// Best-effort "typing" indicator. WhatsApp Cloud API doesn't expose a
    /// typing action, so we mark the inbound message as read (shows blue ticks)
    /// to acknowledge receipt.
    pub async fn send_typing(&self, _recipient: &str, message_id: Option<&str>) {
        if let Some(mid) = message_id {
            let _ = self.mark_as_read(mid).await;
        }
    }

    /// Send a short ephemeral text message and return its WAM ID (for later
    /// reference). Best-effort; returns None on failure.
    pub async fn send_ephemeral(&self, recipient: &str, text: &str) -> Option<String> {
        let url = self.api_url("messages");
        let body = json!({
            "messaging_product": "whatsapp",
            "to": recipient,
            "type": "text",
            "text": { "body": text },
        });
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.token)
            .json(&body)
            .send()
            .await
            .ok()?;
        let json: Value = resp.json().await.ok()?;
        json.pointer("/messages/0/id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    async fn handle_api_response(&self, resp: reqwest::Response) -> Result<Value> {
        let status = resp.status();

        if status.as_u16() == 429 {
            warn!("WhatsApp rate limit hit");
            return Err(IroncladError::Network("rate limited".into()));
        }

        let body: Value = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("response parse error: {e}")))?;

        if !status.is_success() {
            let err_msg = body
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!(status = %status, error = err_msg, "WhatsApp API error");
            return Err(IroncladError::Network(format!(
                "WhatsApp API {status}: {err_msg}"
            )));
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

        let resp = self
            .client
            .post(&url)
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
            "tok".into(),
            "ph".into(),
            "my_verify".into(),
            vec![],
            None,
        );
        let result = adapter.verify_webhook_challenge("subscribe", "my_verify", "challenge123");
        assert_eq!(result.unwrap(), "challenge123");
    }

    #[test]
    fn verify_webhook_challenge_wrong_mode() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "my_verify".into(),
            vec![],
            None,
        );
        let result = adapter.verify_webhook_challenge("unsubscribe", "my_verify", "c");
        assert!(result.is_err());
    }

    #[test]
    fn verify_webhook_challenge_wrong_token() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "my_verify".into(),
            vec![],
            None,
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
            "tok".into(),
            "ph".into(),
            "v".into(),
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
            "tok".into(),
            "ph".into(),
            "v".into(),
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
            "555",
            "image",
            "https://example.com/img.png",
            Some("caption"),
        );
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["type"], "image");
        assert_eq!(payload["to"], "555");
    }

    #[test]
    fn platform_name_is_whatsapp() {
        let adapter = WhatsAppAdapter::new("tok".into(), "ph".into());
        assert_eq!(adapter.platform_name(), "whatsapp");
    }

    #[test]
    fn api_url_formats_correctly() {
        let adapter = WhatsAppAdapter::new("tok".into(), "PHONE123".into());
        assert_eq!(
            adapter.api_url("messages"),
            "https://graph.facebook.com/v21.0/PHONE123/messages"
        );
    }

    #[test]
    fn with_config_app_secret() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "v".into(),
            vec![],
            Some("appsecret123".into()),
        );
        assert_eq!(adapter.app_secret.unwrap(), "appsecret123");
    }

    #[test]
    fn parse_video_message() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w3", "type": "video",
                    "video": {"id": "vid456", "caption": "check this"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("video:vid456"));
        assert!(msg.content.contains("check this"));
    }

    #[test]
    fn parse_audio_message() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w4", "type": "audio",
                    "audio": {"id": "aud789"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("audio:aud789"));
    }

    #[test]
    fn parse_document_message() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w5", "type": "document",
                    "document": {"id": "doc000", "caption": "receipt"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("document:doc000"));
        assert!(msg.content.contains("receipt"));
    }

    #[test]
    fn parse_unsupported_message_type() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w6", "type": "sticker"}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("unsupported message type: sticker"));
    }

    #[test]
    fn parse_inbound_missing_entry() {
        let webhook = json!({"object": "whatsapp_business_account"});
        assert!(WhatsAppAdapter::parse_inbound(&webhook).is_err());
    }

    #[test]
    fn parse_inbound_empty_entry_array() {
        let webhook = json!({"entry": []});
        assert!(WhatsAppAdapter::parse_inbound(&webhook).is_err());
    }

    #[test]
    fn parse_inbound_missing_changes() {
        let webhook = json!({"entry": [{"id": "123"}]});
        assert!(WhatsAppAdapter::parse_inbound(&webhook).is_err());
    }

    #[test]
    fn parse_inbound_missing_value() {
        let webhook = json!({"entry": [{"changes": [{"field": "messages"}]}]});
        assert!(WhatsAppAdapter::parse_inbound(&webhook).is_err());
    }

    #[test]
    fn parse_inbound_missing_messages() {
        let webhook = json!({"entry": [{"changes": [{"value": {}}]}]});
        assert!(WhatsAppAdapter::parse_inbound(&webhook).is_err());
    }

    #[test]
    fn parse_inbound_missing_from_defaults_unknown() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"id": "w7", "type": "text", "text": {"body": "hi"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert_eq!(msg.sender_id, "unknown");
    }

    #[test]
    fn parse_inbound_missing_id_generates_uuid() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "type": "text", "text": {"body": "hi"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(!msg.id.is_empty());
        assert!(msg.id.len() > 10);
    }

    #[test]
    fn parse_inbound_text_no_body_defaults_empty() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w8", "type": "text", "text": {}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert_eq!(msg.content, "");
    }

    #[test]
    fn format_media_outbound_without_caption() {
        let payload = WhatsAppAdapter::format_media_outbound(
            "555",
            "image",
            "https://example.com/img.png",
            None,
        );
        assert_eq!(payload["messaging_product"], "whatsapp");
        assert_eq!(payload["type"], "image");
        assert_eq!(payload["image"]["link"], "https://example.com/img.png");
        assert!(payload["image"].get("caption").is_none());
    }

    #[test]
    fn format_media_outbound_video_with_caption() {
        let payload = WhatsAppAdapter::format_media_outbound(
            "555",
            "video",
            "https://example.com/vid.mp4",
            Some("watch this"),
        );
        assert_eq!(payload["type"], "video");
        assert_eq!(payload["video"]["link"], "https://example.com/vid.mp4");
        assert_eq!(payload["video"]["caption"], "watch this");
    }

    #[test]
    fn parse_image_message_no_caption() {
        let webhook = json!({
            "entry": [{"changes": [{"value": {
                "messages": [{"from": "555", "id": "w10", "type": "image",
                    "image": {"id": "img999"}}]
            }}]}]
        });
        let msg = WhatsAppAdapter::parse_inbound(&webhook).unwrap();
        assert!(msg.content.contains("image:img999"));
    }

    #[test]
    fn verify_webhook_signature_valid() {
        use hmac::Mac;
        let secret = "test_app_secret";
        let body = br#"{"entry": []}"#;
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let sig = hex::encode(mac.finalize().into_bytes());

        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "v".into(),
            vec![],
            Some(secret.into()),
        );
        let result = adapter.verify_webhook_signature(body, Some(&format!("sha256={sig}")));
        assert!(result.is_ok());
    }

    #[test]
    fn verify_webhook_signature_invalid() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "v".into(),
            vec![],
            Some("real_secret".into()),
        );
        let result = adapter.verify_webhook_signature(
            b"some body",
            Some("sha256=0000000000000000000000000000000000000000000000000000000000000000"),
        );
        assert!(result.is_err());
    }

    #[test]
    fn verify_webhook_signature_missing_header() {
        let adapter = WhatsAppAdapter::with_config(
            "tok".into(),
            "ph".into(),
            "v".into(),
            vec![],
            Some("secret".into()),
        );
        let result = adapter.verify_webhook_signature(b"body", None);
        assert!(result.is_err());
    }

    #[test]
    fn verify_webhook_signature_no_secret_rejects() {
        let adapter = WhatsAppAdapter::new("tok".into(), "ph".into());
        let result = adapter.verify_webhook_signature(b"body", None);
        assert!(result.is_err());
    }
}
