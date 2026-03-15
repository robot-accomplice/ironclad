use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;
use ironclad_core::{IroncladError, Result};
use lettre::message::{Mailbox, MessageBuilder};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};
use mail_parser::MimeHeaders;
use serde_json::json;
use tokio::sync::Notify;
use tokio_util::compat::TokioAsyncReadCompatExt;
use tracing::{debug, error, info, warn};

use super::{ChannelAdapter, InboundMessage, OutboundMessage};

/// TLS stream type used for IMAP connections.
/// Chain: tokio TcpStream → compat (futures IO) → native-tls TLS.
type ImapTlsStream = async_native_tls::TlsStream<tokio_util::compat::Compat<tokio::net::TcpStream>>;

/// Maximum email body size (1 MB). Content beyond this limit is truncated.
const MAX_EMAIL_BODY_BYTES: usize = 1_048_576;

/// Email channel adapter for bidirectional email communication.
///
/// Supports SMTP sending (via Lettre) and IMAP receiving (via async-imap).
/// The IMAP listener can authenticate with password or XOAUTH2 (Gmail).
pub struct EmailAdapter {
    from_address: String,
    smtp_host: String,
    imap_host: String,
    imap_port: u16,
    username: String,
    password: String,
    allowed_senders: Vec<String>,
    /// When `true`, an empty `allowed_senders` list denies all messages (secure default).
    /// When `false`, an empty list allows all messages (legacy behavior).
    deny_on_empty: bool,
    buffer: Arc<Mutex<VecDeque<InboundMessage>>>,
    transport: AsyncSmtpTransport<Tokio1Executor>,
    /// IMAP polling interval (defaults to 30s if not overridden).
    poll_interval: Duration,
    /// OAuth2 access token for XOAUTH2 authentication (Gmail).
    oauth2_token: Option<String>,
    /// Whether to use IMAP IDLE when supported by the server.
    imap_idle_enabled: bool,
    /// Handle for the background IMAP listener task.
    imap_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Shutdown signal for the IMAP listener.
    shutdown: Arc<Notify>,
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
    ) -> Result<Self> {
        let creds = Credentials::new(username.clone(), password.clone());
        let transport = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&smtp_host)
            .map_err(|e| IroncladError::Config(format!("invalid SMTP relay hostname: {e}")))?
            .port(smtp_port)
            .credentials(creds)
            .build();

        Ok(Self {
            from_address,
            smtp_host,
            imap_host,
            imap_port,
            username,
            password,
            allowed_senders: Vec::new(),
            deny_on_empty: true,
            buffer: Arc::new(Mutex::new(VecDeque::new())),
            transport,
            poll_interval: Duration::from_secs(30),
            oauth2_token: None,
            imap_idle_enabled: true,
            imap_handle: Mutex::new(None),
            shutdown: Arc::new(Notify::new()),
        })
    }

    pub fn with_allowed_senders(mut self, senders: Vec<String>) -> Self {
        self.allowed_senders = senders;
        self
    }

    pub fn with_deny_on_empty(mut self, deny: bool) -> Self {
        self.deny_on_empty = deny;
        self
    }

    pub fn with_poll_interval(mut self, interval: Duration) -> Self {
        self.poll_interval = interval;
        self
    }

    pub fn with_oauth2_token(mut self, token: Option<String>) -> Self {
        self.oauth2_token = token;
        self
    }

    pub fn with_imap_idle_enabled(mut self, enabled: bool) -> Self {
        self.imap_idle_enabled = enabled;
        self
    }

    /// Check if a sender is in the allowed list (case-insensitive for email).
    ///
    /// When `deny_on_empty` is `true`, an empty list rejects all senders (secure default).
    /// When `deny_on_empty` is `false`, an empty list allows all senders (legacy behavior).
    pub fn is_sender_allowed(&self, sender: &str) -> bool {
        let lower: Vec<String> = self
            .allowed_senders
            .iter()
            .map(|s| s.to_lowercase())
            .collect();
        crate::allowlist::check_allowlist(&lower, &sender.to_lowercase(), self.deny_on_empty)
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
        let mut buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
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

    /// Start the background IMAP listener that polls for new messages.
    ///
    /// Requires `imap_host` to be configured. The listener connects via TLS,
    /// authenticates (password or XOAUTH2), and polls INBOX for unseen messages.
    pub async fn start_imap_listener(self: &Arc<Self>) -> Result<()> {
        if self.imap_host.is_empty() {
            return Err(IroncladError::Config(
                "IMAP host not configured".to_string(),
            ));
        }
        let adapter = Arc::clone(self);
        let handle = tokio::spawn(async move {
            if let Err(e) = imap_poll_loop(adapter).await {
                error!(error = %e, "IMAP listener terminated with error");
            }
        });
        *self.imap_handle.lock().unwrap_or_else(|e| e.into_inner()) = Some(handle);
        if self.imap_idle_enabled {
            warn!("IMAP IDLE requested but not yet implemented; falling back to polling");
        }
        info!(host = %self.imap_host, port = %self.imap_port, "IMAP listener started");
        Ok(())
    }

    /// Shut down the background IMAP listener gracefully.
    pub async fn shutdown_imap(&self) {
        self.shutdown.notify_waiters();
        let handle = self
            .imap_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

/// Build the XOAUTH2 SASL token for Gmail authentication.
///
/// Format: `base64("user=" + user + "\x01auth=Bearer " + token + "\x01\x01")`
pub fn build_xoauth2_token(username: &str, access_token: &str) -> String {
    use base64::Engine;
    let sasl = format!("user={}\x01auth=Bearer {}\x01\x01", username, access_token);
    base64::engine::general_purpose::STANDARD.encode(sasl.as_bytes())
}

/// Parse a raw RFC 5322 email into an InboundMessage using `mail-parser`.
///
/// Returns `None` if the message cannot be parsed or has no usable body.
/// Extracts: From, Subject, Body (text/plain preferred, text/html fallback),
/// Thread-ID (via In-Reply-To / Message-ID), and attachment metadata.
pub fn parse_email_rfc5322(
    raw: &[u8],
    allowed_senders: &[String],
    deny_on_empty: bool,
) -> Option<InboundMessage> {
    let parsed = mail_parser::MessageParser::default().parse(raw)?;

    let from = parsed
        .from()
        .and_then(|addrs| addrs.first())
        .and_then(|addr| addr.address())
        .unwrap_or("");
    if from.is_empty() {
        warn!("IMAP: skipping message with no From address");
        return None;
    }

    // Sender filtering (reuses same logic as EmailAdapter::is_sender_allowed)
    if !is_sender_allowed_static(from, allowed_senders, deny_on_empty) {
        debug!(from = from, "IMAP: sender not in allowed list, skipping");
        return None;
    }

    let subject = parsed.subject().unwrap_or("");
    let message_id = parsed.message_id().map(|s| s.to_string());
    let in_reply_to = parsed
        .in_reply_to()
        .as_text_list()
        .and_then(|list| list.first().map(|s| s.to_string()));

    // Body extraction: prefer text/plain, fall back to text/html
    let body_raw = parsed
        .body_text(0)
        .map(|s| s.to_string())
        .or_else(|| {
            parsed.body_html(0).map(|s| {
                // Strip HTML tags for a rough plaintext fallback
                strip_html_tags(&s)
            })
        })
        .unwrap_or_default();

    let body = if body_raw.len() > MAX_EMAIL_BODY_BYTES {
        warn!(
            from = from,
            original_len = body_raw.len(),
            "IMAP email body exceeds {} bytes; truncating",
            MAX_EMAIL_BODY_BYTES
        );
        let mut end = MAX_EMAIL_BODY_BYTES;
        while end > 0 && !body_raw.is_char_boundary(end) {
            end -= 1;
        }
        body_raw[..end].to_string()
    } else {
        body_raw
    };

    // Attachment metadata extraction — uses MediaAttachment for cross-channel consistency
    let attachments: Vec<super::MediaAttachment> = parsed
        .attachments()
        .map(|part| {
            let ct = part
                .content_type()
                .map(|ct| format!("{}/{}", ct.ctype(), ct.subtype().unwrap_or("octet-stream")))
                .unwrap_or_else(|| "application/octet-stream".to_string());
            super::MediaAttachment {
                media_type: super::MediaType::from_content_type(&ct),
                source_url: None,
                local_path: None,
                filename: Some(part.attachment_name().unwrap_or("unnamed").to_string()),
                content_type: ct,
                size_bytes: Some(part.contents().len()),
                caption: None,
            }
        })
        .collect();

    // DKIM-Signature presence check (lightweight; full verification deferred)
    let has_dkim = parsed.header_values("DKIM-Signature").next().is_some();

    let content = if subject.is_empty() {
        body
    } else {
        format!("[Subject: {subject}] {body}")
    };

    let thread_id = in_reply_to
        .as_deref()
        .or(message_id.as_deref())
        .unwrap_or("default")
        .to_string();

    Some(InboundMessage {
        id: message_id.clone().unwrap_or_else(|| "unknown".to_string()),
        platform: "email".to_string(),
        sender_id: from.to_string(),
        content,
        timestamp: Utc::now(),
        metadata: Some(json!({
            "subject": subject,
            "message_id": message_id,
            "in_reply_to": in_reply_to,
            "thread_id": thread_id,
            "attachments": attachments,
            "has_dkim_signature": has_dkim,
        })),
    })
}

/// Static sender-allowed check (no &self needed — usable from free functions).
fn is_sender_allowed_static(sender: &str, allowed: &[String], deny_on_empty: bool) -> bool {
    let lower: Vec<String> = allowed.iter().map(|s| s.to_lowercase()).collect();
    crate::allowlist::check_allowlist(&lower, &sender.to_lowercase(), deny_on_empty)
}

/// Rough HTML tag stripping for text/html fallback.
fn strip_html_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
}

/// Background IMAP poll loop. Connects, authenticates, and polls for unseen messages.
async fn imap_poll_loop(adapter: Arc<EmailAdapter>) -> Result<()> {
    let mut backoff_secs = 1u64;

    loop {
        match run_imap_session(&adapter).await {
            Ok(()) => {
                // Session ended normally (e.g. shutdown signal)
                info!("IMAP session ended normally");
                return Ok(());
            }
            Err(e) => {
                warn!(
                    error = %e,
                    backoff_secs = backoff_secs,
                    "IMAP session error, reconnecting"
                );
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = adapter.shutdown.notified() => {
                        info!("IMAP listener shutdown requested");
                        return Ok(());
                    }
                }
                backoff_secs = (backoff_secs * 2).min(300);
            }
        }
    }
}

/// Run a single IMAP session: connect, authenticate, poll INBOX.
async fn run_imap_session(adapter: &Arc<EmailAdapter>) -> Result<()> {
    // Establish TLS connection: tokio TCP → compat wrapper → native-tls
    let tcp = tokio::net::TcpStream::connect((adapter.imap_host.as_str(), adapter.imap_port))
        .await
        .map_err(|e| IroncladError::Channel(format!("IMAP TCP connect failed: {e}")))?;
    let tcp_compat = tcp.compat();
    let tls = async_native_tls::TlsConnector::new();
    let tls_stream: ImapTlsStream = tls
        .connect(&adapter.imap_host, tcp_compat)
        .await
        .map_err(|e| IroncladError::Channel(format!("IMAP TLS handshake failed: {e}")))?;
    let client: async_imap::Client<ImapTlsStream> = async_imap::Client::new(tls_stream);

    // Authenticate
    let mut session: async_imap::Session<ImapTlsStream> = if let Some(ref token) =
        adapter.oauth2_token
    {
        let xoauth2 = build_xoauth2_token(&adapter.username, token);
        let auth = ImapXoauth2Auth {
            token: xoauth2.clone(),
        };
        client
            .authenticate("XOAUTH2", auth)
            .await
            .map_err(|(e, _)| IroncladError::Channel(format!("IMAP XOAUTH2 auth failed: {e}")))?
    } else {
        client
            .login(&adapter.username, &adapter.password)
            .await
            .map_err(|(e, _)| IroncladError::Channel(format!("IMAP login failed: {e}")))?
    };

    info!("IMAP authenticated successfully");

    // Select INBOX
    session
        .select("INBOX")
        .await
        .map_err(|e| IroncladError::Channel(format!("IMAP SELECT INBOX failed: {e}")))?;

    // Poll loop
    loop {
        // Search for unseen messages using UIDs (not sequence numbers).
        // UIDs are stable across concurrent modifications, unlike sequence numbers
        // which shift when other messages are deleted.
        let unseen = session
            .uid_search("UNSEEN")
            .await
            .map_err(|e| IroncladError::Channel(format!("IMAP UID SEARCH failed: {e}")))?;

        if !unseen.is_empty() {
            let uid_list: String = unseen
                .iter()
                .map(|uid: &u32| uid.to_string())
                .collect::<Vec<_>>()
                .join(",");

            debug!(count = unseen.len(), "IMAP fetching unseen messages by UID");

            let fetches = session
                .uid_fetch(&uid_list, "RFC822")
                .await
                .map_err(|e| IroncladError::Channel(format!("IMAP UID FETCH failed: {e}")))?;

            let mut fetch_vec: Vec<async_imap::types::Fetch> = Vec::new();
            {
                let mut stream = std::pin::pin!(fetches);
                while let Some(item) = stream.next().await {
                    if let Ok(fetch) = item {
                        fetch_vec.push(fetch);
                    }
                }
            }

            for fetch in &fetch_vec {
                if let Some(body) = fetch.body()
                    && let Some(msg) =
                        parse_email_rfc5322(body, &adapter.allowed_senders, adapter.deny_on_empty)
                {
                    debug!(
                        from = %msg.sender_id,
                        id = %msg.id,
                        "IMAP received email"
                    );
                    adapter.push_message(msg);
                }
            }

            // Mark fetched messages as Seen using UID STORE for consistency
            let store_result = session
                .uid_store(&uid_list, "+FLAGS (\\Seen)")
                .await
                .map_err(|e| IroncladError::Channel(format!("IMAP UID STORE flags failed: {e}")))?;
            // Consume the store response stream to completion
            let mut store_stream = std::pin::pin!(store_result);
            while store_stream.next().await.is_some() {}
        }

        // Wait for poll interval or shutdown signal
        tokio::select! {
            _ = tokio::time::sleep(adapter.poll_interval) => {}
            _ = adapter.shutdown.notified() => {
                info!("IMAP shutdown during poll wait");
                let _ = session.logout().await;
                return Ok(());
            }
        }
    }
}

/// XOAUTH2 authenticator wrapper for async-imap.
struct ImapXoauth2Auth {
    token: String,
}

impl async_imap::Authenticator for ImapXoauth2Auth {
    type Response = String;
    fn process(&mut self, _data: &[u8]) -> Self::Response {
        self.token.clone()
    }
}

#[async_trait]
impl ChannelAdapter for EmailAdapter {
    fn platform_name(&self) -> &str {
        "email"
    }

    async fn recv(&self) -> Result<Option<InboundMessage>> {
        let mut buf = self.buffer.lock().unwrap_or_else(|e| e.into_inner());
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
        .expect("test adapter")
    }

    #[test]
    fn platform_name_is_email() {
        let adapter = test_adapter();
        assert_eq!(adapter.platform_name(), "email");
        assert!(adapter.deny_on_empty);
    }

    #[test]
    fn is_sender_allowed_empty_default_denies_all() {
        // deny_on_empty=true (secure default): empty list denies everyone
        let adapter = test_adapter();
        assert!(!adapter.is_sender_allowed("anyone@example.com"));
    }

    #[test]
    fn is_sender_allowed_empty_secure_denies_all() {
        // deny_on_empty=true (secure default): empty list denies everyone
        let adapter = test_adapter().with_deny_on_empty(true);
        assert!(!adapter.is_sender_allowed("anyone@example.com"));
    }

    #[test]
    fn is_sender_allowed_filters() {
        let adapter = test_adapter().with_allowed_senders(vec!["boss@example.com".into()]);
        assert!(adapter.is_sender_allowed("boss@example.com"));
        assert!(adapter.is_sender_allowed("BOSS@EXAMPLE.COM"));
        assert!(!adapter.is_sender_allowed("stranger@example.com"));
    }

    #[test]
    fn is_sender_allowed_filters_with_deny_on_empty() {
        // deny_on_empty doesn't affect non-empty lists
        let adapter = test_adapter()
            .with_allowed_senders(vec!["boss@example.com".into()])
            .with_deny_on_empty(true);
        assert!(adapter.is_sender_allowed("boss@example.com"));
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

    #[test]
    fn format_reply_without_in_reply_to() {
        let adapter = test_adapter();
        let reply = adapter.format_reply("user@example.com", "Hello!", None);
        assert!(reply["in_reply_to"].is_null());
        assert_eq!(reply["subject"], "Re: Agent Response");
    }

    #[test]
    fn parse_email_with_message_id_and_in_reply_to() {
        let msg = EmailAdapter::parse_email(
            "alice@example.com",
            "Re: Topic",
            "Reply body",
            Some("<msg-1@example.com>"),
            Some("<orig@example.com>"),
        );
        let meta = msg.metadata.unwrap();
        assert_eq!(meta["message_id"], "<msg-1@example.com>");
        assert_eq!(meta["in_reply_to"], "<orig@example.com>");
    }

    #[test]
    fn parse_email_without_message_id() {
        let msg = EmailAdapter::parse_email("alice@example.com", "Test", "Body", None, None);
        assert_eq!(msg.id, "unknown");
    }

    #[test]
    fn parse_email_truncates_large_body() {
        let large_body = "x".repeat(MAX_EMAIL_BODY_BYTES + 1000);
        let msg = EmailAdapter::parse_email("a@b.com", "Big", &large_body, Some("<id>"), None);
        assert!(msg.content.len() <= MAX_EMAIL_BODY_BYTES + 100); // +100 for subject prefix
    }

    #[test]
    fn parse_email_large_body_at_boundary() {
        let exact_body = "y".repeat(MAX_EMAIL_BODY_BYTES);
        let msg = EmailAdapter::parse_email("a@b.com", "", &exact_body, Some("<id>"), None);
        assert_eq!(msg.content.len(), MAX_EMAIL_BODY_BYTES);
    }

    #[test]
    fn buffer_handle_shared() {
        let adapter = test_adapter();
        let handle = adapter.buffer_handle();
        let msg = EmailAdapter::parse_email("test@example.com", "S", "B", Some("<id>"), None);
        adapter.push_message(msg);
        let buf = handle.lock().unwrap();
        assert_eq!(buf.len(), 1);
    }

    #[test]
    fn parse_email_empty_body_with_subject() {
        let msg = EmailAdapter::parse_email("a@b.com", "Subject Only", "", Some("<id>"), None);
        assert!(msg.content.contains("Subject Only"));
    }

    #[test]
    fn parse_email_empty_both() {
        let msg = EmailAdapter::parse_email("a@b.com", "", "", Some("<id>"), None);
        assert!(msg.content.is_empty());
    }

    #[test]
    fn is_sender_allowed_case_insensitive() {
        let adapter = test_adapter().with_allowed_senders(vec!["Alice@Example.COM".into()]);
        assert!(adapter.is_sender_allowed("alice@example.com"));
        assert!(adapter.is_sender_allowed("ALICE@EXAMPLE.COM"));
        assert!(!adapter.is_sender_allowed("bob@example.com"));
    }

    #[test]
    fn parse_email_truncates_multibyte_body() {
        // Create a body that is over the limit and contains multi-byte chars
        // so the char boundary loop (lines 95-96) is exercised.
        let prefix = "x".repeat(MAX_EMAIL_BODY_BYTES - 2);
        // Append a 3-byte UTF-8 char that straddles the boundary
        let body = format!("{prefix}\u{2603}\u{2603}\u{2603}"); // snowman is 3 bytes
        assert!(body.len() > MAX_EMAIL_BODY_BYTES);
        let msg = EmailAdapter::parse_email("a@b.com", "", &body, Some("<id>"), None);
        assert!(msg.content.len() <= MAX_EMAIL_BODY_BYTES);
        // Verify the truncation didn't split a multi-byte char
        assert!(msg.content.is_char_boundary(msg.content.len()));
    }

    #[tokio::test]
    async fn send_fails_with_smtp_error() {
        // test_adapter points at smtp.example.com which is unresolvable
        let adapter = test_adapter();
        let msg = OutboundMessage {
            content: "hello".into(),
            recipient_id: "bob@example.com".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SMTP send failed"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn send_with_in_reply_to() {
        let adapter = test_adapter();
        let msg = OutboundMessage {
            content: "reply content".into(),
            recipient_id: "bob@example.com".into(),
            metadata: Some(serde_json::json!({"in_reply_to": "<orig@example.com>"})),
        };
        let result = adapter.send(msg).await;
        // Should fail at SMTP level, not at message building
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("SMTP send failed"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn send_with_invalid_from_address() {
        // This tests the from_address parse error path (line 184)
        let adapter = EmailAdapter::new(
            "not-an-email".into(),
            "smtp.example.com".into(),
            587,
            "imap.example.com".into(),
            993,
            "user".into(),
            "pass".into(),
        )
        .expect("test adapter");
        let msg = OutboundMessage {
            content: "test".into(),
            recipient_id: "bob@example.com".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid from address"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn send_with_invalid_to_address() {
        let adapter = test_adapter();
        let msg = OutboundMessage {
            content: "test".into(),
            recipient_id: "not-an-email".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("invalid to address"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn recv_returns_none_when_buffer_empty() {
        let adapter = test_adapter();
        let result = adapter.recv().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn recv_returns_buffered_message() {
        let adapter = test_adapter();
        let msg = EmailAdapter::parse_email("test@example.com", "Hi", "Body", Some("<id>"), None);
        adapter.push_message(msg);
        let result = adapter.recv().await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().sender_id, "test@example.com");
    }

    // ── Phase 2: IMAP / RFC 5322 tests ─────────────────────────────

    #[test]
    fn xoauth2_token_format() {
        let token = build_xoauth2_token("user@gmail.com", "ya29.access-token");
        // Decode and verify the SASL XOAUTH2 format
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&token)
            .expect("valid base64");
        let sasl = String::from_utf8(decoded).expect("valid utf-8");
        assert!(sasl.starts_with("user=user@gmail.com\x01auth=Bearer ya29.access-token\x01\x01"));
    }

    #[test]
    fn xoauth2_token_with_empty_inputs() {
        // Must not panic even with empty strings
        let token = build_xoauth2_token("", "");
        use base64::Engine;
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(&token)
            .expect("valid base64");
        let sasl = String::from_utf8(decoded).expect("valid utf-8");
        assert_eq!(sasl, "user=\x01auth=Bearer \x01\x01");
    }

    /// Minimal RFC 5322 message with text/plain body.
    fn rfc5322_plain() -> Vec<u8> {
        b"From: alice@example.com\r\n\
          To: agent@example.com\r\n\
          Subject: Test Subject\r\n\
          Message-ID: <msg-123@example.com>\r\n\
          Date: Mon, 01 Jan 2024 00:00:00 +0000\r\n\
          \r\n\
          Hello from Alice!"
            .to_vec()
    }

    #[test]
    fn rfc5322_parse_plain_text() {
        let msg = parse_email_rfc5322(&rfc5322_plain(), &[], false).expect("should parse");
        assert_eq!(msg.platform, "email");
        assert_eq!(msg.sender_id, "alice@example.com");
        assert!(msg.content.contains("Test Subject"));
        assert!(msg.content.contains("Hello from Alice!"));
        // mail-parser strips angle brackets from Message-ID
        assert_eq!(msg.id, "msg-123@example.com");
        let meta = msg.metadata.unwrap();
        assert_eq!(meta["subject"], "Test Subject");
        assert_eq!(meta["message_id"], "msg-123@example.com");
        assert_eq!(meta["has_dkim_signature"], false);
    }

    #[test]
    fn rfc5322_sender_filtering_deny_on_empty() {
        // deny_on_empty=true with empty allowed list → rejects all
        let result = parse_email_rfc5322(&rfc5322_plain(), &[], true);
        assert!(result.is_none());
    }

    #[test]
    fn rfc5322_sender_filtering_allowed_list() {
        let allowed = vec!["alice@example.com".to_string()];
        let msg = parse_email_rfc5322(&rfc5322_plain(), &allowed, true).expect("should parse");
        assert_eq!(msg.sender_id, "alice@example.com");

        // Different sender not in list
        let raw = b"From: bob@example.com\r\nSubject: Hi\r\n\r\nHello";
        let result = parse_email_rfc5322(raw, &allowed, true);
        assert!(result.is_none());
    }

    #[test]
    fn rfc5322_thread_id_from_in_reply_to() {
        let raw = b"From: alice@example.com\r\n\
                    Message-ID: <msg-2@example.com>\r\n\
                    In-Reply-To: <orig-1@example.com>\r\n\
                    Subject: Re: Thread\r\n\
                    \r\n\
                    Reply body";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        let meta = msg.metadata.unwrap();
        // Thread ID should prefer In-Reply-To over Message-ID
        // mail-parser strips angle brackets from header values
        assert_eq!(meta["thread_id"], "orig-1@example.com");
    }

    #[test]
    fn rfc5322_thread_id_from_message_id_only() {
        let raw = b"From: alice@example.com\r\n\
                    Message-ID: <msg-solo@example.com>\r\n\
                    Subject: New thread\r\n\
                    \r\n\
                    Starting fresh";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        let meta = msg.metadata.unwrap();
        assert_eq!(meta["thread_id"], "msg-solo@example.com");
    }

    #[test]
    fn rfc5322_html_body_fallback() {
        let raw = b"From: alice@example.com\r\n\
                    Subject: HTML\r\n\
                    Content-Type: text/html\r\n\
                    \r\n\
                    <html><body><p>Hello <b>World</b></p></body></html>";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        // HTML tags should be stripped
        assert!(msg.content.contains("Hello World"));
        assert!(!msg.content.contains("<html>"));
        assert!(!msg.content.contains("<b>"));
    }

    #[test]
    fn rfc5322_no_from_address_returns_none() {
        let raw = b"Subject: No From\r\n\r\nBody text";
        let result = parse_email_rfc5322(raw, &[], false);
        assert!(result.is_none());
    }

    #[test]
    fn rfc5322_dkim_signature_detection() {
        let raw = b"From: alice@example.com\r\n\
                    DKIM-Signature: v=1; a=rsa-sha256; d=example.com\r\n\
                    Subject: Signed\r\n\
                    \r\n\
                    Signed body";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        let meta = msg.metadata.unwrap();
        assert_eq!(meta["has_dkim_signature"], true);
    }

    #[test]
    fn rfc5322_attachment_metadata() {
        // Build a multipart/mixed message with an attachment
        let raw = b"From: alice@example.com\r\n\
                    Subject: With attachment\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/mixed; boundary=\"boundary42\"\r\n\
                    \r\n\
                    --boundary42\r\n\
                    Content-Type: text/plain\r\n\
                    \r\n\
                    Main body text\r\n\
                    --boundary42\r\n\
                    Content-Type: application/pdf; name=\"report.pdf\"\r\n\
                    Content-Disposition: attachment; filename=\"report.pdf\"\r\n\
                    Content-Transfer-Encoding: base64\r\n\
                    \r\n\
                    dGVzdA==\r\n\
                    --boundary42--";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        let meta = msg.metadata.unwrap();
        let attachments = meta["attachments"].as_array().expect("attachments array");
        assert!(!attachments.is_empty());
        let first = &attachments[0];
        assert_eq!(first["filename"], "report.pdf");
        assert_eq!(first["content_type"], "application/pdf");
        assert_eq!(first["media_type"], "document");
        assert!(first["size_bytes"].as_u64().unwrap() > 0);
    }

    #[test]
    fn rfc5322_body_truncation() {
        // Build an email with a body exceeding MAX_EMAIL_BODY_BYTES
        let big_body = "x".repeat(MAX_EMAIL_BODY_BYTES + 500);
        let raw = format!(
            "From: alice@example.com\r\nSubject: Big\r\n\r\n{}",
            big_body
        );
        let msg = parse_email_rfc5322(raw.as_bytes(), &[], false).expect("should parse");
        // Content includes "[Subject: Big] " prefix, but body portion should be truncated
        assert!(msg.content.len() <= MAX_EMAIL_BODY_BYTES + 50);
    }

    #[test]
    fn rfc5322_no_body_returns_empty_content() {
        let raw = b"From: alice@example.com\r\nSubject: Headers Only\r\n\r\n";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        // Should still parse — content will just be the subject prefix
        assert!(msg.content.contains("Headers Only"));
    }

    #[test]
    fn strip_html_tags_basic() {
        assert_eq!(strip_html_tags("<p>Hello</p>"), "Hello");
        assert_eq!(
            strip_html_tags("<html><body><b>Bold</b> text</body></html>"),
            "Bold text"
        );
        assert_eq!(strip_html_tags("no tags here"), "no tags here");
        assert_eq!(strip_html_tags(""), "");
        assert_eq!(strip_html_tags("<>"), "");
    }

    #[test]
    fn is_sender_allowed_static_cases() {
        // Empty list + deny=false → allow all
        assert!(is_sender_allowed_static("anyone@x.com", &[], false));
        // Empty list + deny=true → deny all
        assert!(!is_sender_allowed_static("anyone@x.com", &[], true));
        // Specific list → case-insensitive match
        let list = vec!["Alice@Example.COM".to_string()];
        assert!(is_sender_allowed_static("alice@example.com", &list, true));
        assert!(!is_sender_allowed_static("bob@example.com", &list, true));
    }

    #[test]
    fn rfc5322_multipart_alternative_prefers_plain() {
        let raw = b"From: alice@example.com\r\n\
                    Subject: Multi\r\n\
                    MIME-Version: 1.0\r\n\
                    Content-Type: multipart/alternative; boundary=\"alt99\"\r\n\
                    \r\n\
                    --alt99\r\n\
                    Content-Type: text/plain\r\n\
                    \r\n\
                    Plain text version\r\n\
                    --alt99\r\n\
                    Content-Type: text/html\r\n\
                    \r\n\
                    <html><body>HTML version</body></html>\r\n\
                    --alt99--";
        let msg = parse_email_rfc5322(raw, &[], false).expect("should parse");
        // text/plain should be preferred over text/html
        assert!(msg.content.contains("Plain text version"));
        assert!(!msg.content.contains("HTML version"));
    }

    #[test]
    fn rfc5322_garbage_bytes_returns_none() {
        // Random garbage should not panic — just return None
        let garbage = vec![0xFF, 0xFE, 0x00, 0x01, 0x80, 0x90, 0xAB];
        let result = parse_email_rfc5322(&garbage, &[], false);
        assert!(result.is_none());
    }

    #[test]
    fn builder_methods() {
        let adapter = test_adapter()
            .with_poll_interval(Duration::from_secs(60))
            .with_oauth2_token(Some("tok123".to_string()))
            .with_imap_idle_enabled(false);
        assert_eq!(adapter.poll_interval, Duration::from_secs(60));
        assert_eq!(adapter.oauth2_token.as_deref(), Some("tok123"));
        assert!(!adapter.imap_idle_enabled);
    }
}
