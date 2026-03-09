#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextConfig {
    #[serde(default = "default_max_context_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_soft_trim_ratio")]
    pub soft_trim_ratio: f64,
    #[serde(default = "default_hard_clear_ratio")]
    pub hard_clear_ratio: f64,
    #[serde(default = "default_preserve_recent")]
    pub preserve_recent: usize,
    #[serde(default)]
    pub checkpoint_enabled: bool,
    #[serde(default = "default_checkpoint_interval")]
    pub checkpoint_interval_turns: u32,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            max_tokens: default_max_context_tokens(),
            soft_trim_ratio: default_soft_trim_ratio(),
            hard_clear_ratio: default_hard_clear_ratio(),
            preserve_recent: default_preserve_recent(),
            checkpoint_enabled: false,
            checkpoint_interval_turns: default_checkpoint_interval(),
        }
    }
}

fn default_checkpoint_interval() -> u32 {
    10
}

fn default_max_context_tokens() -> usize {
    128_000
}
fn default_soft_trim_ratio() -> f64 {
    0.8
}
fn default_hard_clear_ratio() -> f64 {
    0.95
}
fn default_preserve_recent() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalsConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub gated_tools: Vec<String>,
    #[serde(default)]
    pub blocked_tools: Vec<String>,
    #[serde(default = "default_approval_timeout")]
    pub timeout_seconds: u64,
}

impl Default for ApprovalsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gated_tools: Vec::new(),
            blocked_tools: Vec::new(),
            timeout_seconds: default_approval_timeout(),
        }
    }
}

fn default_approval_timeout() -> u64 {
    300
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginsConfig {
    #[serde(default = "default_plugins_dir")]
    pub dir: PathBuf,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
    #[serde(default)]
    pub strict_permissions: bool,
    #[serde(default)]
    pub allowed_permissions: Vec<String>,
}

impl Default for PluginsConfig {
    fn default() -> Self {
        Self {
            dir: default_plugins_dir(),
            allow: Vec::new(),
            deny: Vec::new(),
            strict_permissions: true,
            allowed_permissions: Vec::new(),
        }
    }
}

fn default_plugins_dir() -> PathBuf {
    dirs_next().join("plugins")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub executable_path: Option<String>,
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_browser_profile_dir")]
    pub profile_dir: PathBuf,
    #[serde(default = "default_cdp_port")]
    pub cdp_port: u16,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            executable_path: None,
            headless: true,
            profile_dir: default_browser_profile_dir(),
            cdp_port: default_cdp_port(),
        }
    }
}

fn default_cdp_port() -> u16 {
    9222
}

fn default_browser_profile_dir() -> PathBuf {
    dirs_next().join("browser-profiles")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    #[serde(default)]
    pub auto_restart: bool,
    #[serde(default = "default_pid_file")]
    pub pid_file: PathBuf,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            auto_restart: false,
            pid_file: default_pid_file(),
        }
    }
}

fn default_pid_file() -> PathBuf {
    dirs_next().join("ironclad.pid")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateConfig {
    #[serde(default = "default_true")]
    pub check_on_start: bool,
    #[serde(default = "default_update_channel")]
    pub channel: String,
    /// Legacy single-registry URL. Kept for backward compatibility.
    /// If `registries` is empty and this is set, `resolve_registries()` synthesizes
    /// a single `RegistrySource` from it.
    #[serde(default = "default_update_registry_url")]
    pub registry_url: String,
    /// Multi-registry support: ordered list of skill registries to sync from.
    /// Higher-priority registries win on name collision.
    #[serde(default)]
    pub registries: Vec<RegistrySource>,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_start: true,
            channel: default_update_channel(),
            registry_url: default_update_registry_url(),
            registries: Vec::new(),
        }
    }
}

impl UpdateConfig {
    /// Resolve the effective list of registries. If explicit `registries` is
    /// non-empty, returns those. Otherwise, falls back to the legacy
    /// `registry_url` field wrapped in a single `RegistrySource`.
    pub fn resolve_registries(&self) -> Vec<RegistrySource> {
        if !self.registries.is_empty() {
            return self.registries.clone();
        }
        vec![RegistrySource {
            name: "default".into(),
            url: self.registry_url.clone(),
            priority: 50,
            enabled: true,
        }]
    }
}

/// A remote skill registry that the agent can sync from.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySource {
    /// Human-readable name, used as the namespace prefix for remote skills
    /// (e.g. `"community"` → skills namespaced as `community/skill_name`).
    pub name: String,
    /// URL of the registry manifest (JSON endpoint).
    pub url: String,
    /// Priority for conflict resolution: higher wins when two registries
    /// publish a skill with the same name. Range: 0–100.
    #[serde(default = "default_registry_priority")]
    pub priority: u32,
    /// Whether this registry is actively synced.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_registry_priority() -> u32 {
    50
}

fn default_update_channel() -> String {
    "stable".into()
}

fn default_update_registry_url() -> String {
    "https://roboticus.ai/registry/manifest.json".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonalityConfig {
    #[serde(default = "default_os_file")]
    pub os_file: String,
    #[serde(default = "default_firmware_file")]
    pub firmware_file: String,
}

impl Default for PersonalityConfig {
    fn default() -> Self {
        Self {
            os_file: default_os_file(),
            firmware_file: default_firmware_file(),
        }
    }
}

fn default_os_file() -> String {
    "OS.toml".into()
}
fn default_firmware_file() -> String {
    "FIRMWARE.toml".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionConfig {
    #[serde(default = "default_session_ttl")]
    pub ttl_seconds: u64,
    #[serde(default = "default_session_scope_mode")]
    pub scope_mode: String,
    #[serde(default)]
    pub reset_schedule: Option<String>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            ttl_seconds: default_session_ttl(),
            scope_mode: default_session_scope_mode(),
            reset_schedule: None,
        }
    }
}

fn default_session_ttl() -> u64 {
    86400
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpConfig {
    #[serde(default)]
    pub server_enabled: bool,
    #[serde(default = "default_mcp_port")]
    pub server_port: u16,
    #[serde(default)]
    pub clients: Vec<McpClientConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpClientConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub transport: McpTransport,
    #[serde(default)]
    pub auth_token_env: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub enum McpTransport {
    #[default]
    Sse,
    Stdio,
    Http,
    WebSocket,
}

fn default_mcp_port() -> u16 {
    3001
}

impl Default for McpConfig {
    fn default() -> Self {
        Self {
            server_enabled: false,
            server_port: default_mcp_port(),
            clients: Vec::new(),
        }
    }
}

fn default_session_scope_mode() -> String {
    "agent".into()
}
