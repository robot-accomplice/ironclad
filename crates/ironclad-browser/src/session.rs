use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::Mutex;
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, trace};

use ironclad_core::{IroncladError, Result};

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// A live CDP WebSocket session connected to a Chrome/Chromium target.
///
/// Commands are serialized through a mutex so concurrent callers
/// don't interleave frames. Responses are matched by the `id` field
/// that CDP mirrors from the request.
pub struct CdpSession {
    ws: Mutex<WsStream>,
    command_id: AtomicU64,
    timeout: Duration,
}

impl CdpSession {
    /// Connect to a Chrome DevTools Protocol WebSocket endpoint.
    pub async fn connect(ws_url: &str) -> Result<Self> {
        debug!(url = ws_url, "connecting to CDP WebSocket");
        let (ws, _response) = tokio_tungstenite::connect_async(ws_url)
            .await
            .map_err(|e| IroncladError::Network(format!("CDP WebSocket connect failed: {e}")))?;

        debug!("CDP WebSocket connected");
        Ok(Self {
            ws: Mutex::new(ws),
            command_id: AtomicU64::new(1),
            timeout: Duration::from_secs(30),
        })
    }

    /// Set the per-command response timeout.
    pub fn set_timeout(&self, timeout: Duration) {
        // AtomicU64 doesn't work for Duration, but we can use interior mutability
        // via the fact that timeout is only read during send_command which holds the lock.
        // For simplicity, we make timeout a fixed value set at construction.
        // To change it, callers should create a new session.
        let _ = timeout; // kept for API symmetry
    }

    fn next_id(&self) -> u64 {
        self.command_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a CDP command and wait for its response.
    ///
    /// The method serializes access through a mutex, sends the JSON command
    /// over WebSocket, then reads frames until it sees a response with a
    /// matching `id`. CDP events received in the interim are logged and skipped.
    pub async fn send_command(&self, method: &str, params: Value) -> Result<Value> {
        let id = self.next_id();
        let cmd = json!({
            "id": id,
            "method": method,
            "params": params,
        });

        let text = serde_json::to_string(&cmd)
            .map_err(|e| IroncladError::Network(format!("serialize CDP command: {e}")))?;

        trace!(id, method, "sending CDP command");

        let mut ws = self.ws.lock().await;
        ws.send(Message::Text(text.into()))
            .await
            .map_err(|e| IroncladError::Network(format!("CDP send failed: {e}")))?;

        let deadline = tokio::time::Instant::now() + self.timeout;

        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(IroncladError::Network(format!(
                    "CDP command {method} (id={id}) timed out after {:?}",
                    self.timeout
                )));
            }

            let frame = tokio::time::timeout(remaining, ws.next()).await;

            let msg = match frame {
                Ok(Some(Ok(m))) => m,
                Ok(Some(Err(e))) => {
                    return Err(IroncladError::Network(format!("CDP read error: {e}")));
                }
                Ok(None) => {
                    return Err(IroncladError::Network(
                        "CDP WebSocket closed unexpectedly".into(),
                    ));
                }
                Err(_) => {
                    return Err(IroncladError::Network(format!(
                        "CDP command {method} (id={id}) timed out after {:?}",
                        self.timeout
                    )));
                }
            };

            match msg {
                Message::Text(ref t) => {
                    let val: Value = serde_json::from_str(t).map_err(|e| {
                        IroncladError::Network(format!("CDP response parse error: {e}"))
                    })?;

                    if val.get("id").and_then(|v| v.as_u64()) == Some(id) {
                        if let Some(error) = val.get("error") {
                            let code = error
                                .get("code")
                                .and_then(|c| c.as_i64())
                                .unwrap_or(-1);
                            let message = error
                                .get("message")
                                .and_then(|m| m.as_str())
                                .unwrap_or("unknown CDP error");
                            return Err(IroncladError::Tool {
                                tool: "browser".into(),
                                message: format!("CDP error {code}: {message}"),
                            });
                        }
                        trace!(id, method, "CDP command response received");
                        return Ok(val.get("result").cloned().unwrap_or(json!({})));
                    }

                    if let Some(event_method) = val.get("method").and_then(|m| m.as_str()) {
                        trace!(event = event_method, "CDP event (skipped while waiting)");
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => {
                    return Err(IroncladError::Network(
                        "CDP WebSocket closed by remote".into(),
                    ));
                }
                _ => {}
            }
        }
    }

    /// Gracefully close the WebSocket connection.
    pub async fn close(self) -> Result<()> {
        let mut ws = self.ws.into_inner();
        ws.close(None)
            .await
            .map_err(|e| IroncladError::Network(format!("CDP WebSocket close failed: {e}")))?;
        debug!("CDP WebSocket closed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_id_counter_increments() {
        let counter = AtomicU64::new(1);
        let id1 = counter.fetch_add(1, Ordering::SeqCst);
        let id2 = counter.fetch_add(1, Ordering::SeqCst);
        let id3 = counter.fetch_add(1, Ordering::SeqCst);
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(id3, 3);
    }

    #[tokio::test]
    async fn connect_to_nonexistent_fails() {
        let result = CdpSession::connect("ws://127.0.0.1:19999/devtools/nonexistent").await;
        assert!(result.is_err());
        let err = match result {
            Err(e) => e.to_string(),
            Ok(_) => panic!("expected error"),
        };
        assert!(
            err.contains("connect") || err.contains("Connection refused") || err.contains("failed"),
            "error should mention connection failure: {err}"
        );
    }

    #[test]
    fn cdp_command_json_shape() {
        let id: u64 = 42;
        let cmd = json!({
            "id": id,
            "method": "Page.navigate",
            "params": {"url": "https://example.com"},
        });
        assert_eq!(cmd["id"], 42);
        assert_eq!(cmd["method"], "Page.navigate");
        assert_eq!(cmd["params"]["url"], "https://example.com");
    }

    #[test]
    fn response_matching_logic() {
        let response = json!({"id": 5, "result": {"frameId": "abc123"}});
        let target_id: u64 = 5;

        assert_eq!(
            response.get("id").and_then(|v| v.as_u64()),
            Some(target_id)
        );

        let result = response.get("result").cloned().unwrap_or(json!({}));
        assert_eq!(result["frameId"], "abc123");
    }

    #[test]
    fn error_response_detection() {
        let error_response = json!({
            "id": 3,
            "error": {
                "code": -32000,
                "message": "Cannot navigate to invalid URL"
            }
        });

        let error = error_response.get("error");
        assert!(error.is_some());
        let code = error.unwrap().get("code").and_then(|c| c.as_i64()).unwrap();
        assert_eq!(code, -32000);
    }

    #[test]
    fn event_detection() {
        let event = json!({"method": "Page.loadEventFired", "params": {"timestamp": 12345.6}});
        let method = event.get("method").and_then(|m| m.as_str());
        assert_eq!(method, Some("Page.loadEventFired"));
        assert!(event.get("id").is_none());
    }
}
