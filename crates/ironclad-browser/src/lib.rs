pub mod actions;
pub mod cdp;
pub mod manager;
pub mod session;

pub use ironclad_core::config::BrowserConfig;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageInfo {
    pub id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScreenshotResult {
    pub data_base64: String,
    pub format: String,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageContent {
    pub url: String,
    pub title: String,
    pub text: String,
    pub html_length: usize,
}

use std::sync::Arc;
use tokio::sync::RwLock;

use ironclad_core::Result;

/// High-level browser facade combining process management, CDP control, and action execution.
pub struct Browser {
    config: BrowserConfig,
    manager: RwLock<manager::BrowserManager>,
    session: RwLock<Option<session::CdpSession>>,
}

impl Browser {
    pub fn new(config: BrowserConfig) -> Self {
        let mgr = manager::BrowserManager::new(config.clone());
        Self {
            config,
            manager: RwLock::new(mgr),
            session: RwLock::new(None),
        }
    }

    pub async fn start(&self) -> Result<()> {
        let mut mgr = self.manager.write().await;
        mgr.start().await?;

        let cdp = cdp::CdpClient::new(self.config.cdp_port);

        let mut attempts = 0;
        let targets = loop {
            match cdp.list_targets().await {
                Ok(t) if !t.is_empty() => break t,
                _ if attempts < 10 => {
                    attempts += 1;
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                Ok(_) => {
                    return Err(ironclad_core::IroncladError::Tool {
                        tool: "browser".into(),
                        message: "no CDP targets available after startup".into(),
                    });
                }
                Err(e) => return Err(e),
            }
        };

        let ws_url = targets
            .iter()
            .find(|t| t.target_type == "page")
            .and_then(|t| t.ws_url.clone())
            .ok_or_else(|| ironclad_core::IroncladError::Tool {
                tool: "browser".into(),
                message: "no page target with WebSocket URL found".into(),
            })?;

        let sess = session::CdpSession::connect(&ws_url).await?;
        sess.send_command("Page.enable", serde_json::json!({})).await?;
        sess.send_command("DOM.enable", serde_json::json!({})).await?;
        sess.send_command("Network.enable", serde_json::json!({})).await?;
        sess.send_command("Runtime.enable", serde_json::json!({})).await?;

        *self.session.write().await = Some(sess);
        Ok(())
    }

    pub async fn stop(&self) -> Result<()> {
        if let Some(sess) = self.session.write().await.take() {
            let _ = sess.close().await;
        }
        self.manager.write().await.stop().await
    }

    pub async fn is_running(&self) -> bool {
        self.manager.read().await.is_running()
    }

    pub async fn execute_action(
        &self,
        action: &actions::BrowserAction,
    ) -> actions::ActionResult {
        let session_guard = self.session.read().await;
        match session_guard.as_ref() {
            Some(sess) => actions::ActionExecutor::execute(sess, action).await,
            None => actions::ActionResult::err(
                &format!("{:?}", action),
                "browser not started".into(),
            ),
        }
    }

    pub fn cdp_port(&self) -> u16 {
        self.config.cdp_port
    }
}

/// Thread-safe wrapper for shared ownership.
pub type SharedBrowser = Arc<Browser>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_config_defaults() {
        let cfg = BrowserConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.headless);
        assert_eq!(cfg.cdp_port, 9222);
        assert!(cfg.executable_path.is_none());
    }

    #[test]
    fn page_info_serde() {
        let info = PageInfo {
            id: "page1".into(),
            url: "https://example.com".into(),
            title: "Example".into(),
        };
        let json = serde_json::to_string(&info).unwrap();
        let back: PageInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "page1");
        assert_eq!(back.url, "https://example.com");
    }

    #[test]
    fn screenshot_result_serde() {
        let result = ScreenshotResult {
            data_base64: "abc123".into(),
            format: "png".into(),
            width: 1920,
            height: 1080,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ScreenshotResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.width, 1920);
    }

    #[test]
    fn browser_facade_creation() {
        let browser = Browser::new(BrowserConfig::default());
        assert_eq!(browser.cdp_port(), 9222);
    }

    #[tokio::test]
    async fn browser_not_running_initially() {
        let browser = Browser::new(BrowserConfig::default());
        assert!(!browser.is_running().await);
    }

    #[tokio::test]
    async fn execute_action_without_start_returns_error() {
        let browser = Browser::new(BrowserConfig::default());
        let result = browser
            .execute_action(&actions::BrowserAction::Screenshot)
            .await;
        assert!(!result.success);
        assert!(result.error.as_deref().unwrap().contains("not started"));
    }
}
