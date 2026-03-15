use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use ironclad_core::{IroncladError, Result};
use rand::Rng;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio::sync::Notify;
use tokio_tungstenite::tungstenite::Message as WsMessage;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";
const MAX_MESSAGE_LEN: usize = 2000;
const GATEWAY_VERSION: &str = "10";
const GATEWAY_ENCODING: &str = "json";

type WsStream = WebSocketStream<MaybeTlsStream<TcpStream>>;
type WsSink = SplitSink<WsStream, WsMessage>;
#[allow(dead_code)]
type WsSource = SplitStream<WsStream>;

pub struct DiscordAdapter {
    pub token: String,
    pub client: reqwest::Client,
    pub allowed_guild_ids: Vec<String>,
    /// When `true`, an empty `allowed_guild_ids` list denies all messages (secure default).
    /// When `false`, an empty list allows all messages (legacy behavior).
    pub deny_on_empty: bool,
    message_buffer: Arc<Mutex<VecDeque<InboundMessage>>>,
    gateway_connection: Arc<GatewayConnection>,
    gateway_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    shutdown: Arc<Notify>,
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
            deny_on_empty: true,
            message_buffer: crate::helpers::new_message_buffer(),
            gateway_connection: Arc::new(GatewayConnection::new()),
            gateway_handle: Mutex::new(None),
            shutdown: Arc::new(Notify::new()),
        }
    }

    pub fn with_config(token: String, allowed_guild_ids: Vec<String>, deny_on_empty: bool) -> Self {
        Self {
            allowed_guild_ids,
            deny_on_empty,
            ..Self::new(token)
        }
    }

    pub fn buffer_handle(&self) -> Arc<Mutex<VecDeque<InboundMessage>>> {
        Arc::clone(&self.message_buffer)
    }

    fn is_guild_allowed(&self, guild_id: &str) -> bool {
        if self.allowed_guild_ids.is_empty() {
            return !self.deny_on_empty;
        }
        self.allowed_guild_ids.iter().any(|g| g == guild_id)
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

        // Extract Discord attachments into structured MediaAttachment objects
        let attachments: Vec<super::MediaAttachment> = data
            .get("attachments")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|att| {
                        let url = att.get("url").and_then(|v| v.as_str())?;
                        let ct = att
                            .get("content_type")
                            .and_then(|v| v.as_str())
                            .unwrap_or("application/octet-stream");
                        Some(super::MediaAttachment {
                            media_type: super::MediaType::from_content_type(ct),
                            source_url: Some(url.to_string()),
                            local_path: None,
                            filename: att
                                .get("filename")
                                .and_then(|v| v.as_str())
                                .map(|s| s.to_string()),
                            content_type: ct.to_string(),
                            size_bytes: att
                                .get("size")
                                .and_then(|v| v.as_u64())
                                .map(|s| s as usize),
                            caption: None,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut metadata = json!({ "channel_id": channel_id });
        if !attachments.is_empty() {
            metadata["attachments"] = serde_json::to_value(&attachments).unwrap_or_default();
        }

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
            metadata: Some(metadata),
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

    /// Send a typing indicator to a Discord channel. The indicator lasts ~10s
    /// or until a message is sent. Best-effort; errors are silently ignored.
    pub async fn send_typing(&self, channel_id: &str) {
        let url = format!("{}/channels/{}/typing", DISCORD_API_BASE, channel_id);
        if let Err(e) = self
            .client
            .post(&url)
            .header("Authorization", format!("Bot {}", self.token))
            .send()
            .await
        {
            tracing::debug!(error = %e, "Discord typing indicator failed");
        }
    }

    /// Send a short ephemeral message and return its message ID. Best-effort.
    pub async fn send_ephemeral(&self, channel_id: &str, text: &str) -> Option<String> {
        let resp = self.send_message(channel_id, text).await.ok()?;
        resp.get("id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
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
        crate::helpers::chunk_message(text, max_len)
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
        Ok(crate::helpers::recv_from_buffer(&self.message_buffer))
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
    heartbeat_interval_ms: Mutex<u64>,
    sequence: Arc<Mutex<Option<u64>>>,
    session_id: Arc<Mutex<Option<String>>>,
    resume_gateway_url: Mutex<Option<String>>,
    heartbeat_acked: AtomicBool,
}

impl Default for GatewayConnection {
    fn default() -> Self {
        Self::new()
    }
}

impl GatewayConnection {
    pub fn new() -> Self {
        Self {
            heartbeat_interval_ms: Mutex::new(41250),
            sequence: Arc::new(Mutex::new(None)),
            session_id: Arc::new(Mutex::new(None)),
            resume_gateway_url: Mutex::new(None),
            heartbeat_acked: AtomicBool::new(true),
        }
    }

    pub fn heartbeat_interval_ms(&self) -> u64 {
        *self
            .heartbeat_interval_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_heartbeat_interval_ms(&self, ms: u64) {
        *self
            .heartbeat_interval_ms
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = ms;
    }

    pub fn sequence(&self) -> Option<u64> {
        *self.sequence.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn set_sequence(&self, seq: Option<u64>) {
        *self.sequence.lock().unwrap_or_else(|e| e.into_inner()) = seq;
    }

    pub fn session_id(&self) -> Option<String> {
        self.session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_session_id(&self, id: String) {
        *self.session_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(id);
    }

    pub fn clear_session(&self) {
        *self.session_id.lock().unwrap_or_else(|e| e.into_inner()) = None;
        *self
            .resume_gateway_url
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn resume_gateway_url(&self) -> Option<String> {
        self.resume_gateway_url
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn set_resume_gateway_url(&self, url: String) {
        *self
            .resume_gateway_url
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(url);
    }

    pub fn heartbeat_acked(&self) -> bool {
        self.heartbeat_acked.load(Ordering::Acquire)
    }

    pub fn set_heartbeat_acked(&self, acked: bool) {
        self.heartbeat_acked.store(acked, Ordering::Release);
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
        // Sequence number is extracted separately by the caller (gateway_loop)
        // to update GatewayConnection state before dispatch processing.

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

    /// Connect to the Discord WebSocket Gateway for real-time event reception.
    ///
    /// Spawns a background task that maintains the gateway connection, handles
    /// reconnection with exponential backoff, and pushes inbound messages to the
    /// adapter's buffer (consumed by the existing poll loop).
    pub async fn connect_gateway(self: &Arc<Self>) -> Result<()> {
        let adapter = Arc::clone(self);
        let handle = tokio::spawn(async move {
            if let Err(e) = gateway_loop(adapter).await {
                error!(error = %e, "Discord gateway loop terminated with error");
            }
        });

        *self
            .gateway_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(handle);
        info!("Discord gateway connection started");
        Ok(())
    }

    /// Gracefully shut down the gateway connection.
    pub async fn shutdown_gateway(&self) {
        self.shutdown.notify_waiters();
        let handle = self
            .gateway_handle
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take();
        if let Some(handle) = handle {
            let _ = handle.await;
        }
    }
}

// ── Gateway WebSocket implementation ─────────────────────────────────

/// Outcome of a single gateway session, used to decide reconnection strategy.
enum GatewayAction {
    /// Fresh reconnect (re-identify).
    Reconnect,
    /// Resume with existing session_id + sequence.
    Resume,
    /// Permanent shutdown (fatal close code or explicit shutdown signal).
    Shutdown,
}

/// Outer reconnection loop. Runs until fatal error or shutdown signal.
async fn gateway_loop(adapter: Arc<DiscordAdapter>) -> Result<()> {
    let mut backoff_secs = 1u64;

    loop {
        // Determine connection URL: prefer resume_gateway_url if we have a session
        let url = if let Some(resume_url) = adapter.gateway_connection.resume_gateway_url() {
            format!(
                "{}/?v={}&encoding={}",
                resume_url, GATEWAY_VERSION, GATEWAY_ENCODING
            )
        } else {
            match adapter.get_gateway_url().await {
                Ok(base) => format!(
                    "{}/?v={}&encoding={}",
                    base, GATEWAY_VERSION, GATEWAY_ENCODING
                ),
                Err(e) => {
                    warn!(error = %e, backoff = backoff_secs, "Failed to fetch gateway URL, retrying");
                    tokio::time::sleep(Duration::from_secs(backoff_secs)).await;
                    backoff_secs = (backoff_secs * 2).min(60);
                    continue;
                }
            }
        };

        match run_gateway_session(&adapter, &url).await {
            Ok(GatewayAction::Reconnect) => {
                info!("Discord gateway reconnecting (fresh identify)");
                backoff_secs = 1;
            }
            Ok(GatewayAction::Resume) => {
                info!("Discord gateway reconnecting (resume)");
                backoff_secs = 1;
            }
            Ok(GatewayAction::Shutdown) => {
                info!("Discord gateway shutting down");
                return Ok(());
            }
            Err(e) => {
                error!(error = %e, backoff = backoff_secs, "Discord gateway error, retrying");
                tokio::select! {
                    _ = tokio::time::sleep(Duration::from_secs(backoff_secs)) => {}
                    _ = adapter.shutdown.notified() => return Ok(()),
                }
                backoff_secs = (backoff_secs * 2).min(60);
            }
        }
    }
}

/// A single gateway session: connect, handshake, read events until disconnect.
async fn run_gateway_session(adapter: &Arc<DiscordAdapter>, url: &str) -> Result<GatewayAction> {
    debug!(url, "Connecting to Discord gateway");
    let (ws_stream, _) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| IroncladError::Network(format!("WebSocket connect failed: {e}")))?;

    let (write, mut read) = ws_stream.split();
    let write = Arc::new(tokio::sync::Mutex::new(write));
    let conn = &adapter.gateway_connection;

    // 1. Read Hello (op 10)
    let hello_text = read_next_text(&mut read).await?;
    let hello: Value = serde_json::from_str(&hello_text)
        .map_err(|e| IroncladError::Network(format!("invalid Hello payload: {e}")))?;

    let interval = DiscordAdapter::extract_heartbeat_interval(&hello)
        .ok_or_else(|| IroncladError::Network("missing heartbeat_interval in Hello".into()))?;
    conn.set_heartbeat_interval_ms(interval);
    conn.set_heartbeat_acked(true);
    debug!(interval_ms = interval, "Received Hello");

    // 2. Spawn heartbeat task
    let hb_conn = Arc::clone(&adapter.gateway_connection);
    let hb_write = Arc::clone(&write);
    let hb_shutdown = Arc::clone(&adapter.shutdown);
    let heartbeat_handle = tokio::spawn(async move {
        heartbeat_task(hb_conn, hb_write, hb_shutdown).await;
    });

    // 3. Send Identify or Resume
    let payload = if let (Some(session_id), Some(seq)) = (conn.session_id(), conn.sequence()) {
        info!(session_id, seq, "Resuming gateway session");
        adapter.build_resume(&session_id, seq)
    } else {
        info!("Identifying with gateway");
        adapter.build_identify()
    };
    send_json(&write, &payload).await?;

    // 4. Event read loop
    let action = loop {
        tokio::select! {
            msg = read.next() => {
                match msg {
                    Some(Ok(WsMessage::Text(text))) => {
                        match handle_gateway_message(adapter, &text, &write).await {
                            Ok(Some(action)) => break action,
                            Ok(None) => continue,
                            Err(e) => {
                                warn!(error = %e, "Error handling gateway message");
                                continue;
                            }
                        }
                    }
                    Some(Ok(WsMessage::Close(frame))) => {
                        let code = frame
                            .as_ref()
                            .map(|f| u16::from(f.code))
                            .unwrap_or(1000);
                        info!(code, "Discord gateway WebSocket closed");
                        if DiscordAdapter::is_fatal_close(code) {
                            error!(code, "Fatal Discord close code");
                            break GatewayAction::Shutdown;
                        } else if DiscordAdapter::is_resumable_close(code) {
                            break GatewayAction::Resume;
                        } else {
                            break GatewayAction::Reconnect;
                        }
                    }
                    Some(Ok(_)) => continue,
                    Some(Err(e)) => {
                        warn!(error = %e, "WebSocket read error");
                        break GatewayAction::Reconnect;
                    }
                    None => {
                        info!("WebSocket stream ended");
                        break GatewayAction::Reconnect;
                    }
                }
            }
            _ = adapter.shutdown.notified() => {
                break GatewayAction::Shutdown;
            }
        }
    };

    heartbeat_handle.abort();
    Ok(action)
}

/// Process a single gateway JSON message. Returns `Some(action)` if the
/// read loop should break, `None` to continue reading.
async fn handle_gateway_message(
    adapter: &Arc<DiscordAdapter>,
    text: &str,
    write: &Arc<tokio::sync::Mutex<WsSink>>,
) -> Result<Option<GatewayAction>> {
    let payload: Value = serde_json::from_str(text)
        .map_err(|e| IroncladError::Network(format!("invalid gateway JSON: {e}")))?;

    let op = DiscordAdapter::gateway_opcode(&payload).unwrap_or(u64::MAX);
    let conn = &adapter.gateway_connection;

    match op {
        // Dispatch (op 0) — the main event carrier
        0 => {
            // Extract and track sequence number before dispatching
            if let Some(seq) = payload.get("s").and_then(|v| v.as_u64()) {
                conn.set_sequence(Some(seq));
            }

            if let Some((event_name, data)) = adapter.parse_dispatch(&payload) {
                match event_name.as_str() {
                    "READY" => {
                        if let Some(sid) = data.get("session_id").and_then(|v| v.as_str()) {
                            conn.set_session_id(sid.to_string());
                        }
                        if let Some(url) = data.get("resume_gateway_url").and_then(|v| v.as_str()) {
                            conn.set_resume_gateway_url(url.to_string());
                        }
                        info!("Discord gateway READY");
                    }
                    "RESUMED" => {
                        info!("Discord gateway RESUMED successfully");
                    }
                    "MESSAGE_CREATE" => match adapter.parse_message_create(&data) {
                        Ok(Some(msg)) => {
                            debug!(id = %msg.id, sender = %msg.sender_id, "Gateway received message");
                            adapter.push_message(msg);
                        }
                        Ok(None) => {} // filtered (bot, empty, wrong guild)
                        Err(e) => warn!(error = %e, "Failed to parse MESSAGE_CREATE"),
                    },
                    _ => {
                        debug!(event = %event_name, "Unhandled gateway dispatch event");
                    }
                }
            }
            Ok(None)
        }
        // Heartbeat request (op 1) — server wants an immediate heartbeat
        1 => {
            let hb = adapter.build_heartbeat(conn.sequence());
            send_json(write, &hb).await?;
            Ok(None)
        }
        // Reconnect (op 7) — server requests we reconnect and resume
        7 => {
            info!("Discord gateway requested reconnect (op 7)");
            Ok(Some(GatewayAction::Resume))
        }
        // Invalid Session (op 9) — d=true means resumable, d=false means re-identify
        9 => {
            let resumable = payload.get("d").and_then(|v| v.as_bool()).unwrap_or(false);
            if !resumable {
                info!("Invalid session (not resumable), clearing state");
                conn.clear_session();
                conn.set_sequence(None);
            }
            // Discord docs: wait 1–5 seconds before reconnecting
            let wait_ms = rand::thread_rng().gen_range(1000..5000);
            tokio::time::sleep(Duration::from_millis(wait_ms)).await;
            Ok(Some(if resumable {
                GatewayAction::Resume
            } else {
                GatewayAction::Reconnect
            }))
        }
        // Heartbeat ACK (op 11)
        11 => {
            conn.set_heartbeat_acked(true);
            Ok(None)
        }
        _ => {
            debug!(op, "Unknown gateway opcode");
            Ok(None)
        }
    }
}

/// Background task that sends heartbeats at the gateway's requested interval.
/// Detects zombie connections by tracking ACK receipt.
async fn heartbeat_task(
    conn: Arc<GatewayConnection>,
    write: Arc<tokio::sync::Mutex<WsSink>>,
    shutdown: Arc<Notify>,
) {
    let interval_ms = conn.heartbeat_interval_ms();

    // Discord spec: first heartbeat should have random jitter (0..interval)
    let jitter_ms = rand::thread_rng().gen_range(0..interval_ms);
    tokio::select! {
        _ = tokio::time::sleep(Duration::from_millis(jitter_ms)) => {}
        _ = shutdown.notified() => return,
    }

    let interval = Duration::from_millis(interval_ms);
    loop {
        // Check if previous heartbeat was acknowledged
        if !conn.heartbeat_acked() {
            warn!("Discord heartbeat not ACK'd — zombie connection, closing");
            // Force-close the write side to trigger read-loop termination.
            // Use a timeout to avoid blocking forever on a dead TCP connection.
            let mut w = write.lock().await;
            let _ = tokio::time::timeout(Duration::from_secs(5), w.close()).await;
            return;
        }

        conn.set_heartbeat_acked(false);
        let hb = json!({ "op": 1, "d": conn.sequence() });
        {
            let mut w = write.lock().await;
            if let Err(e) = w.send(WsMessage::Text(hb.to_string())).await {
                warn!(error = %e, "Failed to send heartbeat");
                return;
            }
        }

        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = shutdown.notified() => return,
        }
    }
}

/// Read the next text frame from the WebSocket, skipping non-text frames.
async fn read_next_text(read: &mut SplitStream<WsStream>) -> Result<String> {
    while let Some(msg) = read.next().await {
        match msg {
            Ok(WsMessage::Text(text)) => return Ok(text.to_string()),
            Ok(WsMessage::Close(_)) => {
                return Err(IroncladError::Network(
                    "WebSocket closed during handshake".into(),
                ));
            }
            Ok(_) => continue,
            Err(e) => {
                return Err(IroncladError::Network(format!("WebSocket read error: {e}")));
            }
        }
    }
    Err(IroncladError::Network(
        "WebSocket stream ended during handshake".into(),
    ))
}

/// Send a JSON payload over the gateway WebSocket.
///
/// SECURITY: `payload` may contain the bot token (e.g. in Identify).
/// Never log `text` or `payload` — use opcode-level tracing only.
async fn send_json(write: &Arc<tokio::sync::Mutex<WsSink>>, payload: &Value) -> Result<()> {
    let text = payload.to_string();
    let mut w = write.lock().await;
    w.send(WsMessage::Text(text))
        .await
        .map_err(|e| IroncladError::Network(format!("WebSocket send failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_adapter_defaults() {
        let adapter = DiscordAdapter::new("test-token".into());
        assert_eq!(adapter.token, "test-token");
        assert!(adapter.allowed_guild_ids.is_empty());
        assert!(adapter.deny_on_empty);
    }

    #[test]
    fn with_config_sets_guilds() {
        let adapter = DiscordAdapter::with_config(
            "tok".into(),
            vec!["guild1".into(), "guild2".into()],
            false,
        );
        assert_eq!(adapter.allowed_guild_ids.len(), 2);
        assert!(!adapter.deny_on_empty);
    }

    #[test]
    fn guild_allowed_empty_default_denies_all() {
        // secure default: empty list denies everyone
        let adapter = DiscordAdapter::new("tok".into());
        assert!(!adapter.is_guild_allowed("any_guild"));
    }

    #[test]
    fn guild_allowed_empty_secure_denies_all() {
        // deny_on_empty=true (secure default): empty list denies everyone
        let adapter = DiscordAdapter::with_config("tok".into(), vec![], true);
        assert!(!adapter.is_guild_allowed("any_guild"));
    }

    #[test]
    fn guild_allowed_filters() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["g1".into()], false);
        assert!(adapter.is_guild_allowed("g1"));
        assert!(!adapter.is_guild_allowed("g2"));
    }

    #[test]
    fn guild_allowed_filters_with_deny_on_empty() {
        // deny_on_empty doesn't affect non-empty lists
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["g1".into()], true);
        assert!(adapter.is_guild_allowed("g1"));
        assert!(!adapter.is_guild_allowed("g2"));
    }

    #[test]
    fn parse_message_create_valid() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["g789".into()], false);
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
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["allowed".into()], false);
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
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["restricted".into()], false);
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

    #[test]
    fn gateway_connection_default() {
        let conn = GatewayConnection::default();
        assert!(conn.sequence().is_none());
        assert!(conn.session_id().is_none());
    }

    #[test]
    fn gateway_connection_set_sequence_none() {
        let conn = GatewayConnection::new();
        conn.set_sequence(Some(100));
        assert_eq!(conn.sequence(), Some(100));
        conn.set_sequence(None);
        assert!(conn.sequence().is_none());
    }

    #[test]
    fn parse_dispatch_missing_t() {
        let adapter = DiscordAdapter::new("tok".into());
        let payload = json!({"op": 0, "d": {"content": "hi"}});
        assert!(adapter.parse_dispatch(&payload).is_none());
    }

    #[test]
    fn parse_dispatch_missing_d() {
        let adapter = DiscordAdapter::new("tok".into());
        let payload = json!({"op": 0, "t": "MESSAGE_CREATE"});
        assert!(adapter.parse_dispatch(&payload).is_none());
    }

    #[test]
    fn gateway_opcode_missing() {
        let payload = json!({"d": "no op"});
        assert!(DiscordAdapter::gateway_opcode(&payload).is_none());
    }

    #[test]
    fn extract_heartbeat_interval_missing() {
        let payload = json!({"op": 10, "d": {}});
        assert!(DiscordAdapter::extract_heartbeat_interval(&payload).is_none());
    }

    #[test]
    fn extract_heartbeat_interval_missing_d() {
        let payload = json!({"op": 10});
        assert!(DiscordAdapter::extract_heartbeat_interval(&payload).is_none());
    }

    #[test]
    fn is_resumable_close_all_codes() {
        let resumable = [4000, 4001, 4002, 4003, 4005, 4007, 4008, 4009];
        for code in resumable {
            assert!(
                DiscordAdapter::is_resumable_close(code),
                "code {} should be resumable",
                code
            );
        }
        // Non-resumable
        assert!(!DiscordAdapter::is_resumable_close(4004));
        assert!(!DiscordAdapter::is_resumable_close(4006));
        assert!(!DiscordAdapter::is_resumable_close(4010));
        assert!(!DiscordAdapter::is_resumable_close(1000));
    }

    #[test]
    fn is_fatal_close_all_codes() {
        let fatal = [4004, 4010, 4011, 4012, 4013, 4014];
        for code in fatal {
            assert!(
                DiscordAdapter::is_fatal_close(code),
                "code {} should be fatal",
                code
            );
        }
        // Non-fatal
        assert!(!DiscordAdapter::is_fatal_close(4000));
        assert!(!DiscordAdapter::is_fatal_close(4001));
        assert!(!DiscordAdapter::is_fatal_close(1000));
    }

    #[test]
    fn build_identify_intents_include_guild_messages() {
        let adapter = DiscordAdapter::new("tok".into());
        let identify = adapter.build_identify();
        let intents = identify["d"]["intents"].as_u64().unwrap();
        // Check all three intents: GUILD_MESSAGES (512), GUILDS (1), MESSAGE_CONTENT (4096)
        assert!(intents & 512 != 0);
        assert!(intents & 1 != 0);
        assert!(intents & 4096 != 0);
    }

    #[test]
    fn build_identify_properties() {
        let adapter = DiscordAdapter::new("tok".into());
        let identify = adapter.build_identify();
        let props = &identify["d"]["properties"];
        assert_eq!(props["os"], "linux");
        assert_eq!(props["browser"], "ironclad");
        assert_eq!(props["device"], "ironclad");
    }

    #[test]
    fn build_resume_includes_token() {
        let adapter = DiscordAdapter::new("secret-token".into());
        let resume = adapter.build_resume("sess-1", 50);
        assert_eq!(resume["d"]["token"], "secret-token");
        assert_eq!(resume["d"]["session_id"], "sess-1");
        assert_eq!(resume["d"]["seq"], 50);
    }

    #[test]
    fn parse_message_create_bot_false_passes() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "u1", "bot": false},
            "content": "not a bot"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn parse_message_create_no_bot_field() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "u1"},
            "content": "no bot field"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn parse_message_create_missing_content_field() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {"id": "u1"}
        });
        let result = adapter.parse_message_create(&data).unwrap();
        // content defaults to empty string -> returns None
        assert!(result.is_none());
    }

    #[test]
    fn parse_message_create_allowed_guild() {
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["allowed_g".into()], false);
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "guild_id": "allowed_g",
            "author": {"id": "u1"},
            "content": "in allowed guild"
        });
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn chunk_message_unicode_boundary() {
        // Multi-byte characters should not be split
        let text = "ab".repeat(30) + &"ñ".repeat(50);
        let chunks = DiscordAdapter::chunk_message(&text, 50);
        for chunk in &chunks {
            assert!(chunk.len() <= 50);
            // Verify all chunks are valid UTF-8 (implied by being a String)
            assert!(chunk.is_ascii() || !chunk.is_empty());
        }
    }

    #[test]
    fn push_multiple_messages_fifo() {
        let adapter = DiscordAdapter::new("tok".into());
        for i in 0..3 {
            adapter.push_message(InboundMessage {
                id: format!("d{}", i),
                platform: "discord".into(),
                sender_id: "u".into(),
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
    fn parse_dispatch_without_sequence() {
        let adapter = DiscordAdapter::new("tok".into());
        let payload = json!({
            "op": 0,
            "t": "READY",
            "d": {"v": 10}
        });
        let (name, data) = adapter.parse_dispatch(&payload).unwrap();
        assert_eq!(name, "READY");
        assert_eq!(data["v"], 10);
    }

    #[test]
    fn parse_message_create_unknown_author_id() {
        let adapter = DiscordAdapter::new("tok".into());
        let data = json!({
            "id": "m1",
            "channel_id": "c1",
            "author": {},
            "content": "no author id"
        });
        let result = adapter.parse_message_create(&data).unwrap().unwrap();
        assert_eq!(result.sender_id, "unknown");
    }

    #[test]
    fn parse_message_create_extracts_attachments() {
        let adapter = DiscordAdapter::new("test-token".into());
        let data = json!({
            "id": "msg-attach-1",
            "channel_id": "ch-99",
            "author": { "id": "user-1" },
            "content": "Check this out!",
            "attachments": [
                {
                    "id": "att-1",
                    "filename": "photo.png",
                    "content_type": "image/png",
                    "size": 123456,
                    "url": "https://cdn.discordapp.com/attachments/123/456/photo.png"
                },
                {
                    "id": "att-2",
                    "filename": "report.pdf",
                    "content_type": "application/pdf",
                    "size": 54321,
                    "url": "https://cdn.discordapp.com/attachments/123/456/report.pdf"
                }
            ]
        });
        let msg = adapter.parse_message_create(&data).unwrap().unwrap();
        assert_eq!(msg.content, "Check this out!");

        let meta = msg.metadata.unwrap();
        let attachments = meta["attachments"].as_array().expect("attachments array");
        assert_eq!(attachments.len(), 2);

        // First attachment: image
        assert_eq!(attachments[0]["media_type"], "image");
        assert_eq!(attachments[0]["filename"], "photo.png");
        assert_eq!(attachments[0]["content_type"], "image/png");
        assert_eq!(attachments[0]["size_bytes"], 123456);
        assert!(
            attachments[0]["source_url"]
                .as_str()
                .unwrap()
                .starts_with("https://")
        );

        // Second attachment: document
        assert_eq!(attachments[1]["media_type"], "document");
        assert_eq!(attachments[1]["filename"], "report.pdf");
    }

    #[test]
    fn parse_message_create_no_attachments_no_key() {
        let adapter = DiscordAdapter::new("test-token".into());
        let data = json!({
            "id": "msg-no-att",
            "channel_id": "ch-1",
            "author": { "id": "user-1" },
            "content": "plain text"
        });
        let msg = adapter.parse_message_create(&data).unwrap().unwrap();
        let meta = msg.metadata.unwrap();
        // When no attachments, the key should not be present
        assert!(meta.get("attachments").is_none());
    }

    #[test]
    fn parse_message_create_empty_attachments_array() {
        let adapter = DiscordAdapter::new("test-token".into());
        let data = json!({
            "id": "msg-empty-att",
            "channel_id": "ch-1",
            "author": { "id": "user-1" },
            "content": "text with empty attachments",
            "attachments": []
        });
        let msg = adapter.parse_message_create(&data).unwrap().unwrap();
        let meta = msg.metadata.unwrap();
        // Empty attachments array should not produce an attachments key
        assert!(meta.get("attachments").is_none());
    }

    // ── async method tests (exercise error paths via connection refusal) ──

    fn fast_fail_adapter() -> DiscordAdapter {
        // Route discord.com to a non-routable TEST-NET address (RFC 5737) so
        // requests fail deterministically regardless of CI network speed.
        let mut adapter = DiscordAdapter::new("test-bot-token".into());
        adapter.client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(50))
            .resolve(
                "discord.com",
                std::net::SocketAddr::from(([192, 0, 2, 1], 443)),
            )
            .build()
            .unwrap();
        adapter
    }

    #[tokio::test]
    async fn send_message_network_error() {
        let adapter = fast_fail_adapter();
        let result = adapter.send_message("channel123", "test content").await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("Discord send failed"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn send_typing_best_effort_no_panic() {
        let adapter = fast_fail_adapter();
        adapter.send_typing("channel123").await;
    }

    #[tokio::test]
    async fn send_ephemeral_returns_none_on_failure() {
        let adapter = fast_fail_adapter();
        let result = adapter.send_ephemeral("channel123", "test").await;
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn get_gateway_url_network_error() {
        let adapter = fast_fail_adapter();
        let result = adapter.get_gateway_url().await;
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("get gateway failed"),
            "unexpected error: {err}"
        );
    }

    #[tokio::test]
    async fn send_trait_impl_network_error() {
        let adapter = fast_fail_adapter();
        let msg = OutboundMessage {
            content: "hello".into(),
            recipient_id: "channel123".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn send_trait_impl_long_message_network_error() {
        let adapter = fast_fail_adapter();
        let long_content = "word ".repeat(500);
        let msg = OutboundMessage {
            content: long_content,
            recipient_id: "channel123".into(),
            metadata: None,
        };
        let result = adapter.send(msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn recv_returns_none_when_buffer_empty() {
        let adapter = DiscordAdapter::new("test-token".into());
        let result = adapter.recv().await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn recv_returns_buffered_message() {
        let adapter = DiscordAdapter::new("test-token".into());
        {
            let mut buf = adapter.message_buffer.lock().unwrap();
            buf.push_back(InboundMessage {
                id: "d1".into(),
                platform: "discord".into(),
                sender_id: "u1".into(),
                content: "buffered msg".into(),
                timestamp: Utc::now(),
                metadata: None,
            });
        }
        let result = adapter.recv().await.unwrap();
        assert!(result.is_some());
        assert_eq!(result.unwrap().content, "buffered msg");
    }

    // ── Gateway connection state tests ──────────────────────────────

    #[test]
    fn gateway_connection_heartbeat_acked_tracking() {
        let conn = GatewayConnection::new();
        // Starts acked (true)
        assert!(conn.heartbeat_acked());

        conn.set_heartbeat_acked(false);
        assert!(!conn.heartbeat_acked());

        conn.set_heartbeat_acked(true);
        assert!(conn.heartbeat_acked());
    }

    #[test]
    fn gateway_connection_resume_url_storage() {
        let conn = GatewayConnection::new();
        assert!(conn.resume_gateway_url().is_none());

        conn.set_resume_gateway_url("wss://gateway-resume.discord.gg".to_string());
        assert_eq!(
            conn.resume_gateway_url().as_deref(),
            Some("wss://gateway-resume.discord.gg")
        );
    }

    #[test]
    fn gateway_connection_heartbeat_interval_get_set() {
        let conn = GatewayConnection::new();
        // Default is 41250
        assert_eq!(conn.heartbeat_interval_ms(), 41250);

        conn.set_heartbeat_interval_ms(45000);
        assert_eq!(conn.heartbeat_interval_ms(), 45000);
    }

    #[test]
    fn gateway_connection_clear_session_resets_all() {
        let conn = GatewayConnection::new();
        conn.set_session_id("sess-abc".to_string());
        assert!(conn.session_id().is_some());

        conn.clear_session();
        assert!(conn.session_id().is_none());
    }

    // ── Gateway dispatch pipeline tests ─────────────────────────────

    #[test]
    fn dispatch_message_create_full_pipeline() {
        // Simulates the full gateway dispatch path:
        // raw JSON → extract sequence → parse_dispatch → parse_message_create → push → recv
        let adapter = DiscordAdapter::with_config("tok".into(), vec!["g-456".into()], false);
        let conn = &adapter.gateway_connection;

        let raw = json!({
            "op": 0,
            "s": 42,
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "msg-gw-001",
                "channel_id": "ch-123",
                "guild_id": "g-456",
                "author": {"id": "user-789", "username": "testuser"},
                "content": "hello from gateway"
            }
        });

        // Step 1: Extract sequence
        if let Some(seq) = raw.get("s").and_then(|v| v.as_u64()) {
            conn.set_sequence(Some(seq));
        }
        assert_eq!(conn.sequence(), Some(42));

        // Step 2: Parse dispatch
        let (event_name, data) = adapter.parse_dispatch(&raw).unwrap();
        assert_eq!(event_name, "MESSAGE_CREATE");

        // Step 3: Parse message and push
        let msg = adapter.parse_message_create(&data).unwrap().unwrap();
        assert_eq!(msg.id, "msg-gw-001");
        assert_eq!(msg.sender_id, "user-789");
        assert_eq!(msg.content, "hello from gateway");
        adapter.push_message(msg);

        // Step 4: Verify buffer
        let rt = tokio::runtime::Runtime::new().unwrap();
        let received = rt.block_on(adapter.recv()).unwrap().unwrap();
        assert_eq!(received.content, "hello from gateway");
    }

    #[test]
    fn dispatch_ready_extracts_session_and_resume_url() {
        let adapter = DiscordAdapter::new("tok".into());
        let conn = &adapter.gateway_connection;

        let raw = json!({
            "op": 0,
            "s": 1,
            "t": "READY",
            "d": {
                "v": 10,
                "session_id": "sess-ready-001",
                "resume_gateway_url": "wss://gateway-us-east1-b.discord.gg",
                "user": {"id": "bot-id", "username": "ironclad"}
            }
        });

        // Extract sequence
        if let Some(seq) = raw.get("s").and_then(|v| v.as_u64()) {
            conn.set_sequence(Some(seq));
        }

        let (event_name, data) = adapter.parse_dispatch(&raw).unwrap();
        assert_eq!(event_name, "READY");

        // Extract session_id and resume_gateway_url (mirrors handle_gateway_message logic)
        if let Some(sid) = data.get("session_id").and_then(|v| v.as_str()) {
            conn.set_session_id(sid.to_string());
        }
        if let Some(url) = data.get("resume_gateway_url").and_then(|v| v.as_str()) {
            conn.set_resume_gateway_url(url.to_string());
        }

        assert_eq!(conn.session_id().as_deref(), Some("sess-ready-001"));
        assert_eq!(
            conn.resume_gateway_url().as_deref(),
            Some("wss://gateway-us-east1-b.discord.gg")
        );
    }

    #[test]
    fn dispatch_sequence_tracking_increments() {
        let conn = GatewayConnection::new();
        assert!(conn.sequence().is_none());

        // Simulate receiving sequential dispatches
        for seq in [1, 2, 3, 10, 42] {
            conn.set_sequence(Some(seq));
            assert_eq!(conn.sequence(), Some(seq));
        }
    }

    #[test]
    fn dispatch_bot_message_filtered_in_pipeline() {
        let adapter = DiscordAdapter::new("tok".into());

        let raw = json!({
            "op": 0,
            "s": 5,
            "t": "MESSAGE_CREATE",
            "d": {
                "id": "bot-msg-001",
                "channel_id": "ch-1",
                "author": {"id": "bot-id", "bot": true},
                "content": "I am a bot"
            }
        });

        let (event_name, data) = adapter.parse_dispatch(&raw).unwrap();
        assert_eq!(event_name, "MESSAGE_CREATE");

        // Bot messages are filtered by parse_message_create
        let result = adapter.parse_message_create(&data).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn identify_vs_resume_decision() {
        let adapter = DiscordAdapter::new("tok".into());
        let conn = &adapter.gateway_connection;

        // No session → should identify (op 2)
        let payload = adapter.build_identify();
        assert_eq!(payload["op"], 2);

        // With session → should resume (op 6)
        conn.set_session_id("sess-001".to_string());
        conn.set_sequence(Some(99));
        let payload = adapter.build_resume(&conn.session_id().unwrap(), conn.sequence().unwrap());
        assert_eq!(payload["op"], 6);
        assert_eq!(payload["d"]["session_id"], "sess-001");
        assert_eq!(payload["d"]["seq"], 99);
    }

    #[test]
    fn invalid_session_non_resumable_clears_state() {
        let conn = GatewayConnection::new();
        conn.set_session_id("old-session".to_string());
        conn.set_sequence(Some(500));

        // Simulate op 9 with d=false (non-resumable)
        let payload = json!({"op": 9, "d": false});
        let resumable = payload.get("d").and_then(|v| v.as_bool()).unwrap_or(false);
        assert!(!resumable);

        // Non-resumable → clear state
        conn.clear_session();
        conn.set_sequence(None);
        assert!(conn.session_id().is_none());
        assert!(conn.sequence().is_none());
    }

    #[test]
    fn invalid_session_resumable_preserves_state() {
        let conn = GatewayConnection::new();
        conn.set_session_id("keep-session".to_string());
        conn.set_sequence(Some(200));

        // Simulate op 9 with d=true (resumable)
        let payload = json!({"op": 9, "d": true});
        let resumable = payload.get("d").and_then(|v| v.as_bool()).unwrap_or(false);
        assert!(resumable);

        // Resumable → don't clear
        assert_eq!(conn.session_id().as_deref(), Some("keep-session"));
        assert_eq!(conn.sequence(), Some(200));
    }

    #[test]
    fn close_code_classification_exhaustive() {
        // Every code 1000-4020 should be classified as exactly one of:
        // resumable, fatal, or neither (normal reconnect)
        for code in 1000u16..=4020 {
            let is_r = DiscordAdapter::is_resumable_close(code);
            let is_f = DiscordAdapter::is_fatal_close(code);
            assert!(
                !(is_r && is_f),
                "code {code} classified as both resumable and fatal"
            );
        }
    }
}
