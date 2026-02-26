use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use chrono::Utc;
use ironclad_core::{IroncladError, Result};
use lettre::message::{Mailbox, MessageBuilder};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use serde_json::json;
use tracing::{debug, warn};

use super::{ChannelAdapter, InboundMessage, OutboundMessage};

/// Maximum email body size (1 MB). Content beyond this limit is truncated.
const MAX_EMAIL_BODY_BYTES: usize = 1_048_576;

/// Email channel adapter for bidirectional email communication.
pub struct EmailAdapter {
    from_address: String,
    smtp_host: String,
    #[allow(dead_code)]
    imap_host: String,
    #[allow(dead_code)]
    imap_port: u16,
    allowed_senders: Vec<String>,
    buffer: Arc<Mutex<VecDeque<InboundMessage>>>,
    transport: AsyncSmtpTransport<Tokio1Executor>,
}

impl EmailAdapter {
    pub fn new(
        from_address: String,
        smtp_host: String,
        smtp_port: u16,
        imap_host: String,
        imap_port: u16,
        username: String,
        password: String,
    ) -> Self {
        let creds = Credentials::new(username, password);
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)
            .expect("valid SMTP relay hostname")
            .port(smtp_port)
            .credentials(creds)
            .build();

        Self {
            from_address,
            smtp_host,
            imap_host,
            imap_port,
            allowed_senders: Vec::new(),
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            transport,
        }
    }

    pub fn with_allowed_senders(mut self, senders: Vec<String>) -> Self {
        self.allowed_senders = senders;
        self
    }

    /// Check if a sender is in the allowed list (empty list = allow all).
    pub fn is_sender_allowed(&self, sender: &str) -> bool {
        if self.allowed_senders.is_empty() {
            return true;
        }
        let sender_lower = sender.to_lowercase();
        self.allowed_senders
            .iter()
            .any(|s| s.to_lowercase() == sender_lower)
    }

    /// Parse a raw email into an InboundMessage.
    ///
    /// Body content exceeding `MAX_EMAIL_BODY_BYTES` (1 MB) is truncated to
    /// prevent excessive memory use from oversized messages.
    pub fn parse_email(
        from: &str,
        subject: &str,
        body: &str,
        message_id: Option<&str>,
        in_reply_to: Option<&str>,
    ) -> InboundMessage {
        let truncated_body = if body.len() > MAX_EMAIL_BODY_BYTES {
            warn!(
                from = from,
                original_len = body.len(),
                "email body exceeds {} bytes; truncating",
                MAX_EMAIL_BODY_BYTES
            );
            // Truncate at a char boundary to avoid splitting a multi-byte character.
            let mut end = MAX_EMAIL_BODY_BYTES;
            while end > 0 && !body.is_char_boundary(end) {
                end -= 1;
            }
            &body[..end]
        } else {
            body
        };

        let content = if subject.is_empty() {
            truncated_body.to_string()
        } else {
            format!("[Subject: {subject}] {truncated_body}")
        };

        InboundMessage {
            id: message_id.unwrap_or("unknown").to_string(),
            platform: "email".to_string(),
            sender_id: from.to_string(),
            content,
            timestamp: Utc::now(),
            metadata: Some(json!({
                "subject": subject,
                "message_id": message_id,
                "in_reply_to": in_reply_to,
            })),
        }
    }

    /// Extract a thread ID from email headers for session mapping.
    pub fn thread_id(message_id: Option<&str>, in_reply_to: Option<&str>) -> String {
        in_reply_to.or(message_id).unwrap_or("default").to_string()
    }

    /// Push a parsed email into the receive buffer.
    pub fn push_message(&self, msg: InboundMessage) {
        let mut buf = self.buffer.lock().expect("mutex poisoned");
        buf.push_back(msg);
    }

    /// Get a clone of the buffer handle for external use.
    pub fn buffer_handle(&self) -> Arc<Mutex<VecDeque<InboundMessage>>> {
        Arc::clone(&self.buffer)
    }

    /// Build an outbound email body with threading headers.
    pub fn format_reply(
        &self,
        to: &str,
        content: &str,
        in_reply_to: Option<&str>,
    ) -> serde_json::Value {
        json!({
            "from": self.from_address,
            "to": to,
            "subject": "Re: Agent Response",
            "body": content,
            "in_reply_to": in_reply_to,
            "message_id": format!("<{}.ironclad@{}>", uuid::Uuid::new_v4(), self.smtp_host),
        })
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn platform_name(&self) -> &str {
        "email"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let mut buf = self.buffer.lock().expect("mutex poisoned");
        Ok(buf.pop_front())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        debug!(
            to = %msg.recipient_id,
            len = msg.content.len(),
            "email send requested"
        );

        let in_reply_to = msg
            .metadata
            .as_ref()
            .and_then(|m| m.get("in_reply_to"))
            .and_then(|v| v.as_str());

        let from_mailbox: Mailbox = self
            .from_address
            .parse()
            .map_err(|e| IroncladError::Channel(format!("invalid from address: {e}")))?;
        let to_mailbox: Mailbox = msg
            .recipient_id
            .parse()
            .map_err(|e| IroncladError::Channel(format!("invalid to address: {e}")))?;

        let message_id = format!("<{}.ironclad@{}>", uuid::Uuid::new_v4(), self.smtp_host);

        let mut builder: MessageBuilder = lettre::Message::builder()
            .from(from_mailbox)
            .to(to_mailbox)
            .subject("Re: Agent Response")
            .message_id(Some(message_id));

        if let Some(reply_id) = in_reply_to {
            builder = builder.in_reply_to(reply_id.to_string());
        }

        let email = builder
            .body(msg.content.clone())
            .map_err(|e| IroncladError::Channel(format!("failed to build email: {e}")))?;

        self.transport
            .send(email)
            .await
            .map_err(|e| IroncladError::Channel(format!("SMTP send failed: {e}")))?;

        debug!(to = %msg.recipient_id, "email sent successfully");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_adapter() -> EmailAdapter {
        EmailAdapter::new(
            "agent@example.com".into(),
            "smtp.example.com".into(),
            587,
            "imap.example.com".into(),
            993,
            "agent@example.com".into(),
            "password".into(),
        )
    }

    #[test]
    fn platform_name_is_email() {
        let adapter = test_adapter();
        assert_eq!(adapter.platform_name(), "email");
    }

    #[test]
    fn is_sender_allowed_empty_list_allows_all() {
        let adapter = test_adapter();
        assert!(adapter.is_sender_allowed("anyone@example.com"));
    }

    #[test]
    fn is_sender_allowed_filters() {
        let adapter = test_adapter().with_allowed_senders(vec!["boss@example.com".into()]);
        assert!(adapter.is_sender_allowed("boss@example.com"));
        assert!(adapter.is_sender_allowed("BOSS@EXAMPLE.COM"));
        assert!(!adapter.is_sender_allowed("stranger@example.com"));
    }

    #[test]
    fn parse_email_with_subject() {
        let msg = EmailAdapter::parse_email(
            "alice@example.com",
            "Hello",
            "How are you?",
            Some("<msg-1@example.com>"),
            None,
        );
        assert_eq!(msg.platform, "email");
        assert_eq!(msg.sender_id, "alice@example.com");
        assert!(msg.content.contains("Hello"));
        assert!(msg.content.contains("How are you?"));
        assert_eq!(msg.metadata.as_ref().unwrap()["subject"], "Hello");
    }

    #[test]
    fn parse_email_without_subject() {
        let msg = EmailAdapter::parse_email("alice@example.com", "", "Just the body", None, None);
        assert_eq!(msg.content, "Just the body");
    }

    #[test]
    fn thread_id_from_in_reply_to() {
        let tid = EmailAdapter::thread_id(Some("<orig>"), Some("<reply>"));
        assert_eq!(tid, "<reply>");
    }

    #[test]
    fn thread_id_from_message_id() {
        let tid = EmailAdapter::thread_id(Some("<orig>"), None);
        assert_eq!(tid, "<orig>");
    }

    #[test]
    fn thread_id_default() {
        let tid = EmailAdapter::thread_id(None, None);
        assert_eq!(tid, "default");
    }

    #[test]
    fn buffer_push_and_recv() {
        let adapter = test_adapter();
        let msg = EmailAdapter::parse_email("test@example.com", "Test", "Body", Some("<id>"), None);
        adapter.push_message(msg);

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let received = rt.block_on(adapter.recv()).unwrap();
        assert!(received.is_some());
        assert_eq!(received.unwrap().sender_id, "test@example.com");
    }

    #[tokio::test]
    async fn recv_empty_buffer_returns_none() {
        let adapter = test_adapter();
        let msg = adapter.recv().await.unwrap();
        assert!(msg.is_none());
    }

    #[test]
    fn format_reply_has_required_fields() {
        let adapter = test_adapter();
        let reply = adapter.format_reply("user@example.com", "Hello!", Some("<orig-id>"));
        assert_eq!(reply["from"], "agent@example.com");
        assert_eq!(reply["to"], "user@example.com");
        assert_eq!(reply["body"], "Hello!");
        assert_eq!(reply["in_reply_to"], "<orig-id>");
        assert!(reply["message_id"].as_str().unwrap().contains("ironclad"));
    }
}
