use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultimodalConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub media_dir: Option<PathBuf>,
    #[serde(default = "default_max_image_size")]
    pub max_image_size_bytes: usize,
    #[serde(default = "default_max_audio_size")]
    pub max_audio_size_bytes: usize,
    #[serde(default = "default_max_video_size")]
    pub max_video_size_bytes: usize,
    #[serde(default = "default_max_document_size")]
    pub max_document_size_bytes: usize,
    #[serde(default)]
    pub vision_model: Option<String>,
    #[serde(default)]
    pub transcription_model: Option<String>,
    /// Automatically transcribe audio attachments via the voice pipeline.
    #[serde(default)]
    pub auto_transcribe_audio: bool,
    /// Automatically describe images via the configured vision model.
    #[serde(default)]
    pub auto_describe_images: bool,
}

impl Default for MultimodalConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            media_dir: None,
            max_image_size_bytes: default_max_image_size(),
            max_audio_size_bytes: default_max_audio_size(),
            max_video_size_bytes: default_max_video_size(),
            max_document_size_bytes: default_max_document_size(),
            vision_model: None,
            transcription_model: None,
            auto_transcribe_audio: false,
            auto_describe_images: false,
        }
    }
}

fn default_max_image_size() -> usize {
    10 * 1024 * 1024
}

fn default_max_audio_size() -> usize {
    25 * 1024 * 1024 // 25 MB — Whisper API limit
}

fn default_max_video_size() -> usize {
    50 * 1024 * 1024 // 50 MB
}

fn default_max_document_size() -> usize {
    50 * 1024 * 1024 // 50 MB
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct KnowledgeConfig {
    #[serde(default)]
    pub sources: Vec<KnowledgeSourceEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSourceEntry {
    pub name: String,
    pub source_type: String,
    #[serde(default)]
    pub path: Option<PathBuf>,
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default = "default_max_chunks")]
    pub max_chunks: usize,
}

fn default_max_chunks() -> usize {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DigestConfig {
    #[serde(default = "default_digest_enabled")]
    pub enabled: bool,
    #[serde(default = "default_digest_max_tokens")]
    pub max_tokens: usize,
    #[serde(default = "default_decay_half_life_days")]
    pub decay_half_life_days: u32,
}

impl Default for DigestConfig {
    fn default() -> Self {
        Self {
            enabled: default_digest_enabled(),
            max_tokens: default_digest_max_tokens(),
            decay_half_life_days: default_decay_half_life_days(),
        }
    }
}

fn default_digest_enabled() -> bool {
    true
}
fn default_digest_max_tokens() -> usize {
    512
}
fn default_decay_half_life_days() -> u32 {
    7
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    #[serde(default)]
    pub soul_versioning: bool,
    #[serde(default)]
    pub index_on_start: bool,
    #[serde(default)]
    pub watch_for_changes: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IroncladConfig {
    pub agent: AgentConfig,
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub models: ModelsConfig,
    #[serde(default)]
    pub providers: HashMap<String, ProviderConfig>,
    #[serde(default)]
    pub circuit_breaker: CircuitBreakerConfig,
    #[serde(default)]
    pub memory: MemoryConfig,
    #[serde(default)]
    pub cache: CacheConfig,
    #[serde(default)]
    pub treasury: TreasuryConfig,
    #[serde(default)]
    pub r#yield: YieldConfig,
    #[serde(default)]
    pub wallet: WalletConfig,
    #[serde(default)]
    pub a2a: A2aConfig,
    #[serde(default)]
    pub skills: SkillsConfig,
    #[serde(default)]
    pub channels: ChannelsConfig,
    #[serde(default)]
    pub context: ContextConfig,
    #[serde(default)]
    pub approvals: ApprovalsConfig,
    #[serde(default)]
    pub plugins: PluginsConfig,
    #[serde(default)]
    pub browser: BrowserConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub update: UpdateConfig,
    #[serde(default)]
    pub tier_adapt: TierAdaptConfig,
    #[serde(default)]
    pub personality: PersonalityConfig,
    #[serde(default)]
    pub session: SessionConfig,
    #[serde(default)]
    pub digest: DigestConfig,
    #[serde(default)]
    pub multimodal: MultimodalConfig,
    #[serde(default)]
    pub knowledge: KnowledgeConfig,
    #[serde(default)]
    pub workspace_config: WorkspaceConfig,
    #[serde(default)]
    pub mcp: McpConfig,
    #[serde(default)]
    pub devices: DeviceConfig,
    #[serde(default)]
    pub discovery: DiscoveryConfig,
    #[serde(default)]
    pub obsidian: ObsidianConfig,
    #[serde(default)]
    pub security: SecurityConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveryConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub dns_sd: bool,
    #[serde(default)]
    pub mdns: bool,
    #[serde(default)]
    pub advertise: bool,
    #[serde(default = "default_service_name")]
    pub service_name: String,
}

fn default_service_name() -> String {
    "_ironclad._tcp".to_string()
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            dns_sd: false,
            mdns: false,
            advertise: false,
            service_name: default_service_name(),
        }
    }
}

// ── Security / RBAC ─────────────────────────────────────────────────────────

/// Claim-based RBAC configuration.
///
/// Controls how authentication layers compose into an effective authority.
/// See `ironclad_core::security::resolve_claim` for the composition algorithm.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SecurityConfig {
    /// When `true` (default), channels with empty allow-lists reject all
    /// messages. When `false`, empty allow-lists permit everyone (legacy).
    #[serde(default = "default_true")]
    pub deny_on_empty_allowlist: bool,

    /// Authority granted to senders who pass a channel's allow-list.
    /// Default: `Peer` (can use Safe + Caution tools like filesystem access).
    #[serde(default = "default_allowlist_authority")]
    pub allowlist_authority: crate::types::InputAuthority,

    /// Authority granted to senders in `channels.trusted_sender_ids`.
    /// Default: `Creator` (full access).
    #[serde(default = "default_trusted_authority")]
    pub trusted_authority: crate::types::InputAuthority,

    /// Authority granted to HTTP API / WebSocket callers.
    /// Default: `Creator`.
    #[serde(default = "default_api_authority")]
    pub api_authority: crate::types::InputAuthority,

    /// Maximum authority when the threat scanner returns Caution.
    /// Effective authority is capped at this level.
    /// Default: `External` (Safe tools only).
    #[serde(default = "default_threat_ceiling")]
    pub threat_caution_ceiling: crate::types::InputAuthority,
}

fn default_allowlist_authority() -> crate::types::InputAuthority {
    crate::types::InputAuthority::Peer
}
fn default_trusted_authority() -> crate::types::InputAuthority {
    crate::types::InputAuthority::Creator
}
fn default_api_authority() -> crate::types::InputAuthority {
    crate::types::InputAuthority::Creator
}
fn default_threat_ceiling() -> crate::types::InputAuthority {
    crate::types::InputAuthority::External
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            deny_on_empty_allowlist: true,
            allowlist_authority: default_allowlist_authority(),
            trusted_authority: default_trusted_authority(),
            api_authority: default_api_authority(),
            threat_caution_ceiling: default_threat_ceiling(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsidianConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub vault_path: Option<PathBuf>,
    #[serde(default)]
    pub auto_detect: bool,
    #[serde(default)]
    pub auto_detect_paths: Vec<PathBuf>,
    #[serde(default = "default_true")]
    pub index_on_start: bool,
    #[serde(default)]
    pub watch_for_changes: bool,
    #[serde(default = "default_obsidian_ignored_folders")]
    pub ignored_folders: Vec<String>,
    #[serde(default = "default_obsidian_template_folder")]
    pub template_folder: String,
    #[serde(default = "default_obsidian_default_folder")]
    pub default_folder: String,
    #[serde(default = "default_true")]
    pub preferred_destination: bool,
    #[serde(default = "default_obsidian_tag_boost")]
    pub tag_boost: f64,
}

impl Default for ObsidianConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            vault_path: None,
            auto_detect: false,
            auto_detect_paths: Vec::new(),
            index_on_start: true,
            watch_for_changes: false,
            ignored_folders: default_obsidian_ignored_folders(),
            template_folder: default_obsidian_template_folder(),
            default_folder: default_obsidian_default_folder(),
            preferred_destination: true,
            tag_boost: default_obsidian_tag_boost(),
        }
    }
}

fn default_obsidian_ignored_folders() -> Vec<String> {
    vec![".obsidian".into(), ".trash".into(), ".git".into()]
}

fn default_obsidian_template_folder() -> String {
    "templates".into()
}

fn default_obsidian_default_folder() -> String {
    "ironclad".into()
}

fn default_obsidian_tag_boost() -> f64 {
    0.2
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub identity_path: Option<PathBuf>,
    #[serde(default)]
    pub sync_enabled: bool,
    #[serde(default)]
    pub max_paired_devices: usize,
}

impl Default for DeviceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            identity_path: None,
            sync_enabled: false,
            max_paired_devices: 5,
        }
    }
}

const BUNDLED_PROVIDERS_TOML: &str = include_str!("bundled_providers.toml");

#[derive(Debug, Clone, Deserialize, Default)]
struct BundledProviders {
    #[serde(default)]
    providers: HashMap<String, ProviderConfig>,
}

impl IroncladConfig {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    /// Parse configuration from a TOML string.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironclad_core::config::IroncladConfig;
    ///
    /// let toml = r#"
    /// [agent]
    /// name = "Test"
    /// id = "test-1"
    /// workspace = "/tmp"
    /// log_level = "info"
    ///
    /// [server]
    /// bind = "127.0.0.1"
    /// port = 3001
    ///
    /// [database]
    /// path = "/tmp/test.db"
    ///
    /// [models]
    /// primary = "ollama/qwen3:8b"
    /// "#;
    /// let config = IroncladConfig::from_str(toml).unwrap();
    /// assert_eq!(config.server.port, 3001);
    /// ```
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(toml_str: &str) -> Result<Self> {
        let mut config: Self = toml::from_str(toml_str)?;
        config.database.path = expand_tilde(&config.database.path);
        config.agent.workspace = expand_tilde(&config.agent.workspace);
        config.server.log_dir = expand_tilde(&config.server.log_dir);
        config.skills.skills_dir = expand_tilde(&config.skills.skills_dir);
        config.wallet.path = expand_tilde(&config.wallet.path);
        config.plugins.dir = expand_tilde(&config.plugins.dir);
        config.browser.profile_dir = expand_tilde(&config.browser.profile_dir);
        config.daemon.pid_file = expand_tilde(&config.daemon.pid_file);
        if let Some(ref vp) = config.obsidian.vault_path {
            config.obsidian.vault_path = Some(expand_tilde(vp));
        }
        config.obsidian.auto_detect_paths = config
            .obsidian
            .auto_detect_paths
            .iter()
            .map(|p| expand_tilde(p))
            .collect();
        config.merge_bundled_providers();
        config.validate()?;
        Ok(config)
    }

    fn merge_bundled_providers(&mut self) {
        let bundled: BundledProviders = toml::from_str(BUNDLED_PROVIDERS_TOML)
            .expect("bundled providers TOML must parse — this is a build-time error");
        for (name, bundled_cfg) in bundled.providers {
            self.providers.entry(name).or_insert(bundled_cfg);
        }
    }

    pub fn bundled_providers_toml() -> &'static str {
        BUNDLED_PROVIDERS_TOML
    }

    pub fn validate(&self) -> Result<()> {
        if self.models.primary.is_empty() {
            return Err(IroncladError::Config(
                "models.primary must be non-empty".into(),
            ));
        }

        if self.agent.id.is_empty() {
            return Err(IroncladError::Config("agent.id must be non-empty".into()));
        }

        if self.agent.name.is_empty() {
            return Err(IroncladError::Config("agent.name must be non-empty".into()));
        }

        if !matches!(self.session.scope_mode.as_str(), "agent" | "peer" | "group") {
            return Err(IroncladError::Config(format!(
                "session.scope_mode must be one of \"agent\", \"peer\", \"group\", got \"{}\"",
                self.session.scope_mode
            )));
        }

        let sum = self.memory.working_budget_pct
            + self.memory.episodic_budget_pct
            + self.memory.semantic_budget_pct
            + self.memory.procedural_budget_pct
            + self.memory.relationship_budget_pct;

        if (sum - 100.0).abs() > 0.01 {
            return Err(IroncladError::Config(format!(
                "memory budget percentages must sum to 100, got {sum}"
            )));
        }

        if self.treasury.per_payment_cap <= 0.0 {
            return Err(IroncladError::Config(
                "treasury.per_payment_cap must be positive".into(),
            ));
        }

        if self.treasury.minimum_reserve < 0.0 {
            return Err(IroncladError::Config(
                "treasury.minimum_reserve must be non-negative".into(),
            ));
        }

        if self.server.bind.parse::<std::net::IpAddr>().is_err() && self.server.bind != "localhost"
        {
            return Err(IroncladError::Config(format!(
                "server.bind '{}' is not a valid IP address",
                self.server.bind
            )));
        }

        // ── Security validation ─────────────────────────────────
        // Allow-list authority must not exceed trusted authority (the allow-list
        // is a weaker authentication signal than trusted_sender_ids).
        if self.security.allowlist_authority > self.security.trusted_authority {
            return Err(IroncladError::Config(
                "security.allowlist_authority must be ≤ security.trusted_authority \
                 (allow-list is a weaker signal than trusted_sender_ids)"
                    .into(),
            ));
        }

        // Threat scanner ceiling must be below Creator. If the ceiling is Creator,
        // the threat scanner can never actually restrict anything — it's a no-op.
        if self.security.threat_caution_ceiling >= crate::types::InputAuthority::Creator {
            return Err(IroncladError::Config(
                "security.threat_caution_ceiling must be below Creator \
                 (otherwise the threat scanner has no effect)"
                    .into(),
            ));
        }

        let routing = &self.models.routing;
        if !(0.0..=1.0).contains(&routing.accuracy_floor) {
            return Err(IroncladError::Config(format!(
                "models.routing.accuracy_floor must be in [0.0, 1.0], got {}",
                routing.accuracy_floor
            )));
        }
        if routing.accuracy_min_obs == 0 {
            return Err(IroncladError::Config(
                "models.routing.accuracy_min_obs must be >= 1".into(),
            ));
        }
        if let Some(cost_weight) = routing.cost_weight
            && !(0.0..=1.0).contains(&cost_weight)
        {
            return Err(IroncladError::Config(format!(
                "models.routing.cost_weight must be in [0.0, 1.0], got {cost_weight}"
            )));
        }
        if !(0.0..=1.0).contains(&routing.canary_fraction) {
            return Err(IroncladError::Config(format!(
                "models.routing.canary_fraction must be in [0.0, 1.0], got {}",
                routing.canary_fraction
            )));
        }
        if routing.canary_model.is_none() && routing.canary_fraction > 0.0 {
            return Err(IroncladError::Config(
                "models.routing.canary_fraction > 0 requires models.routing.canary_model".into(),
            ));
        }
        if routing.canary_model.is_some() && routing.canary_fraction <= 0.0 {
            return Err(IroncladError::Config(
                "models.routing.canary_model requires models.routing.canary_fraction > 0".into(),
            ));
        }
        if let Some(canary_model) = &routing.canary_model {
            if canary_model.trim().is_empty() {
                return Err(IroncladError::Config(
                    "models.routing.canary_model must be non-empty when set".into(),
                ));
            }
            if routing.blocked_models.iter().any(|m| m == canary_model) {
                return Err(IroncladError::Config(format!(
                    "models.routing.canary_model '{canary_model}' must not appear in models.routing.blocked_models"
                )));
            }
        }
        if routing.blocked_models.iter().any(|m| m.trim().is_empty()) {
            return Err(IroncladError::Config(
                "models.routing.blocked_models entries must be non-empty".into(),
            ));
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub id: String,
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_true")]
    pub delegation_enabled: bool,
    #[serde(default = "default_min_decomposition_complexity")]
    pub delegation_min_complexity: f64,
    #[serde(default = "default_min_delegation_utility_margin")]
    pub delegation_min_utility_margin: f64,
    #[serde(default = "default_true")]
    pub specialist_creation_requires_approval: bool,
}

fn default_workspace() -> PathBuf {
    dirs_next().join("workspace")
}

fn default_log_level() -> String {
    "info".into()
}

fn default_min_decomposition_complexity() -> f64 {
    0.35
}

fn default_min_delegation_utility_margin() -> f64 {
    0.15
}

fn default_log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".ironclad").join("logs")
}

fn default_log_max_days() -> u32 {
    7
}

fn dirs_next() -> PathBuf {
    home_dir().join(".ironclad")
}

/// Returns the user's home directory, checking `HOME` first (Unix / MSYS2 / Git Bash)
/// then `USERPROFILE` (native Windows). Falls back to the platform temp directory.
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Resolves the configuration file path using a standard precedence chain:
///
/// 1. Explicit path (from `--config` flag or `IRONCLAD_CONFIG` env var)
/// 2. `~/.ironclad/ironclad.toml` (if it exists)
/// 3. `./ironclad.toml` in the current working directory (if it exists)
/// 4. `None` — caller decides the fallback (e.g., built-in defaults or error)
pub fn resolve_config_path(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(PathBuf::from(p));
    }
    let home_config = home_dir().join(".ironclad").join("ironclad.toml");
    if home_config.exists() {
        return Some(home_config);
    }
    let cwd_config = PathBuf::from("ironclad.toml");
    if cwd_config.exists() {
        return Some(cwd_config);
    }
    None
}

/// Expands a leading `~` in `path` to the user's home directory; otherwise returns the path unchanged.
fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        home_dir().join(stripped)
    } else {
        path.to_path_buf()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind", alias = "host")]
    pub bind: String,
    #[serde(default, skip_serializing)]
    pub api_key: Option<String>,
    #[serde(default = "default_log_dir")]
    pub log_dir: PathBuf,
    #[serde(default = "default_log_max_days")]
    pub log_max_days: u32,
    #[serde(default = "default_rate_limit_requests")]
    pub rate_limit_requests: u32,
    #[serde(default = "default_rate_limit_window_secs")]
    pub rate_limit_window_secs: u64,
    #[serde(default = "default_per_ip_rate_limit_requests")]
    pub per_ip_rate_limit_requests: u32,
    #[serde(default = "default_per_actor_rate_limit_requests")]
    pub per_actor_rate_limit_requests: u32,
    #[serde(default)]
    pub trusted_proxy_cidrs: Vec<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: default_port(),
            bind: default_bind(),
            api_key: None,
            log_dir: default_log_dir(),
            log_max_days: default_log_max_days(),
            rate_limit_requests: default_rate_limit_requests(),
            rate_limit_window_secs: default_rate_limit_window_secs(),
            per_ip_rate_limit_requests: default_per_ip_rate_limit_requests(),
            per_actor_rate_limit_requests: default_per_actor_rate_limit_requests(),
            trusted_proxy_cidrs: Vec::new(),
        }
    }
}

fn default_rate_limit_requests() -> u32 {
    100
}

fn default_rate_limit_window_secs() -> u64 {
    60
}

fn default_per_ip_rate_limit_requests() -> u32 {
    300
}

fn default_per_actor_rate_limit_requests() -> u32 {
    200
}

fn default_port() -> u16 {
    18789
}

fn default_bind() -> String {
    "127.0.0.1".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    #[serde(default = "default_db_path")]
    pub path: PathBuf,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            path: default_db_path(),
        }
    }
}

fn default_db_path() -> PathBuf {
    dirs_next().join("state.db")
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelsConfig {
    pub primary: String,
    #[serde(default)]
    pub fallbacks: Vec<String>,
    #[serde(default)]
    pub routing: RoutingConfig,
    #[serde(default)]
    pub model_overrides: HashMap<String, ModelOverride>,
    #[serde(default)]
    pub stream_by_default: bool,
    #[serde(default)]
    pub tiered_inference: TieredInferenceConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TieredInferenceConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_confidence_floor")]
    pub confidence_floor: f64,
    #[serde(default = "default_escalation_latency_ms")]
    pub escalation_latency_budget_ms: u64,
}

fn default_confidence_floor() -> f64 {
    0.6
}
fn default_escalation_latency_ms() -> u64 {
    3000
}

impl Default for TieredInferenceConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            confidence_floor: default_confidence_floor(),
            escalation_latency_budget_ms: default_escalation_latency_ms(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingConfig {
    #[serde(default = "default_routing_mode")]
    pub mode: String,
    #[serde(default = "default_confidence_threshold")]
    pub confidence_threshold: f64,
    #[serde(default = "default_true")]
    pub local_first: bool,
    #[serde(default)]
    pub cost_aware: bool,
    #[serde(default = "default_estimated_output_tokens")]
    pub estimated_output_tokens: u32,
    /// Minimum observed quality score (0.0–1.0) for a model to be considered
    /// during metascore routing.  Models with fewer than `accuracy_min_obs`
    /// observations are exempt (insufficient data). Set to 0.0 to disable.
    #[serde(default)]
    pub accuracy_floor: f64,
    /// Minimum observations before the accuracy floor applies to a model.
    #[serde(default = "default_accuracy_min_obs")]
    pub accuracy_min_obs: usize,
    /// Custom cost weight for metascore \[0.0–1.0\]. When set, replaces the
    /// binary `cost_aware` toggle with a continuous dial: 0.0 = ignore cost,
    /// 1.0 = maximize savings. Efficacy weight adjusts inversely.
    /// When `None`, falls back to `cost_aware` boolean behavior.
    #[serde(default)]
    pub cost_weight: Option<f64>,
    /// Canary model to route a fraction of traffic through for A/B validation.
    /// When set, `canary_fraction` of requests are routed to this model instead
    /// of the metascore winner. Set to `None` to disable canary routing.
    #[serde(default)]
    pub canary_model: Option<String>,
    /// Fraction of requests routed to the canary model [0.0–1.0].
    /// Only effective when `canary_model` is set. Default: 0.0 (disabled).
    #[serde(default)]
    pub canary_fraction: f64,
    /// Static model blocklist — models listed here are unconditionally excluded
    /// from all routing paths (override, metascore, fallback). Useful as an
    /// instant kill-switch without restarting the server.
    #[serde(default)]
    pub blocked_models: Vec<String>,
}

impl Default for RoutingConfig {
    fn default() -> Self {
        Self {
            mode: default_routing_mode(),
            confidence_threshold: default_confidence_threshold(),
            local_first: true,
            cost_aware: false,
            estimated_output_tokens: default_estimated_output_tokens(),
            accuracy_floor: 0.0,
            accuracy_min_obs: default_accuracy_min_obs(),
            cost_weight: None,
            canary_model: None,
            canary_fraction: 0.0,
            blocked_models: Vec::new(),
        }
    }
}

fn default_accuracy_min_obs() -> usize {
    10
}

fn default_estimated_output_tokens() -> u32 {
    500
}

fn default_routing_mode() -> String {
    "heuristic".into()
}

fn default_confidence_threshold() -> f64 {
    0.9
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub url: String,
    pub tier: String,
    #[serde(default)]
    pub format: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub chat_path: Option<String>,
    #[serde(default)]
    pub embedding_path: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default)]
    pub embedding_dimensions: Option<usize>,
    #[serde(default)]
    pub is_local: Option<bool>,
    #[serde(default)]
    pub cost_per_input_token: Option<f64>,
    #[serde(default)]
    pub cost_per_output_token: Option<f64>,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub extra_headers: Option<HashMap<String, String>>,
    #[serde(default)]
    pub tpm_limit: Option<u64>,
    #[serde(default)]
    pub rpm_limit: Option<u64>,
    #[serde(default)]
    pub auth_mode: Option<String>,
    #[serde(default)]
    pub oauth_client_id: Option<String>,
    #[serde(default)]
    pub oauth_redirect_uri: Option<String>,
    #[serde(default)]
    pub api_key_ref: Option<String>,
}

impl ProviderConfig {
    pub fn new(url: impl Into<String>, tier: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            tier: tier.into(),
            format: None,
            api_key_env: None,
            chat_path: None,
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: None,
            cost_per_input_token: None,
            cost_per_output_token: None,
            auth_header: None,
            extra_headers: None,
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: None,
            oauth_client_id: None,
            oauth_redirect_uri: None,
            api_key_ref: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelOverride {
    #[serde(default)]
    pub tier: Option<String>,
    #[serde(default)]
    pub cost_per_input_token: Option<f64>,
    #[serde(default)]
    pub cost_per_output_token: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TierAdaptConfig {
    #[serde(default)]
    pub t1_strip_system: bool,
    #[serde(default)]
    pub t1_condense_turns: bool,
    #[serde(default = "default_t2_preamble")]
    pub t2_default_preamble: Option<String>,
    #[serde(default = "default_true")]
    pub t3_t4_passthrough: bool,
}

impl Default for TierAdaptConfig {
    fn default() -> Self {
        Self {
            t1_strip_system: false,
            t1_condense_turns: false,
            t2_default_preamble: default_t2_preamble(),
            t3_t4_passthrough: true,
        }
    }
}

fn default_t2_preamble() -> Option<String> {
    Some("Be concise and direct. Focus on accuracy.".into())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CircuitBreakerConfig {
    #[serde(default = "default_cb_threshold")]
    pub threshold: u32,
    #[serde(default = "default_cb_window")]
    pub window_seconds: u64,
    #[serde(default = "default_cb_cooldown")]
    pub cooldown_seconds: u64,
    /// DEPRECATED (v0.8.0): This field is never read by `circuit.rs`.
    /// Credit-tripped breakers use `credit_tripped` bool to prevent auto-recovery
    /// entirely, making a separate cooldown moot.  Kept for TOML backwards compat.
    #[serde(default = "default_cb_credit_cooldown")]
    pub credit_cooldown_seconds: u64,
    #[serde(default = "default_cb_max_cooldown")]
    pub max_cooldown_seconds: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            threshold: default_cb_threshold(),
            window_seconds: default_cb_window(),
            cooldown_seconds: default_cb_cooldown(),
            credit_cooldown_seconds: default_cb_credit_cooldown(),
            max_cooldown_seconds: default_cb_max_cooldown(),
        }
    }
}

fn default_cb_threshold() -> u32 {
    3
}
fn default_cb_window() -> u64 {
    60
}
fn default_cb_cooldown() -> u64 {
    60
}
fn default_cb_credit_cooldown() -> u64 {
    300
}
fn default_cb_max_cooldown() -> u64 {
    900
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    #[serde(default = "default_working_pct")]
    pub working_budget_pct: f64,
    #[serde(default = "default_episodic_pct")]
    pub episodic_budget_pct: f64,
    #[serde(default = "default_semantic_pct")]
    pub semantic_budget_pct: f64,
    #[serde(default = "default_procedural_pct")]
    pub procedural_budget_pct: f64,
    #[serde(default = "default_relationship_pct")]
    pub relationship_budget_pct: f64,
    #[serde(default)]
    pub embedding_provider: Option<String>,
    #[serde(default)]
    pub embedding_model: Option<String>,
    #[serde(default = "default_hybrid_weight")]
    pub hybrid_weight: f64,
    #[serde(default)]
    pub ann_index: bool,
}

impl Default for MemoryConfig {
    fn default() -> Self {
        Self {
            working_budget_pct: default_working_pct(),
            episodic_budget_pct: default_episodic_pct(),
            semantic_budget_pct: default_semantic_pct(),
            procedural_budget_pct: default_procedural_pct(),
            relationship_budget_pct: default_relationship_pct(),
            embedding_provider: None,
            embedding_model: None,
            hybrid_weight: default_hybrid_weight(),
            ann_index: false,
        }
    }
}

fn default_hybrid_weight() -> f64 {
    0.5
}

fn default_working_pct() -> f64 {
    30.0
}
fn default_episodic_pct() -> f64 {
    25.0
}
fn default_semantic_pct() -> f64 {
    20.0
}
fn default_procedural_pct() -> f64 {
    15.0
}
fn default_relationship_pct() -> f64 {
    10.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_cache_ttl")]
    pub exact_match_ttl_seconds: u64,
    #[serde(default = "default_semantic_threshold")]
    pub semantic_threshold: f64,
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    #[serde(default)]
    pub prompt_compression: bool,
    #[serde(default = "default_compression_ratio")]
    pub compression_target_ratio: f64,
}

impl Default for CacheConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            exact_match_ttl_seconds: default_cache_ttl(),
            semantic_threshold: default_semantic_threshold(),
            max_entries: default_max_entries(),
            prompt_compression: false,
            compression_target_ratio: default_compression_ratio(),
        }
    }
}

fn default_compression_ratio() -> f64 {
    0.5
}

fn default_cache_ttl() -> u64 {
    3600
}
fn default_semantic_threshold() -> f64 {
    0.95
}
fn default_max_entries() -> usize {
    10000
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreasuryConfig {
    #[serde(default = "default_per_payment_cap")]
    pub per_payment_cap: f64,
    #[serde(default = "default_hourly_limit")]
    pub hourly_transfer_limit: f64,
    #[serde(default = "default_daily_limit")]
    pub daily_transfer_limit: f64,
    #[serde(default = "default_min_reserve")]
    pub minimum_reserve: f64,
    #[serde(default = "default_inference_budget")]
    pub daily_inference_budget: f64,
}

impl Default for TreasuryConfig {
    fn default() -> Self {
        Self {
            per_payment_cap: default_per_payment_cap(),
            hourly_transfer_limit: default_hourly_limit(),
            daily_transfer_limit: default_daily_limit(),
            minimum_reserve: default_min_reserve(),
            daily_inference_budget: default_inference_budget(),
        }
    }
}

fn default_per_payment_cap() -> f64 {
    100.0
}
fn default_hourly_limit() -> f64 {
    500.0
}
fn default_daily_limit() -> f64 {
    2000.0
}
fn default_min_reserve() -> f64 {
    5.0
}
fn default_inference_budget() -> f64 {
    50.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_yield_protocol")]
    pub protocol: String,
    #[serde(default = "default_yield_chain")]
    pub chain: String,
    #[serde(default = "default_min_deposit")]
    pub min_deposit: f64,
    #[serde(default = "default_withdrawal_threshold")]
    pub withdrawal_threshold: f64,
    /// RPC URL for yield chain (e.g. Base Sepolia). If unset, deposit/withdraw use mock behavior.
    #[serde(default)]
    pub chain_rpc_url: Option<String>,
    /// Aave V3 Pool address. Default: Base Sepolia.
    #[serde(default = "default_yield_pool_address")]
    pub pool_address: String,
    /// Underlying asset (e.g. USDC) address for supply/withdraw. Default: Base Sepolia USDC.
    #[serde(default = "default_yield_usdc_address")]
    pub usdc_address: String,
    /// aToken address for balance checks (e.g. aBase Sepolia USDC).
    /// When `None`, falls back to the Base Sepolia aUSDC default.
    #[serde(default)]
    pub atoken_address: Option<String>,
}

impl Default for YieldConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            protocol: default_yield_protocol(),
            chain: default_yield_chain(),
            min_deposit: default_min_deposit(),
            withdrawal_threshold: default_withdrawal_threshold(),
            chain_rpc_url: None,
            pool_address: default_yield_pool_address(),
            usdc_address: default_yield_usdc_address(),
            atoken_address: None,
        }
    }
}

fn default_yield_protocol() -> String {
    "aave".into()
}
fn default_yield_chain() -> String {
    "base".into()
}
fn default_min_deposit() -> f64 {
    50.0
}
fn default_withdrawal_threshold() -> f64 {
    30.0
}

/// Aave V3 Pool on Base Sepolia
fn default_yield_pool_address() -> String {
    "0x07eA79F68B2B3df564D0A34F8e19D9B1e339814b".into()
}
/// USDC on Base Sepolia
fn default_yield_usdc_address() -> String {
    "0x036CbD53842c5426634e7929541eC2318f3dCF7e".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    #[serde(default = "default_wallet_path")]
    pub path: PathBuf,
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            path: default_wallet_path(),
            chain_id: default_chain_id(),
            rpc_url: default_rpc_url(),
        }
    }
}

fn default_wallet_path() -> PathBuf {
    dirs_next().join("wallet.json")
}

fn default_chain_id() -> u64 {
    8453
}

fn default_rpc_url() -> String {
    "https://mainnet.base.org".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct A2aConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_a2a_max_msg_size")]
    pub max_message_size: usize,
    #[serde(default = "default_a2a_rate_limit")]
    pub rate_limit_per_peer: u32,
    #[serde(default = "default_a2a_session_timeout")]
    pub session_timeout_seconds: u64,
    #[serde(default = "default_true")]
    pub require_on_chain_identity: bool,
    #[serde(default = "default_a2a_nonce_ttl")]
    pub nonce_ttl_seconds: u64,
}

impl Default for A2aConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            max_message_size: default_a2a_max_msg_size(),
            rate_limit_per_peer: default_a2a_rate_limit(),
            session_timeout_seconds: default_a2a_session_timeout(),
            require_on_chain_identity: true,
            nonce_ttl_seconds: default_a2a_nonce_ttl(),
        }
    }
}

fn default_a2a_max_msg_size() -> usize {
    65536
}
fn default_a2a_rate_limit() -> u32 {
    10
}
fn default_a2a_session_timeout() -> u64 {
    3600
}
fn default_a2a_nonce_ttl() -> u64 {
    7200
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsConfig {
    #[serde(default = "default_skills_dir")]
    pub skills_dir: PathBuf,
    #[serde(default = "default_script_timeout")]
    pub script_timeout_seconds: u64,
    #[serde(default = "default_script_max_output")]
    pub script_max_output_bytes: usize,
    #[serde(default = "default_interpreters")]
    pub allowed_interpreters: Vec<String>,
    #[serde(default = "default_true")]
    pub sandbox_env: bool,
    #[serde(default = "default_true")]
    pub hot_reload: bool,
}

impl Default for SkillsConfig {
    fn default() -> Self {
        Self {
            skills_dir: default_skills_dir(),
            script_timeout_seconds: default_script_timeout(),
            script_max_output_bytes: default_script_max_output(),
            allowed_interpreters: default_interpreters(),
            sandbox_env: true,
            hot_reload: true,
        }
    }
}

fn default_skills_dir() -> PathBuf {
    dirs_next().join("skills")
}
fn default_script_timeout() -> u64 {
    30
}
fn default_script_max_output() -> usize {
    1_048_576
}
fn default_interpreters() -> Vec<String> {
    #[cfg(windows)]
    {
        vec![
            "bash".into(),
            "python".into(),
            "python3".into(),
            "node".into(),
        ]
    }
    #[cfg(not(windows))]
    {
        vec!["bash".into(), "python3".into(), "node".into()]
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VoiceChannelConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub stt_model: Option<String>,
    #[serde(default)]
    pub tts_model: Option<String>,
    #[serde(default)]
    pub tts_voice: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelsConfig {
    #[serde(default)]
    pub telegram: Option<TelegramConfig>,
    #[serde(default)]
    pub whatsapp: Option<WhatsAppConfig>,
    #[serde(default)]
    pub discord: Option<DiscordConfig>,
    #[serde(default)]
    pub signal: Option<SignalConfig>,
    #[serde(default)]
    pub email: EmailConfig,
    #[serde(default)]
    pub voice: VoiceChannelConfig,
    /// Sender IDs (chat IDs, phone numbers) trusted with Creator-level authority.
    /// Messages from senders not in this list get External authority.
    /// Empty list means all senders are treated as External.
    #[serde(default)]
    pub trusted_sender_ids: Vec<String>,
    /// Estimated latency threshold (in seconds) above which a thinking indicator
    /// (🤖🧠…) is sent before LLM inference on any chat channel. Set to 0 to
    /// always send, or a very large value to effectively disable. Default: 30.
    #[serde(default = "default_thinking_threshold")]
    pub thinking_threshold_seconds: u64,
    /// Optional list of channels that should receive a direct startup
    /// announcement (for example: ["telegram", "whatsapp", "signal"]).
    /// If unset/empty/false/"none"/"null", startup announcements are disabled.
    #[serde(default)]
    pub startup_announcements: Option<StartupAnnouncementsConfig>,
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            telegram: None,
            whatsapp: None,
            discord: None,
            signal: None,
            email: EmailConfig::default(),
            voice: VoiceChannelConfig::default(),
            trusted_sender_ids: Vec::new(),
            thinking_threshold_seconds: default_thinking_threshold(),
            startup_announcements: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum StartupAnnouncementsConfig {
    Flag(bool),
    Text(String),
    Channels(Vec<String>),
}

impl ChannelsConfig {
    pub fn startup_announcement_channels(&self) -> Vec<String> {
        fn normalize_channel(s: &str) -> Option<String> {
            let v = s.trim().to_ascii_lowercase();
            if v.is_empty() || v == "none" || v == "null" || v == "false" {
                None
            } else {
                Some(v)
            }
        }

        let mut out = match &self.startup_announcements {
            None => Vec::new(),
            Some(StartupAnnouncementsConfig::Flag(_)) => Vec::new(),
            Some(StartupAnnouncementsConfig::Text(v)) => {
                normalize_channel(v).map(|s| vec![s]).unwrap_or_default()
            }
            Some(StartupAnnouncementsConfig::Channels(v)) => {
                v.iter().filter_map(|s| normalize_channel(s)).collect()
            }
        };
        out.sort();
        out.dedup();
        out
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub token_env: String,
    #[serde(default)]
    pub token_ref: Option<String>,
    #[serde(default)]
    pub allowed_chat_ids: Vec<i64>,
    #[serde(default = "default_poll_timeout")]
    pub poll_timeout_seconds: u64,
    #[serde(default)]
    pub webhook_mode: bool,
    #[serde(default)]
    pub webhook_path: Option<String>,
    #[serde(default)]
    pub webhook_secret: Option<String>,
}

fn default_thinking_threshold() -> u64 {
    30
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhatsAppConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub token_env: String,
    #[serde(default)]
    pub token_ref: Option<String>,
    #[serde(default)]
    pub phone_number_id: String,
    #[serde(default)]
    pub verify_token: String,
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
    /// App secret for webhook X-Hub-Signature-256 verification (HMAC-SHA256).
    #[serde(default)]
    pub app_secret: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub token_env: String,
    #[serde(default)]
    pub token_ref: Option<String>,
    #[serde(default)]
    pub application_id: String,
    #[serde(default)]
    pub allowed_guild_ids: Vec<String>,
}

/// Signal channel adapter configuration. Uses signal-cli's JSON-RPC daemon
/// as a local relay for sending and receiving messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Phone number registered with signal-cli (e.g. "+15551234567").
    #[serde(default)]
    pub phone_number: String,
    /// Base URL of the signal-cli JSON-RPC daemon (default: http://127.0.0.1:8080).
    #[serde(default = "default_signal_daemon_url")]
    pub daemon_url: String,
    /// Contacts (phone numbers) allowed to talk to the agent. Empty = allow all.
    #[serde(default)]
    pub allowed_numbers: Vec<String>,
}

fn default_signal_daemon_url() -> String {
    "http://127.0.0.1:8080".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub imap_host: String,
    #[serde(default = "default_imap_port")]
    pub imap_port: u16,
    #[serde(default)]
    pub smtp_host: String,
    #[serde(default = "default_smtp_port")]
    pub smtp_port: u16,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password_env: String,
    #[serde(default)]
    pub from_address: String,
    #[serde(default)]
    pub allowed_senders: Vec<String>,
    #[serde(default = "default_poll_interval")]
    pub poll_interval_seconds: u64,
    /// Environment variable name holding the OAuth2 access token (for Gmail XOAUTH2).
    #[serde(default)]
    pub oauth2_token_env: String,
    /// Prefer XOAUTH2 authentication over password-based login.
    #[serde(default)]
    pub use_oauth2: bool,
    /// Use IMAP IDLE for push notifications when the server supports it (default: true).
    #[serde(default = "default_imap_idle_enabled")]
    pub imap_idle_enabled: bool,
}

fn default_imap_idle_enabled() -> bool {
    true
}

impl Default for EmailConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            imap_host: String::new(),
            imap_port: default_imap_port(),
            smtp_host: String::new(),
            smtp_port: default_smtp_port(),
            username: String::new(),
            password_env: String::new(),
            from_address: String::new(),
            allowed_senders: Vec::new(),
            poll_interval_seconds: default_poll_interval(),
            oauth2_token_env: String::new(),
            use_oauth2: false,
            imap_idle_enabled: default_imap_idle_enabled(),
        }
    }
}

fn default_imap_port() -> u16 {
    993
}
fn default_smtp_port() -> u16 {
    587
}
fn default_poll_interval() -> u64 {
    30
}

fn default_poll_timeout() -> u64 {
    30
}

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
    #[serde(default = "default_update_registry_url")]
    pub registry_url: String,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self {
            check_on_start: true,
            channel: default_update_channel(),
            registry_url: default_update_registry_url(),
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn config_toml_roundtrip_preserves_values(port in 1024u16..=65535u16) {
            let toml_str = format!(r#"
[agent]
name = "TestBot"
id = "test"
workspace = "/tmp/test"
log_level = "debug"

[server]
bind = "127.0.0.1"
port = {port}

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#);
            let config = IroncladConfig::from_str(&toml_str).unwrap();
            assert_eq!(config.server.port, port);
            assert_eq!(config.server.bind, "127.0.0.1");
        }
    }

    fn minimal_toml() -> &'static str {
        r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#
    }

    #[test]
    fn parse_minimal_config() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.agent.name, "TestBot");
        assert_eq!(cfg.agent.id, "test");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.models.primary, "ollama/qwen3:8b");
    }

    #[test]
    fn defaults_applied() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.memory.working_budget_pct, 30.0);
        assert_eq!(cfg.memory.episodic_budget_pct, 25.0);
        assert_eq!(cfg.memory.semantic_budget_pct, 20.0);
        assert_eq!(cfg.memory.procedural_budget_pct, 15.0);
        assert_eq!(cfg.memory.relationship_budget_pct, 10.0);
        assert_eq!(cfg.cache.semantic_threshold, 0.95);
        assert_eq!(cfg.cache.max_entries, 10000);
        assert_eq!(cfg.treasury.per_payment_cap, 100.0);
        assert!(cfg.skills.sandbox_env);
        assert_eq!(cfg.skills.script_timeout_seconds, 30);
        assert_eq!(
            cfg.skills.allowed_interpreters,
            vec!["bash", "python3", "node"]
        );
        assert_eq!(cfg.a2a.max_message_size, 65536);
        assert_eq!(cfg.a2a.rate_limit_per_peer, 10);
        assert!(cfg.a2a.enabled);
    }

    #[test]
    fn memory_budget_validation_fail() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[memory]
working_budget_pct = 50.0
episodic_budget_pct = 25.0
semantic_budget_pct = 20.0
procedural_budget_pct = 15.0
relationship_budget_pct = 10.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("sum to 100"));
    }

    #[test]
    fn treasury_validation_fail() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[treasury]
per_payment_cap = -1.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("per_payment_cap"));
    }

    #[test]
    fn full_config_roundtrip() {
        let toml = r#"
[agent]
name = "Duncan Idaho"
id = "duncan"
workspace = "/tmp/workspace"
log_level = "debug"

[server]
port = 18789
bind = "0.0.0.0"

[database]
path = "/tmp/state.db"

[models]
primary = "openai/gpt-5.3-codex"
fallbacks = ["google/gemini-3-flash", "ollama/qwen3:14b"]

[models.routing]
mode = "ml"
confidence_threshold = 0.85
local_first = true

[providers.anthropic]
url = "https://api.anthropic.com"
tier = "T3"

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"

[circuit_breaker]
threshold = 5
window_seconds = 120

[memory]
working_budget_pct = 30.0
episodic_budget_pct = 25.0
semantic_budget_pct = 20.0
procedural_budget_pct = 15.0
relationship_budget_pct = 10.0

[cache]
enabled = true
exact_match_ttl_seconds = 7200
semantic_threshold = 0.92
max_entries = 5000

[treasury]
per_payment_cap = 50.0
hourly_transfer_limit = 200.0
daily_transfer_limit = 1000.0
minimum_reserve = 10.0
daily_inference_budget = 25.0

[yield]
enabled = false
protocol = "aave"
chain = "base"
min_deposit = 100.0
withdrawal_threshold = 50.0

[wallet]
path = "/tmp/wallet.json"
chain_id = 8453
rpc_url = "https://mainnet.base.org"

[a2a]
enabled = true
max_message_size = 32768
rate_limit_per_peer = 5
session_timeout_seconds = 1800
require_on_chain_identity = true

[skills]
skills_dir = "/tmp/skills"
script_timeout_seconds = 15
script_max_output_bytes = 524288
allowed_interpreters = ["bash", "python3"]
sandbox_env = true
hot_reload = true
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.agent.name, "Duncan Idaho");
        assert_eq!(cfg.models.routing.confidence_threshold, 0.85);
        assert!(
            cfg.providers.len() >= 2,
            "user providers plus bundled defaults"
        );
        assert!(cfg.providers.contains_key("anthropic"));
        assert!(cfg.providers.contains_key("ollama"));
        assert_eq!(cfg.providers["anthropic"].url, "https://api.anthropic.com");
        assert_eq!(cfg.providers["anthropic"].tier, "T3");
        assert_eq!(cfg.circuit_breaker.threshold, 5);
        assert_eq!(cfg.cache.semantic_threshold, 0.92);
        assert_eq!(cfg.treasury.per_payment_cap, 50.0);
        assert!(!cfg.r#yield.enabled);
        assert_eq!(cfg.a2a.max_message_size, 32768);
        assert_eq!(cfg.skills.script_timeout_seconds, 15);
        assert_eq!(cfg.skills.allowed_interpreters, vec!["bash", "python3"]);
    }

    #[test]
    fn config_from_missing_file() {
        let result = IroncladConfig::from_file(Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn skills_config_defaults() {
        let cfg = SkillsConfig::default();
        assert_eq!(cfg.script_timeout_seconds, 30);
        assert_eq!(cfg.script_max_output_bytes, 1_048_576);
        assert!(cfg.sandbox_env);
        assert!(cfg.hot_reload);
        #[cfg(windows)]
        assert_eq!(
            cfg.allowed_interpreters,
            vec!["bash", "python", "python3", "node"]
        );
        #[cfg(not(windows))]
        assert_eq!(cfg.allowed_interpreters, vec!["bash", "python3", "node"]);
    }

    #[test]
    fn new_config_defaults() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.context.max_tokens, 128_000);
        assert_eq!(cfg.context.soft_trim_ratio, 0.8);
        assert_eq!(cfg.context.preserve_recent, 10);
        assert!(!cfg.approvals.enabled);
        assert!(cfg.approvals.gated_tools.is_empty());
        assert!(!cfg.browser.enabled);
        assert!(cfg.browser.headless);
        assert!(!cfg.daemon.auto_restart);
        assert_eq!(cfg.memory.hybrid_weight, 0.5);
        assert!(cfg.memory.embedding_provider.is_none());
    }

    #[test]
    fn bundled_providers_merged_on_minimal_config() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(cfg.providers.contains_key("ollama"));
        assert!(cfg.providers.contains_key("openai"));
        assert!(cfg.providers.contains_key("anthropic"));
        assert!(cfg.providers.contains_key("google"));
        assert!(cfg.providers.contains_key("openrouter"));
        assert!(cfg.providers.contains_key("moonshot"));
        assert_eq!(cfg.providers["ollama"].tier, "T1");
        assert_eq!(cfg.providers["moonshot"].tier, "T2");
        assert_eq!(
            cfg.providers["anthropic"].format.as_deref(),
            Some("anthropic")
        );
        assert_eq!(cfg.providers["ollama"].is_local, Some(true));
    }

    #[test]
    fn user_provider_overrides_bundled() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[providers.ollama]
url = "http://custom-host:9999"
tier = "T2"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.providers["ollama"].url, "http://custom-host:9999");
        assert_eq!(cfg.providers["ollama"].tier, "T2");
        assert!(
            cfg.providers.contains_key("openai"),
            "bundled providers still present"
        );
    }

    #[test]
    fn tier_adapt_defaults() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!cfg.tier_adapt.t1_strip_system);
        assert!(!cfg.tier_adapt.t1_condense_turns);
        assert_eq!(
            cfg.tier_adapt.t2_default_preamble.as_deref(),
            Some("Be concise and direct. Focus on accuracy.")
        );
        assert!(cfg.tier_adapt.t3_t4_passthrough);
    }

    #[test]
    fn model_overrides_in_config() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "openai/gpt-4o"

[models.model_overrides."openai/gpt-4o"]
tier = "T4"
cost_per_input_token = 0.00005
cost_per_output_token = 0.00015
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        let ov = &cfg.models.model_overrides["openai/gpt-4o"];
        assert_eq!(ov.tier.as_deref(), Some("T4"));
        assert!((ov.cost_per_input_token.unwrap() - 0.00005).abs() < f64::EPSILON);
    }

    #[test]
    fn bundled_providers_toml_is_valid() {
        let toml_str = IroncladConfig::bundled_providers_toml();
        let parsed: BundledProviders = toml::from_str(toml_str).expect("bundled TOML must parse");
        assert!(!parsed.providers.is_empty());
    }

    #[test]
    fn context_checkpoint_config_defaults() {
        let cfg = ContextConfig::default();
        assert!(!cfg.checkpoint_enabled);
        assert_eq!(cfg.checkpoint_interval_turns, 10);
    }

    #[test]
    fn session_config_defaults() {
        let cfg = SessionConfig::default();
        assert_eq!(cfg.ttl_seconds, 86400);
        assert_eq!(cfg.scope_mode, "agent");
        assert!(cfg.reset_schedule.is_none());
    }

    #[test]
    fn digest_config_defaults() {
        let cfg = DigestConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.max_tokens, 512);
        assert_eq!(cfg.decay_half_life_days, 7);

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(full.digest.enabled);
        assert_eq!(full.digest.max_tokens, 512);
        assert_eq!(full.digest.decay_half_life_days, 7);
    }

    #[test]
    fn session_config_from_toml() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
ttl_seconds = 3600
scope_mode = "peer"
reset_schedule = "0 0 * * *"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.session.ttl_seconds, 3600);
        assert_eq!(cfg.session.scope_mode, "peer");
        assert_eq!(cfg.session.reset_schedule.as_deref(), Some("0 0 * * *"));
    }

    #[test]
    fn session_reset_schedule_accepts_timezone_prefix() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
reset_schedule = "CRON_TZ=UTC+02:00 0 9 * * *"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(
            cfg.session.reset_schedule.as_deref(),
            Some("CRON_TZ=UTC+02:00 0 9 * * *")
        );
    }

    #[test]
    fn tilde_expansion_in_database_path() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let expected = std::path::PathBuf::from(&home)
            .join(".ironclad")
            .join("state.db");
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "~/.ironclad/state.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(
            cfg.database.path, expected,
            "~/.ironclad/state.db should expand to $HOME/.ironclad/state.db"
        );
    }

    #[test]
    fn obsidian_config_defaults() {
        let cfg = ObsidianConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.vault_path.is_none());
        assert!(!cfg.auto_detect);
        assert!(cfg.auto_detect_paths.is_empty());
        assert!(cfg.index_on_start);
        assert!(!cfg.watch_for_changes);
        assert_eq!(cfg.ignored_folders, vec![".obsidian", ".trash", ".git"]);
        assert_eq!(cfg.template_folder, "templates");
        assert_eq!(cfg.default_folder, "ironclad");
        assert!(cfg.preferred_destination);
        assert!((cfg.tag_boost - 0.2).abs() < f64::EPSILON);

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!full.obsidian.enabled);
        assert!(full.obsidian.vault_path.is_none());
    }

    #[test]
    fn obsidian_config_from_toml() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[obsidian]
enabled = true
vault_path = "~/Documents/MyVault"
default_folder = "agent-notes"
tag_boost = 0.3
ignored_folders = [".obsidian", ".git"]
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert!(cfg.obsidian.enabled);
        assert!(cfg.obsidian.vault_path.is_some());
        let vp = cfg.obsidian.vault_path.unwrap();
        assert!(
            !vp.to_str().unwrap().starts_with("~"),
            "tilde should be expanded"
        );
        assert!(vp.to_str().unwrap().contains("Documents/MyVault"));
        assert_eq!(cfg.obsidian.default_folder, "agent-notes");
        assert!((cfg.obsidian.tag_boost - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.obsidian.ignored_folders.len(), 2);
    }

    #[test]
    fn multimodal_config_defaults() {
        let cfg = MultimodalConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.media_dir.is_none());
        assert_eq!(cfg.max_image_size_bytes, 10 * 1024 * 1024);
        assert!(cfg.vision_model.is_none());
        assert!(cfg.transcription_model.is_none());

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!full.multimodal.enabled);
        assert!(full.multimodal.vision_model.is_none());
    }

    // ── direct default_*() function coverage ────────────────────────────

    #[test]
    fn default_functions_return_expected_values() {
        assert_eq!(default_max_image_size(), 10 * 1024 * 1024);
        assert_eq!(default_max_chunks(), 10);
        assert!(default_digest_enabled());
        assert_eq!(default_digest_max_tokens(), 512);
        assert_eq!(default_decay_half_life_days(), 7);
        assert_eq!(default_service_name(), "_ironclad._tcp");
        assert_eq!(
            default_obsidian_ignored_folders(),
            vec![".obsidian", ".trash", ".git"]
        );
        assert_eq!(default_obsidian_template_folder(), "templates");
        assert_eq!(default_obsidian_default_folder(), "ironclad");
        assert!((default_obsidian_tag_boost() - 0.2).abs() < f64::EPSILON);
        assert_eq!(default_log_level(), "info");
        assert!((default_min_decomposition_complexity() - 0.35).abs() < f64::EPSILON);
        assert!((default_min_delegation_utility_margin() - 0.15).abs() < f64::EPSILON);
        assert_eq!(default_log_max_days(), 7);
        assert_eq!(default_rate_limit_requests(), 100);
        assert_eq!(default_rate_limit_window_secs(), 60);
        assert_eq!(default_per_ip_rate_limit_requests(), 300);
        assert_eq!(default_per_actor_rate_limit_requests(), 200);
        assert_eq!(default_port(), 18789);
        assert_eq!(default_bind(), "127.0.0.1");
        assert_eq!(default_estimated_output_tokens(), 500);
        assert_eq!(default_routing_mode(), "heuristic");
        assert!((default_confidence_threshold() - 0.9).abs() < f64::EPSILON);
        assert!(default_true());
        assert_eq!(default_cb_threshold(), 3);
        assert_eq!(default_cb_window(), 60);
        assert_eq!(default_cb_cooldown(), 60);
        assert_eq!(default_cb_credit_cooldown(), 300);
        assert_eq!(default_cb_max_cooldown(), 900);
        assert!((default_working_pct() - 30.0).abs() < f64::EPSILON);
        assert!((default_episodic_pct() - 25.0).abs() < f64::EPSILON);
        assert!((default_semantic_pct() - 20.0).abs() < f64::EPSILON);
        assert!((default_procedural_pct() - 15.0).abs() < f64::EPSILON);
        assert!((default_relationship_pct() - 10.0).abs() < f64::EPSILON);
        assert!((default_hybrid_weight() - 0.5).abs() < f64::EPSILON);
        assert!((default_compression_ratio() - 0.5).abs() < f64::EPSILON);
        assert_eq!(default_cache_ttl(), 3600);
        assert!((default_semantic_threshold() - 0.95).abs() < f64::EPSILON);
        assert_eq!(default_max_entries(), 10000);
        assert!((default_per_payment_cap() - 100.0).abs() < f64::EPSILON);
        assert!((default_hourly_limit() - 500.0).abs() < f64::EPSILON);
        assert!((default_daily_limit() - 2000.0).abs() < f64::EPSILON);
        assert!((default_min_reserve() - 5.0).abs() < f64::EPSILON);
        assert!((default_inference_budget() - 50.0).abs() < f64::EPSILON);
        assert_eq!(default_yield_protocol(), "aave");
        assert_eq!(default_yield_chain(), "base");
        assert!((default_min_deposit() - 50.0).abs() < f64::EPSILON);
        assert!((default_withdrawal_threshold() - 30.0).abs() < f64::EPSILON);
        assert!(default_yield_pool_address().starts_with("0x"));
        assert!(default_yield_usdc_address().starts_with("0x"));
        assert_eq!(default_chain_id(), 8453);
        assert_eq!(default_rpc_url(), "https://mainnet.base.org");
        assert_eq!(default_a2a_max_msg_size(), 65536);
        assert_eq!(default_a2a_rate_limit(), 10);
        assert_eq!(default_a2a_session_timeout(), 3600);
        assert_eq!(default_script_timeout(), 30);
        assert_eq!(default_script_max_output(), 1_048_576);
        assert_eq!(default_thinking_threshold(), 30);
        assert_eq!(default_signal_daemon_url(), "http://127.0.0.1:8080");
        assert_eq!(default_imap_port(), 993);
        assert_eq!(default_smtp_port(), 587);
        assert_eq!(default_poll_interval(), 30);
        assert_eq!(default_poll_timeout(), 30);
        assert_eq!(default_max_context_tokens(), 128_000);
        assert!((default_soft_trim_ratio() - 0.8).abs() < f64::EPSILON);
        assert!((default_hard_clear_ratio() - 0.95).abs() < f64::EPSILON);
        assert_eq!(default_preserve_recent(), 10);
        assert_eq!(default_checkpoint_interval(), 10);
        assert_eq!(default_approval_timeout(), 300);
        assert_eq!(default_cdp_port(), 9222);
        assert_eq!(default_update_channel(), "stable");
        assert!(default_update_registry_url().starts_with("https://"));
        assert_eq!(default_os_file(), "OS.toml");
        assert_eq!(default_firmware_file(), "FIRMWARE.toml");
        assert_eq!(default_session_ttl(), 86400);
        assert_eq!(default_session_scope_mode(), "agent");
        assert_eq!(default_mcp_port(), 3001);
        assert!((default_confidence_floor() - 0.6).abs() < f64::EPSILON);
        assert_eq!(default_escalation_latency_ms(), 3000);
        assert_eq!(
            default_t2_preamble(),
            Some("Be concise and direct. Focus on accuracy.".into())
        );
    }

    #[test]
    fn default_path_functions_return_valid_paths() {
        let ws = default_workspace();
        assert!(ws.to_str().unwrap().contains("workspace"));
        let db = default_db_path();
        assert!(db.to_str().unwrap().contains("state.db"));
        let log = default_log_dir();
        assert!(log.to_str().unwrap().contains("logs"));
        let wallet = default_wallet_path();
        assert!(wallet.to_str().unwrap().contains("wallet.json"));
        let skills = default_skills_dir();
        assert!(skills.to_str().unwrap().contains("skills"));
        let plugins = default_plugins_dir();
        assert!(plugins.to_str().unwrap().contains("plugins"));
        let browser = default_browser_profile_dir();
        assert!(browser.to_str().unwrap().contains("browser-profiles"));
        let pid = default_pid_file();
        assert!(pid.to_str().unwrap().contains("ironclad.pid"));
    }

    #[test]
    fn default_interpreters_contains_bash() {
        let interp = default_interpreters();
        assert!(interp.contains(&"bash".to_string()));
    }

    // ── Default impl coverage for struct types ──────────────────────────

    #[test]
    fn server_config_default() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.port, 18789);
        assert_eq!(cfg.bind, "127.0.0.1");
        assert!(cfg.api_key.is_none());
        assert_eq!(cfg.log_max_days, 7);
        assert_eq!(cfg.rate_limit_requests, 100);
        assert_eq!(cfg.rate_limit_window_secs, 60);
        assert_eq!(cfg.per_ip_rate_limit_requests, 300);
        assert_eq!(cfg.per_actor_rate_limit_requests, 200);
        assert!(cfg.trusted_proxy_cidrs.is_empty());
    }

    #[test]
    fn database_config_default() {
        let cfg = DatabaseConfig::default();
        assert!(cfg.path.to_str().unwrap().contains("state.db"));
    }

    #[test]
    fn routing_config_default() {
        let cfg = RoutingConfig::default();
        assert_eq!(cfg.mode, "heuristic");
        assert!((cfg.confidence_threshold - 0.9).abs() < f64::EPSILON);
        assert!(cfg.local_first);
        assert!(!cfg.cost_aware);
        assert_eq!(cfg.estimated_output_tokens, 500);
    }

    #[test]
    fn tiered_inference_config_default() {
        let cfg = TieredInferenceConfig::default();
        assert!(!cfg.enabled);
        assert!((cfg.confidence_floor - 0.6).abs() < f64::EPSILON);
        assert_eq!(cfg.escalation_latency_budget_ms, 3000);
    }

    #[test]
    fn circuit_breaker_config_default() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.threshold, 3);
        assert_eq!(cfg.window_seconds, 60);
        assert_eq!(cfg.cooldown_seconds, 60);
        assert_eq!(cfg.credit_cooldown_seconds, 300);
        assert_eq!(cfg.max_cooldown_seconds, 900);
    }

    #[test]
    fn memory_config_default() {
        let cfg = MemoryConfig::default();
        assert!((cfg.working_budget_pct - 30.0).abs() < f64::EPSILON);
        assert!((cfg.hybrid_weight - 0.5).abs() < f64::EPSILON);
        assert!(!cfg.ann_index);
    }

    #[test]
    fn cache_config_default() {
        let cfg = CacheConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.exact_match_ttl_seconds, 3600);
        assert!((cfg.compression_target_ratio - 0.5).abs() < f64::EPSILON);
        assert!(!cfg.prompt_compression);
    }

    #[test]
    fn treasury_config_default() {
        let cfg = TreasuryConfig::default();
        assert!((cfg.per_payment_cap - 100.0).abs() < f64::EPSILON);
        assert!((cfg.hourly_transfer_limit - 500.0).abs() < f64::EPSILON);
        assert!((cfg.daily_transfer_limit - 2000.0).abs() < f64::EPSILON);
        assert!((cfg.minimum_reserve - 5.0).abs() < f64::EPSILON);
        assert!((cfg.daily_inference_budget - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn yield_config_default() {
        let cfg = YieldConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.protocol, "aave");
        assert_eq!(cfg.chain, "base");
        assert!(cfg.chain_rpc_url.is_none());
        assert!(cfg.atoken_address.is_none());
    }

    #[test]
    fn wallet_config_default() {
        let cfg = WalletConfig::default();
        assert_eq!(cfg.chain_id, 8453);
        assert_eq!(cfg.rpc_url, "https://mainnet.base.org");
    }

    #[test]
    fn a2a_config_default() {
        let cfg = A2aConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.require_on_chain_identity);
    }

    #[test]
    fn channels_config_default() {
        let cfg = ChannelsConfig::default();
        assert!(cfg.telegram.is_none());
        assert!(cfg.whatsapp.is_none());
        assert!(cfg.discord.is_none());
        assert!(cfg.signal.is_none());
        assert!(cfg.trusted_sender_ids.is_empty());
        assert_eq!(cfg.thinking_threshold_seconds, 30);
        assert!(cfg.startup_announcements.is_none());
    }

    #[test]
    fn email_config_default() {
        let cfg = EmailConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.imap_port, 993);
        assert_eq!(cfg.smtp_port, 587);
        assert_eq!(cfg.poll_interval_seconds, 30);
        assert!(cfg.oauth2_token_env.is_empty());
        assert!(!cfg.use_oauth2);
        assert!(cfg.imap_idle_enabled);
    }

    #[test]
    fn approvals_config_default() {
        let cfg = ApprovalsConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.gated_tools.is_empty());
        assert!(cfg.blocked_tools.is_empty());
        assert_eq!(cfg.timeout_seconds, 300);
    }

    #[test]
    fn plugins_config_default() {
        let cfg = PluginsConfig::default();
        assert!(cfg.allow.is_empty());
        assert!(cfg.deny.is_empty());
    }

    #[test]
    fn browser_config_default() {
        let cfg = BrowserConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.executable_path.is_none());
        assert!(cfg.headless);
        assert_eq!(cfg.cdp_port, 9222);
    }

    #[test]
    fn daemon_config_default() {
        let cfg = DaemonConfig::default();
        assert!(!cfg.auto_restart);
    }

    #[test]
    fn update_config_default() {
        let cfg = UpdateConfig::default();
        assert!(cfg.check_on_start);
        assert_eq!(cfg.channel, "stable");
    }

    #[test]
    fn personality_config_default() {
        let cfg = PersonalityConfig::default();
        assert_eq!(cfg.os_file, "OS.toml");
        assert_eq!(cfg.firmware_file, "FIRMWARE.toml");
    }

    #[test]
    fn mcp_config_default() {
        let cfg = McpConfig::default();
        assert!(!cfg.server_enabled);
        assert_eq!(cfg.server_port, 3001);
        assert!(cfg.clients.is_empty());
    }

    #[test]
    fn device_config_default() {
        let cfg = DeviceConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.identity_path.is_none());
        assert!(!cfg.sync_enabled);
        assert_eq!(cfg.max_paired_devices, 5);
    }

    #[test]
    fn discovery_config_default() {
        let cfg = DiscoveryConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.dns_sd);
        assert!(!cfg.mdns);
        assert!(!cfg.advertise);
        assert_eq!(cfg.service_name, "_ironclad._tcp");
    }

    #[test]
    fn tier_adapt_config_default() {
        let cfg = TierAdaptConfig::default();
        assert!(!cfg.t1_strip_system);
        assert!(!cfg.t1_condense_turns);
        assert!(cfg.t3_t4_passthrough);
    }

    // ── validate() edge cases ───────────────────────────────────────────

    #[test]
    fn validate_empty_agent_name_fails() {
        let toml = r#"
[agent]
name = ""
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("agent.name"));
    }

    #[test]
    fn validate_empty_agent_id_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = ""

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("agent.id"));
    }

    #[test]
    fn validate_empty_model_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = ""
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("models.primary"));
    }

    #[test]
    fn validate_invalid_bind_address_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999
bind = "not-an-ip"

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("not a valid IP"));
    }

    #[test]
    fn validate_localhost_bind_ok() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999
bind = "localhost"

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_invalid_session_scope_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "invalid"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("scope_mode"));
    }

    #[test]
    fn validate_group_scope_ok() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "group"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_negative_minimum_reserve_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[treasury]
minimum_reserve = -1.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("minimum_reserve"));
    }

    #[test]
    fn validate_zero_payment_cap_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[treasury]
per_payment_cap = 0.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("per_payment_cap"));
    }

    // ── startup_announcement_channels coverage ──────────────────────────

    #[test]
    fn startup_announcements_none_returns_empty() {
        let cfg = ChannelsConfig::default();
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_flag_returns_empty() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Flag(true)),
            ..ChannelsConfig::default()
        };
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_text_returns_normalized() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Text("Telegram".into())),
            ..ChannelsConfig::default()
        };
        assert_eq!(cfg.startup_announcement_channels(), vec!["telegram"]);
    }

    #[test]
    fn startup_announcements_text_none_variant() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Text("none".into())),
            ..ChannelsConfig::default()
        };
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_channels_dedup_and_sort() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Channels(vec![
                "whatsapp".into(),
                "telegram".into(),
                "TELEGRAM".into(),
                "none".into(),
            ])),
            ..ChannelsConfig::default()
        };
        let ch = cfg.startup_announcement_channels();
        assert_eq!(ch, vec!["telegram", "whatsapp"]);
    }

    // ── expand_tilde coverage ───────────────────────────────────────────

    #[test]
    fn expand_tilde_no_tilde() {
        let p = PathBuf::from("/absolute/path");
        assert_eq!(expand_tilde(&p), p);
    }

    #[test]
    fn expand_tilde_with_tilde() {
        let p = PathBuf::from("~/Documents/vault");
        let expanded = expand_tilde(&p);
        assert!(!expanded.to_str().unwrap().starts_with("~"));
        assert!(expanded.to_str().unwrap().contains("Documents/vault"));
    }

    // ── ProviderConfig::new ─────────────────────────────────────────────

    #[test]
    fn provider_config_new() {
        let pc = ProviderConfig::new("http://localhost:11434", "T1");
        assert_eq!(pc.url, "http://localhost:11434");
        assert_eq!(pc.tier, "T1");
        assert!(pc.format.is_none());
        assert!(pc.api_key_env.is_none());
        assert!(pc.is_local.is_none());
        assert!(pc.tpm_limit.is_none());
        assert!(pc.rpm_limit.is_none());
    }

    // ── MCP transport default ───────────────────────────────────────────

    #[test]
    fn mcp_transport_default_is_sse() {
        let t = McpTransport::default();
        assert!(matches!(t, McpTransport::Sse));
    }

    // ── home_dir and dirs_next helpers ───────────────────────────────────

    #[test]
    fn home_dir_returns_valid_path() {
        let h = home_dir();
        assert!(h.is_absolute() || h == std::path::Path::new("/tmp"));
    }

    #[test]
    fn dirs_next_appends_ironclad() {
        let d = dirs_next();
        assert!(d.to_str().unwrap().contains(".ironclad"));
    }

    #[test]
    fn resolve_config_path_explicit_overrides_all() {
        let p = resolve_config_path(Some("/tmp/custom.toml"));
        assert_eq!(p.unwrap(), std::path::PathBuf::from("/tmp/custom.toml"));
    }

    #[test]
    fn resolve_config_path_explicit_even_if_nonexistent() {
        // Explicit path is returned even if file doesn't exist — caller handles errors
        let p = resolve_config_path(Some("/nonexistent/path/ironclad.toml"));
        assert_eq!(
            p.unwrap(),
            std::path::PathBuf::from("/nonexistent/path/ironclad.toml")
        );
    }

    // ── KnowledgeConfig / WorkspaceConfig defaults ──────────────────────

    #[test]
    fn knowledge_config_default() {
        let cfg = KnowledgeConfig::default();
        assert!(cfg.sources.is_empty());
    }

    #[test]
    fn workspace_config_default() {
        let cfg = WorkspaceConfig::default();
        assert!(!cfg.soul_versioning);
        assert!(!cfg.index_on_start);
        assert!(!cfg.watch_for_changes);
    }

    #[test]
    fn voice_channel_config_default() {
        let cfg = VoiceChannelConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.stt_model.is_none());
        assert!(cfg.tts_model.is_none());
        assert!(cfg.tts_voice.is_none());
    }

    // ── Security validation ────────────────────────────────────────────

    #[test]
    fn validate_default_security_config_ok() {
        // Default SecurityConfig should pass validation.
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_allowlist_authority_exceeds_trusted_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[security]
allowlist_authority = "Creator"
trusted_authority = "Peer"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("allowlist_authority"));
    }

    #[test]
    fn validate_threat_ceiling_creator_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[security]
threat_caution_ceiling = "Creator"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("threat_caution_ceiling"));
    }

    #[test]
    fn validate_security_peer_ceiling_ok() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[security]
threat_caution_ceiling = "Peer"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_routing_accuracy_floor_out_of_range_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[models.routing]
accuracy_floor = 1.5
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("accuracy_floor"));
    }

    #[test]
    fn validate_routing_canary_fraction_requires_model() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[models.routing]
canary_fraction = 0.2
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("canary_model"));
    }

    #[test]
    fn validate_routing_canary_model_requires_fraction() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[models.routing]
canary_model = "openai/gpt-4o"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("canary_fraction"));
    }

    #[test]
    fn validate_routing_canary_model_must_not_be_blocked() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[models.routing]
canary_model = "openai/gpt-4o"
canary_fraction = 0.5
blocked_models = ["openai/gpt-4o"]
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("blocked_models"));
    }
}
