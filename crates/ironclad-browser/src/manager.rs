use std::path::Path;
use std::process::Stdio;

use tokio::process::{Child, Command};
use tracing::{debug, info};

use ironclad_core::config::BrowserConfig;
use ironclad_core::{IroncladError, Result};

pub struct BrowserManager {
    config: BrowserConfig,
    process: Option<Child>,
}

impl BrowserManager {
    pub fn new(config: BrowserConfig) -> Self {
        Self {
            config,
            process: None,
        }
    }

    // SECURITY: `executable_path` comes from the server configuration file
    // which is trusted (written by an administrator). We do not sanitize
    // or restrict the path beyond checking that the file exists.
    fn find_chrome_executable(&self) -> Option<String> {
        if let Some(ref path) = self.config.executable_path
            && Path::new(path).exists()
        {
            return Some(path.clone());
        }

        let candidates = [
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/usr/bin/google-chrome",
            "/usr/bin/google-chrome-stable",
            "/usr/bin/chromium",
            "/usr/bin/chromium-browser",
            "/snap/bin/chromium",
        ];

        for candidate in &candidates {
            if Path::new(candidate).exists() {
                return Some(candidate.to_string());
            }
        }

        None
    }

    pub async fn start(&mut self) -> Result<()> {
        if self.process.is_some() {
            return Ok(());
        }

        let executable = self
            .find_chrome_executable()
            .ok_or_else(|| IroncladError::Tool {
                tool: "browser".into(),
                message: "Chrome/Chromium not found".into(),
            })?;

        let profile = self.config.profile_dir.display().to_string();
        let mut args = vec![
            format!("--remote-debugging-port={}", self.config.cdp_port),
            format!("--user-data-dir={profile}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            "--disable-background-networking".to_string(),
            "--disable-extensions".to_string(),
            "--disable-plugins".to_string(),
            "--disable-popup-blocking".to_string(),
            "--disable-component-update".to_string(),
        ];

        if self.config.headless {
            args.push("--headless=new".to_string());
        }

        info!(
            executable_path = %executable,
            port = self.config.cdp_port,
            headless = self.config.headless,
            "starting browser"
        );

        let child = Command::new(&executable)
            .args(&args)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| IroncladError::Tool {
                tool: "browser".into(),
                message: format!("failed to start Chrome: {e}"),
            })?;

        self.process = Some(child);

        // Brief grace period for the browser process to initialize its CDP
        // listener. The caller (Browser::start in lib.rs) retries
        // cdp.list_targets() up to 10 times with 300ms back-off, so this
        // initial delay only needs to cover typical startup jitter.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        debug!("browser process spawned, CDP listener may still be initializing");
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<()> {
        if let Some(mut child) = self.process.take() {
            debug!("stopping browser");
            child.kill().await.map_err(|e| IroncladError::Tool {
                tool: "browser".into(),
                message: format!("failed to stop Chrome: {e}"),
            })?;
        }
        Ok(())
    }

    pub fn is_running(&self) -> bool {
        self.process.is_some()
    }

    pub fn cdp_port(&self) -> u16 {
        self.config.cdp_port
    }
}

impl Drop for BrowserManager {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_defaults() {
        let mgr = BrowserManager::new(BrowserConfig::default());
        assert!(!mgr.is_running());
        assert_eq!(mgr.cdp_port(), 9222);
    }

    #[test]
    fn find_chrome_with_explicit_path() {
        let config = BrowserConfig {
            executable_path: Some("/usr/bin/false".into()),
            ..Default::default()
        };
        let mgr = BrowserManager::new(config);
        let found = mgr.find_chrome_executable();
        assert!(found.is_some());
        assert_eq!(found.unwrap(), "/usr/bin/false");
    }

    #[test]
    fn find_chrome_explicit_nonexistent() {
        let config = BrowserConfig {
            executable_path: Some("/nonexistent/chrome".into()),
            ..Default::default()
        };
        let mgr = BrowserManager::new(config);
        let found = mgr.find_chrome_executable();
        if let Some(path) = found {
            assert!(Path::new(&path).exists());
        }
    }

    #[test]
    fn custom_cdp_port() {
        let config = BrowserConfig {
            cdp_port: 9333,
            ..Default::default()
        };
        let mgr = BrowserManager::new(config);
        assert_eq!(mgr.cdp_port(), 9333);
    }
}
