//! Sandboxed server instances for parallel test execution.
//!
//! [`SandboxedServer`] is the core abstraction: each test gets its own
//! server with an isolated port, database, and config. The `InProcess`
//! mode calls [`ironclad_server::bootstrap_with_config_path`] and binds
//! the resulting router to a real TCP listener — giving full TCP/WS
//! fidelity while contributing to code coverage.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use tempfile::TempDir;

use crate::client::HarnessClient;
use crate::config_gen::{ConfigOverrides, generate_config};

/// How the sandbox runs the server.
#[derive(Debug, Clone, Copy)]
pub enum SandboxMode {
    /// Bootstrap in-process and bind to a real TCP listener.
    /// Contributes to coverage, ~10x faster than Process mode.
    InProcess,

    /// Spawn the real `ironclad` binary as a child process.
    /// Highest fidelity, no coverage contribution. Requires `full-process` feature.
    #[cfg(feature = "full-process")]
    Process,
}

/// An isolated server instance with its own port, database, and config.
///
/// Drop cleans up the temp directory and shuts down the server task.
pub struct SandboxedServer {
    /// TCP port the server is listening on.
    pub port: u16,
    /// Base URL for HTTP requests (e.g., `http://127.0.0.1:49200`).
    pub base_url: String,
    /// API key configured for this sandbox (if any).
    pub api_key: Option<String>,
    /// Path to the generated config file.
    pub config_path: PathBuf,
    /// Path to the SQLite database.
    pub db_path: PathBuf,

    // Internal state
    _tmp_dir: TempDir,
    _server_handle: Option<tokio::task::JoinHandle<()>>,
    #[cfg(feature = "full-process")]
    _child: Option<std::process::Child>,
}

impl SandboxedServer {
    /// Spawn a new sandboxed server with default config.
    pub async fn spawn(mode: SandboxMode) -> Result<Self, Box<dyn std::error::Error>> {
        Self::spawn_with(mode, ConfigOverrides::default()).await
    }

    /// Spawn a new sandboxed server with custom config overrides.
    pub async fn spawn_with(
        mode: SandboxMode,
        overrides: ConfigOverrides,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let tmp_dir = TempDir::new()?;
        let api_key = overrides.api_key.clone();
        let db_path = tmp_dir.path().join("state.db");

        match mode {
            SandboxMode::InProcess => {
                // Bind first so the kernel reserves a unique free port; this avoids
                // cross-process collisions during highly parallel test runs.
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let port = listener.local_addr()?.port();
                let config_path = generate_config(tmp_dir.path(), port, &overrides)?;
                let base_url = format!("http://127.0.0.1:{port}");
                let config = ironclad_core::IroncladConfig::from_file(&config_path)?;
                let router =
                    ironclad_server::bootstrap_with_config_path(config, Some(config_path.clone()))
                        .await?;

                let handle = tokio::spawn(async move {
                    let _ = axum::serve(
                        listener,
                        router.into_make_service_with_connect_info::<SocketAddr>(),
                    )
                    .await;
                });

                let server = Self {
                    port,
                    base_url,
                    api_key,
                    config_path,
                    db_path,
                    _tmp_dir: tmp_dir,
                    _server_handle: Some(handle),
                    #[cfg(feature = "full-process")]
                    _child: None,
                };

                server.wait_healthy(Duration::from_secs(10)).await?;
                Ok(server)
            }

            #[cfg(feature = "full-process")]
            SandboxMode::Process => {
                let port = crate::port_pool::allocate_port();
                let config_path = generate_config(tmp_dir.path(), port, &overrides)?;
                let base_url = format!("http://127.0.0.1:{port}");
                #[allow(deprecated)]
                let bin = assert_cmd::cargo::cargo_bin("ironclad");
                let child = std::process::Command::new(bin)
                    .args(["serve", "-c"])
                    .arg(&config_path)
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()?;

                let server = Self {
                    port,
                    base_url,
                    api_key,
                    config_path,
                    db_path,
                    _tmp_dir: tmp_dir,
                    _server_handle: None,
                    _child: Some(child),
                };

                server.wait_healthy(Duration::from_secs(30)).await?;
                Ok(server)
            }
        }
    }

    /// Wait for the server's `/api/health` endpoint to return 200.
    pub async fn wait_healthy(&self, timeout: Duration) -> Result<(), Box<dyn std::error::Error>> {
        let client = reqwest::Client::new();
        let url = format!("{}/api/health", self.base_url);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            match client.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => return Ok(()),
                _ => {}
            }

            if tokio::time::Instant::now() >= deadline {
                return Err(format!(
                    "server at {} did not become healthy within {timeout:?}",
                    self.base_url
                )
                .into());
            }

            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// Get a pre-configured HTTP client for this sandbox.
    pub fn client(&self) -> HarnessClient {
        HarnessClient::new(&self.base_url, self.api_key.clone())
    }
}

impl Drop for SandboxedServer {
    fn drop(&mut self) {
        if let Some(handle) = self._server_handle.take() {
            handle.abort();
        }

        #[cfg(feature = "full-process")]
        if let Some(mut child) = self._child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
