use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use serde_json::{Value, json};
use tracing::{debug, error};

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

/// Signal channel adapter backed by signal-cli's JSON-RPC daemon.
///
/// signal-cli must be running in daemon mode (`signal-cli -a +NUMBER daemon --json-rpc`)
/// for this adapter to function. All API calls go through the local daemon URL.
pub struct SignalAdapter {
    pub phone_number: String,
    pub daemon_url: String,
    pub client: reqwest::Client,
    pub allowed_numbers: Vec<String>,
    /// When `true`, an empty `allowed_numbers` list denies all messages (secure default).
    /// When `false`, an empty list allows all messages (legacy behavior).
    pub deny_on_empty: bool,
    message_buffer: Arc<Mutex<VecDeque<InboundMessage>>>,
}

impl SignalAdapter {
    pub fn new(phone_number: String, daemon_url: String) -> Self {
        Self {
            phone_number,
            daemon_url,
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            allowed_numbers: Vec::new(),
            deny_on_empty: true,
            message_buffer: Arc::new(Mutex::new(VecDeque::new())),
        }
    }

    pub fn with_config(
        phone_number: String,
        daemon_url: String,
        allowed_numbers: Vec<String>,
        deny_on_empty: bool,
    ) -> Self {
        Self {
            allowed_numbers,
            deny_on_empty,
            ..Self::new(phone_number, daemon_url)
        }
    }

    fn is_sender_allowed(&self, sender: &str) -> bool {
        crate::allowlist::check_allowlist(&self.allowed_numbers, sender, self.deny_on_empty)
    }

    fn rpc_url(&self) -> &str {
        &self.daemon_url
    }

    async fn json_rpc(&self, method: &str, params: Value) -> Result<Value> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
            "id": 1,
        });

        let resp = self
            .client
            .post(self.rpc_url())
            .json(&payload)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("signal-cli RPC failed: {e}")))?;

        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("signal-cli response parse error: {e}")))?;

        if !status.is_success() {
            let err_msg = body
                .pointer("/error/message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            error!(status = %status, error = err_msg, "signal-cli RPC error");
            return Err(IroncladError::Network(format!(
                "signal-cli RPC {status}: {err_msg}"
            )));
        }

        if let Some(err) = body.get("error") {
            let msg = err
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("RPC error");
            return Err(IroncladError::Network(format!("signal-cli error: {msg}")));
        }

        Ok(body.get("result").cloned().unwrap_or(Value::Null))
    }

    pub fn parse_inbound(envelope: &Value) -> Option<InboundMessage> {
        let data_message = envelope.get("dataMessage")?;
        let message = data_message
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        if message.is_empty() {
            return None;
        }

        let sender = envelope
            .get("sourceNumber")
            .or_else(|| envelope.get("source"))
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let timestamp = data_message
            .get("timestamp")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        Some(InboundMessage {
            id: format!("{}-{}", sender, timestamp),
            platform: "signal".into(),
            sender_id: sender.to_string(),
            content: message.to_string(),
            timestamp: Utc::now(),
            metadata: Some(envelope.clone()),
        })
    }

    pub fn push_message(&self, msg: InboundMessage) {
        let mut buf = self
            .message_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        buf.push_back(msg);
    }

    pub fn process_envelope(&self, envelope: &Value) -> Option<InboundMessage> {
        let msg = Self::parse_inbound(envelope)?;

        if !self.is_sender_allowed(&msg.sender_id) {
            debug!(sender = %msg.sender_id, "ignoring Signal message from disallowed number");
            return None;
        }

        Some(msg)
    }

    pub async fn send_text(&self, recipient: &str, text: &str) -> Result<Value> {
        self.json_rpc(
            "send",
            json!({
                "recipient": [recipient],
                "message": text,
                "account": self.phone_number,
            }),
        )
        .await
    }

    /// Best-effort typing indicator. Signal doesn't expose a public "typing"
    /// API through signal-cli's JSON-RPC, so we send a short receipt action.
    /// This is a no-op if the daemon doesn't support it.
    pub async fn send_typing(&self, recipient: &str) {
        if let Err(e) = self
            .json_rpc(
                "sendTyping",
                json!({
                    "recipient": [recipient],
                    "account": self.phone_number,
                }),
            )
            .await
        {
            tracing::debug!(error = %e, "Signal typing indicator failed");
        }
    }

    /// Send a short ephemeral text message. Returns the timestamp (Signal's
    /// message identifier) on success.
    pub async fn send_ephemeral(&self, recipient: &str, text: &str) -> Option<u64> {
        let result = self.send_text(recipient, text).await.ok()?;
        result.get("timestamp").and_then(|v| v.as_u64())
    }
}

#[async_trait]
impl ChannelAdapter for SignalAdapter {
    fn platform_name(&self) -> &str {
        "signal"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let mut buf = self
            .message_buffer
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        Ok(buf.pop_front())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        self.send_typing(&msg.recipient_id).await;

        debug!(to = %msg.recipient_id, "sending Signal message");
        self.send_text(&msg.recipient_id, &msg.content).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_adapter_defaults() {
        let adapter = SignalAdapter::new("+15551234567".into(), "http://localhost:8080".into());
        assert_eq!(adapter.phone_number, "+15551234567");
        assert_eq!(adapter.daemon_url, "http://localhost:8080");
        assert!(adapter.allowed_numbers.is_empty());
        assert!(adapter.deny_on_empty);
    }

    #[test]
    fn with_config_sets_fields() {
        let adapter = SignalAdapter::with_config(
            "+1555".into(),
            "http://localhost:9090".into(),
            vec!["+1666".into()],
            false,
        );
        assert_eq!(adapter.allowed_numbers, vec!["+1666"]);
        assert!(!adapter.deny_on_empty);
    }

    #[test]
    fn sender_allowed_empty_default_denies_all() {
        // deny_on_empty=true (secure default): empty list denies everyone
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        assert!(!adapter.is_sender_allowed("+any_number"));
    }

    #[test]
    fn sender_allowed_empty_secure_denies_all() {
        // deny_on_empty=true (secure default): empty list denies everyone
        let adapter =
            SignalAdapter::with_config("+1".into(), "http://localhost:8080".into(), vec![], true);
        assert!(!adapter.is_sender_allowed("+any_number"));
    }

    #[test]
    fn sender_allowed_filters() {
        let adapter = SignalAdapter::with_config(
            "+1".into(),
            "http://localhost:8080".into(),
            vec!["+111".into(), "+222".into()],
            false,
        );
        assert!(adapter.is_sender_allowed("+111"));
        assert!(!adapter.is_sender_allowed("+333"));
    }

    #[test]
    fn parse_inbound_valid_message() {
        let envelope = json!({
            "sourceNumber": "+15559876543",
            "dataMessage": {
                "timestamp": 1700000000000_u64,
                "message": "Hello from Signal"
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        assert_eq!(msg.platform, "signal");
        assert_eq!(msg.sender_id, "+15559876543");
        assert_eq!(msg.content, "Hello from Signal");
    }

    #[test]
    fn parse_inbound_empty_message_returns_none() {
        let envelope = json!({
            "sourceNumber": "+155",
            "dataMessage": {
                "timestamp": 170000,
                "message": ""
            }
        });
        assert!(SignalAdapter::parse_inbound(&envelope).is_none());
    }

    #[test]
    fn parse_inbound_no_data_message_returns_none() {
        let envelope = json!({ "sourceNumber": "+155" });
        assert!(SignalAdapter::parse_inbound(&envelope).is_none());
    }

    #[test]
    fn process_envelope_filters_disallowed() {
        let adapter = SignalAdapter::with_config(
            "+1".into(),
            "http://localhost:8080".into(),
            vec!["+allowed".into()],
            false,
        );
        let envelope = json!({
            "sourceNumber": "+not_allowed",
            "dataMessage": {
                "timestamp": 1,
                "message": "nope"
            }
        });
        assert!(adapter.process_envelope(&envelope).is_none());
    }

    #[test]
    fn process_envelope_passes_allowed() {
        let adapter = SignalAdapter::with_config(
            "+1".into(),
            "http://localhost:8080".into(),
            vec!["+allowed".into()],
            false,
        );
        let envelope = json!({
            "sourceNumber": "+allowed",
            "dataMessage": {
                "timestamp": 1,
                "message": "ok"
            }
        });
        let msg = adapter.process_envelope(&envelope).unwrap();
        assert_eq!(msg.content, "ok");
    }

    #[test]
    fn push_and_recv_message() {
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        let msg = InboundMessage {
            id: "s1".into(),
            platform: "signal".into(),
            sender_id: "+555".into(),
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
    fn platform_name_is_signal() {
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        assert_eq!(adapter.platform_name(), "signal");
    }

    #[test]
    fn rpc_url_returns_daemon_url() {
        let adapter = SignalAdapter::new("+1".into(), "http://custom:9999".into());
        assert_eq!(adapter.rpc_url(), "http://custom:9999");
    }

    #[test]
    fn parse_inbound_source_fallback() {
        // Uses "source" instead of "sourceNumber"
        let envelope = json!({
            "source": "+15559999999",
            "dataMessage": {
                "timestamp": 1700000000000_u64,
                "message": "via source field"
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        assert_eq!(msg.sender_id, "+15559999999");
        assert_eq!(msg.content, "via source field");
    }

    #[test]
    fn parse_inbound_unknown_sender() {
        // Neither sourceNumber nor source present
        let envelope = json!({
            "dataMessage": {
                "timestamp": 1700000000000_u64,
                "message": "no sender"
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        assert_eq!(msg.sender_id, "unknown");
    }

    #[test]
    fn parse_inbound_no_timestamp_defaults_zero() {
        let envelope = json!({
            "sourceNumber": "+1555",
            "dataMessage": {
                "message": "no timestamp"
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        assert!(msg.id.contains("+1555"));
        assert!(msg.id.contains("-0"));
    }

    #[test]
    fn parse_inbound_no_message_field_defaults_empty() {
        let envelope = json!({
            "sourceNumber": "+1555",
            "dataMessage": {
                "timestamp": 100
            }
        });
        // message field missing means empty string, which returns None
        let result = SignalAdapter::parse_inbound(&envelope);
        assert!(result.is_none());
    }

    #[test]
    fn parse_inbound_id_format() {
        let envelope = json!({
            "sourceNumber": "+15551234567",
            "dataMessage": {
                "timestamp": 1234567890,
                "message": "test"
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        assert_eq!(msg.id, "+15551234567-1234567890");
    }

    #[test]
    fn parse_inbound_metadata_preserved() {
        let envelope = json!({
            "sourceNumber": "+1555",
            "dataMessage": {
                "timestamp": 100,
                "message": "meta test",
                "groupInfo": { "groupId": "abc" }
            }
        });
        let msg = SignalAdapter::parse_inbound(&envelope).unwrap();
        let meta = msg.metadata.unwrap();
        assert_eq!(meta["sourceNumber"], "+1555");
    }

    #[test]
    fn process_envelope_returns_none_for_no_data_message() {
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        let envelope = json!({"sourceNumber": "+1555"});
        assert!(adapter.process_envelope(&envelope).is_none());
    }

    #[test]
    fn process_envelope_returns_none_for_empty_message() {
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        let envelope = json!({
            "sourceNumber": "+1555",
            "dataMessage": {
                "timestamp": 1,
                "message": ""
            }
        });
        assert!(adapter.process_envelope(&envelope).is_none());
    }

    #[test]
    fn push_multiple_messages_fifo() {
        let adapter = SignalAdapter::new("+1".into(), "http://localhost:8080".into());
        for i in 0..3 {
            adapter.push_message(InboundMessage {
                id: format!("s{}", i),
                platform: "signal".into(),
                sender_id: "+555".into(),
                content: format!("msg{}", i),
                timestamp: Utc::now(),
                metadata: None,
            });
        }

        let rt = tokio::runtime::Runtime::new().unwrap();
        for i in 0..3 {
            let result = rt.block_on(adapter.recv()).unwrap().unwrap();
            assert_eq!(result.content, format!("msg{}", i));
        }
        assert!(rt.block_on(adapter.recv()).unwrap().is_none());
    }

    #[test]
    fn with_config_multiple_allowed_numbers() {
        let adapter = SignalAdapter::with_config(
            "+1".into(),
            "http://localhost:8080".into(),
            vec!["+111".into(), "+222".into(), "+333".into()],
            false,
        );
        assert!(adapter.is_sender_allowed("+111"));
        assert!(adapter.is_sender_allowed("+222"));
        assert!(adapter.is_sender_allowed("+333"));
        assert!(!adapter.is_sender_allowed("+444"));
    }

    #[test]
    fn new_adapter_fields() {
        let adapter = SignalAdapter::new("+15551234567".into(), "http://signal:7583".into());
        assert_eq!(adapter.phone_number, "+15551234567");
        assert_eq!(adapter.daemon_url, "http://signal:7583");
        assert!(adapter.allowed_numbers.is_empty());
        let buf = adapter.message_buffer.lock().unwrap();
        assert!(buf.is_empty());
    }

    // ── async method tests (exercise error paths via connection refusal) ──

    fn fast_fail_adapter() -> SignalAdapter {
        let mut adapter = SignalAdapter::new("+1555".into(), "http://127.0.0.1:1/rpc".into());
        adapter.client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .build()
            .unwrap();
        adapter
    }

    #[tokio::test]
    async fn json_rpc_network_error() {
        let adapter = fast_fail_adapter();
        let result = adapter.json_rpc("send", json!({"message": "test"})).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("signal-cli RPC failed"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn send_text_network_error() {
        let adapter = fast_fail_adapter();
        let result = adapter.send_text("+1666", "hello").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_typing_best_effort_no_panic() {
        let adapter = fast_fail_adapter();
        adapter.send_typing("+1666").await;
    }

    #[tokio::test]
    async fn send_ephemeral_returns_none_on_failure() {
        let adapter = fast_fail_adapter();
        let result = adapter.send_ephemeral("+1666", "test").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn send_trait_impl_network_error() {
        let adapter = fast_fail_adapter();
        let msg = OutboundMessage {
            content: "hello".into(),
            recipient_id: "+1666".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
    }
}
