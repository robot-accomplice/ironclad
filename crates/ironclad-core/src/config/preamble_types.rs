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
    pub self_funding: SelfFundingConfig,
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
    /// messages.
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
