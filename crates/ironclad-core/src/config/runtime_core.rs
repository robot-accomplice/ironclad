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

