use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::debug;

use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CdpTarget {
    pub id: String,
    pub title: String,
    pub url: String,
    #[serde(rename = "type")]
    pub target_type: String,
    #[serde(rename = "webSocketDebuggerUrl")]
    pub ws_url: Option<String>,
}

/// Low-level HTTP client for the Chrome DevTools Protocol JSON endpoints.
///
/// # Security
///
/// The CDP port (`http://127.0.0.1:<port>`) is accessible to **all** local
/// processes. Any program running on the same host can list targets, attach
/// debuggers, and execute arbitrary JavaScript in browser contexts. In
/// production deployments, callers should consider firewall rules or network
/// namespaces to restrict access to the CDP port.
pub struct CdpClient {
    http_base: String,
    client: reqwest::Client,
    command_id: AtomicU64,
}

impl CdpClient {
    pub fn new(port: u16) -> Self {
        Self {
            http_base: format!("http://127.0.0.1:{port}"),
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(10))
                .build()
                .expect("HTTP client initialization - check TLS certificates"),
            command_id: AtomicU64::new(1),
        }
    }

    pub fn next_id(&self) -> u64 {
        self.command_id.fetch_add(1, Ordering::SeqCst)
    }

    pub fn build_command(&self, method: &str, params: Value) -> Value {
        json!({
            "id": self.next_id(),
            "method": method,
            "params": params,
        })
    }

    pub async fn list_targets(&self) -> Result<Vec<CdpTarget>> {
        let url = format!("{}/json/list", self.http_base);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP list targets failed: {e}")))?;

        let targets: Vec<CdpTarget> = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP parse targets failed: {e}")))?;

        debug!(count = targets.len(), "listed CDP targets");
        Ok(targets)
    }

    pub async fn new_tab(&self, url: &str) -> Result<CdpTarget> {
        let api_url = format!("{}/json/new?{}", self.http_base, url);
        let resp = self
            .client
            .get(&api_url)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP new tab failed: {e}")))?;

        let target: CdpTarget = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP parse new tab failed: {e}")))?;

        debug!(id = %target.id, url = %target.url, "opened new tab");
        Ok(target)
    }

    pub async fn close_tab(&self, target_id: &str) -> Result<()> {
        let url = format!("{}/json/close/{}", self.http_base, target_id);
        self.client
            .get(&url)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP close tab failed: {e}")))?;
        debug!(id = target_id, "closed tab");
        Ok(())
    }

    pub async fn version(&self) -> Result<Value> {
        let url = format!("{}/json/version", self.http_base);
        let resp = self
            .client
            .get(&url)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP version failed: {e}")))?;

        resp.json()
            .await
            .map_err(|e| IroncladError::Network(format!("CDP version parse failed: {e}")))
    }

    pub fn navigate_command(&self, url: &str) -> Value {
        self.build_command("Page.navigate", json!({ "url": url }))
    }

    pub fn evaluate_command(&self, expression: &str) -> Value {
        self.build_command(
            "Runtime.evaluate",
            json!({
                "expression": expression,
                "returnByValue": true,
            }),
        )
    }

    pub fn screenshot_command(&self) -> Value {
        self.build_command(
            "Page.captureScreenshot",
            json!({
                "format": "png",
                "quality": 80,
            }),
        )
    }

    pub fn get_document_command(&self) -> Value {
        self.build_command("DOM.getDocument", json!({}))
    }

    pub fn click_command(&self, x: f64, y: f64) -> Value {
        self.build_command(
            "Input.dispatchMouseEvent",
            json!({
                "type": "mousePressed",
                "x": x,
                "y": y,
                "button": "left",
                "clickCount": 1,
            }),
        )
    }

    pub fn type_text_command(&self, text: &str) -> Value {
        self.build_command(
            "Input.insertText",
            json!({
                "text": text,
            }),
        )
    }

    pub fn pdf_command(&self) -> Value {
        self.build_command(
            "Page.printToPDF",
            json!({
                "printBackground": true,
            }),
        )
    }

    pub fn get_cookies_command(&self) -> Value {
        self.build_command("Network.getCookies", json!({}))
    }

    pub fn clear_cookies_command(&self) -> Value {
        self.build_command("Network.clearBrowserCookies", json!({}))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cdp_client_new() {
        let client = CdpClient::new(9222);
        assert_eq!(client.http_base, "http://127.0.0.1:9222");
    }

    #[test]
    fn command_ids_increment() {
        let client = CdpClient::new(9222);
        let id1 = client.next_id();
        let id2 = client.next_id();
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn build_command_structure() {
        let client = CdpClient::new(9222);
        let cmd = client.build_command("Page.navigate", json!({"url": "https://example.com"}));
        assert!(cmd.get("id").is_some());
        assert_eq!(cmd["method"], "Page.navigate");
        assert_eq!(cmd["params"]["url"], "https://example.com");
    }

    #[test]
    fn navigate_command() {
        let client = CdpClient::new(9222);
        let cmd = client.navigate_command("https://test.com");
        assert_eq!(cmd["method"], "Page.navigate");
        assert_eq!(cmd["params"]["url"], "https://test.com");
    }

    #[test]
    fn evaluate_command() {
        let client = CdpClient::new(9222);
        let cmd = client.evaluate_command("document.title");
        assert_eq!(cmd["method"], "Runtime.evaluate");
        assert_eq!(cmd["params"]["expression"], "document.title");
    }

    #[test]
    fn screenshot_command() {
        let client = CdpClient::new(9222);
        let cmd = client.screenshot_command();
        assert_eq!(cmd["method"], "Page.captureScreenshot");
    }

    #[test]
    fn click_command() {
        let client = CdpClient::new(9222);
        let cmd = client.click_command(100.0, 200.0);
        assert_eq!(cmd["method"], "Input.dispatchMouseEvent");
        assert_eq!(cmd["params"]["x"], 100.0);
        assert_eq!(cmd["params"]["y"], 200.0);
    }

    #[test]
    fn type_text_command() {
        let client = CdpClient::new(9222);
        let cmd = client.type_text_command("hello");
        assert_eq!(cmd["method"], "Input.insertText");
        assert_eq!(cmd["params"]["text"], "hello");
    }

    #[test]
    fn pdf_command() {
        let client = CdpClient::new(9222);
        let cmd = client.pdf_command();
        assert_eq!(cmd["method"], "Page.printToPDF");
    }

    #[test]
    fn cookie_commands() {
        let client = CdpClient::new(9222);
        let get = client.get_cookies_command();
        assert_eq!(get["method"], "Network.getCookies");
        let clear = client.clear_cookies_command();
        assert_eq!(clear["method"], "Network.clearBrowserCookies");
    }
}
