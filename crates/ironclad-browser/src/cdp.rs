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

    #[test]
    fn get_document_command() {
        let client = CdpClient::new(9222);
        let cmd = client.get_document_command();
        assert_eq!(cmd["method"], "DOM.getDocument");
        assert!(cmd.get("id").is_some());
        assert!(cmd.get("params").is_some());
    }

    #[test]
    fn cdp_target_serde_roundtrip() {
        let target = CdpTarget {
            id: "ABC123".into(),
            title: "Test Page".into(),
            url: "https://example.com".into(),
            target_type: "page".into(),
            ws_url: Some("ws://127.0.0.1:9222/devtools/page/ABC123".into()),
        };
        let json = serde_json::to_string(&target).unwrap();
        let back: CdpTarget = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "ABC123");
        assert_eq!(back.title, "Test Page");
        assert_eq!(back.url, "https://example.com");
        assert_eq!(back.target_type, "page");
        assert!(back.ws_url.is_some());
    }

    #[test]
    fn cdp_target_serde_without_ws_url() {
        let json_str = r#"{
            "id": "DEF456",
            "title": "Background",
            "url": "chrome://newtab",
            "type": "background_page"
        }"#;
        let target: CdpTarget = serde_json::from_str(json_str).unwrap();
        assert_eq!(target.id, "DEF456");
        assert_eq!(target.target_type, "background_page");
        assert!(target.ws_url.is_none());
    }

    #[test]
    fn custom_port_http_base() {
        let client = CdpClient::new(9333);
        assert_eq!(client.http_base, "http://127.0.0.1:9333");
    }

    #[test]
    fn command_ids_are_sequential() {
        let client = CdpClient::new(9222);
        let cmd1 = client.build_command("A", json!({}));
        let cmd2 = client.build_command("B", json!({}));
        let cmd3 = client.build_command("C", json!({}));
        let id1 = cmd1["id"].as_u64().unwrap();
        let id2 = cmd2["id"].as_u64().unwrap();
        let id3 = cmd3["id"].as_u64().unwrap();
        assert_eq!(id2, id1 + 1);
        assert_eq!(id3, id2 + 1);
    }

    #[test]
    fn all_command_builders_have_correct_structure() {
        let client = CdpClient::new(9222);

        // Each builder should produce: id, method, params
        let cmds = vec![
            client.navigate_command("https://example.com"),
            client.evaluate_command("1+1"),
            client.screenshot_command(),
            client.get_document_command(),
            client.click_command(10.0, 20.0),
            client.type_text_command("hello"),
            client.pdf_command(),
            client.get_cookies_command(),
            client.clear_cookies_command(),
        ];

        for cmd in &cmds {
            assert!(cmd.get("id").is_some(), "missing id in command: {cmd}");
            assert!(
                cmd.get("method").is_some(),
                "missing method in command: {cmd}"
            );
            assert!(
                cmd.get("params").is_some(),
                "missing params in command: {cmd}"
            );
        }
    }

    #[tokio::test]
    async fn list_targets_connection_refused() {
        // Use a port that is (almost certainly) not listening
        let client = CdpClient::new(19999);
        let result = client.list_targets().await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("CDP list targets failed") || err_str.contains("Network"),
            "unexpected error: {err_str}"
        );
    }

    #[tokio::test]
    async fn new_tab_connection_refused() {
        let client = CdpClient::new(19999);
        let result = client.new_tab("https://example.com").await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("CDP new tab failed") || err_str.contains("Network"),
            "unexpected error: {err_str}"
        );
    }

    #[tokio::test]
    async fn close_tab_connection_refused() {
        let client = CdpClient::new(19999);
        let result = client.close_tab("some-target-id").await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("CDP close tab failed") || err_str.contains("Network"),
            "unexpected error: {err_str}"
        );
    }

    #[tokio::test]
    async fn version_connection_refused() {
        let client = CdpClient::new(19999);
        let result = client.version().await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("CDP version failed") || err_str.contains("Network"),
            "unexpected error: {err_str}"
        );
    }
}
