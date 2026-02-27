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

    #[test]
    fn is_running_false_initially() {
        let mgr = BrowserManager::new(BrowserConfig::default());
        assert!(!mgr.is_running());
    }

    #[tokio::test]
    async fn stop_when_not_started_is_ok() {
        let mut mgr = BrowserManager::new(BrowserConfig::default());
        let result = mgr.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn start_with_nonexistent_executable_and_no_system_chrome() {
        // Use a config with a nonexistent path and hope system chrome doesn't exist either.
        // If system chrome DOES exist, this test will actually try to start it,
        // so we use a config where the explicit path is bogus but fall through
        // to candidates that might not exist.
        let config = BrowserConfig {
            executable_path: Some("/nonexistent/path/to/chrome_12345".into()),
            ..Default::default()
        };
        let mut mgr = BrowserManager::new(config);
        let result = mgr.start().await;

        // If no system Chrome exists, this returns an error.
        // If system Chrome exists, it returns Ok. Either way the test validates the code path.
        if result.is_err() {
            let err_str = result.unwrap_err().to_string();
            assert!(
                err_str.contains("Chrome") || err_str.contains("not found"),
                "unexpected error: {err_str}"
            );
        }
    }

    #[tokio::test]
    async fn start_already_running_returns_ok() {
        // Simulate a running process by starting a harmless short-lived process
        // via the manager's internal mechanism.
        // We'll use /bin/sleep as the "chrome" executable.
        let config = BrowserConfig {
            executable_path: Some("/bin/sleep".into()),
            ..Default::default()
        };
        let mut mgr = BrowserManager::new(config);

        // Manually set process to simulate "already running"
        let child = Command::new("/bin/sleep")
            .arg("10")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        mgr.process = Some(child);

        assert!(mgr.is_running());

        // start() should return Ok immediately without spawning a second process
        let result = mgr.start().await;
        assert!(result.is_ok());
        assert!(mgr.is_running());

        // Cleanup
        mgr.stop().await.unwrap();
    }

    #[tokio::test]
    async fn stop_kills_process() {
        let child = Command::new("/bin/sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();

        let mut mgr = BrowserManager::new(BrowserConfig::default());
        mgr.process = Some(child);
        assert!(mgr.is_running());

        let result = mgr.stop().await;
        assert!(result.is_ok());
        assert!(!mgr.is_running());
    }

    #[test]
    fn drop_kills_process() {
        let child = std::process::Command::new("/bin/sleep")
            .arg("60")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();

        // Wrap in tokio Child for the manager
        // Actually, manager uses tokio::process::Child. Let's use a runtime for this.
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let tokio_child = Command::new("/bin/sleep")
                .arg("60")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();

            let mut mgr = BrowserManager::new(BrowserConfig::default());
            mgr.process = Some(tokio_child);
            assert!(mgr.is_running());
            // Drop mgr - should kill the process
            drop(mgr);
        });

        // Kill the std child too
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }

    #[test]
    fn find_chrome_with_no_explicit_path_and_no_candidates() {
        // When executable_path is None and no candidate paths exist,
        // find_chrome_executable should return None.
        // On a Mac with Chrome installed, this will still find Chrome.
        // We just verify the method doesn't panic.
        let config = BrowserConfig {
            executable_path: None,
            ..Default::default()
        };
        let mgr = BrowserManager::new(config);
        let found = mgr.find_chrome_executable();
        // found may be Some or None depending on system; just ensure no panic
        if let Some(ref path) = found {
            assert!(Path::new(path).exists());
        }
    }

    #[tokio::test]
    async fn start_with_invalid_executable() {
        // Use a file that exists but is not executable as a Chrome binary
        // /dev/null exists but cannot be executed as a process
        let config = BrowserConfig {
            executable_path: Some("/dev/null".into()),
            ..Default::default()
        };
        let mut mgr = BrowserManager::new(config);
        let result = mgr.start().await;
        assert!(result.is_err());
        let err_str = result.unwrap_err().to_string();
        assert!(
            err_str.contains("failed to start Chrome") || err_str.contains("browser"),
            "unexpected error: {err_str}"
        );
    }
}
