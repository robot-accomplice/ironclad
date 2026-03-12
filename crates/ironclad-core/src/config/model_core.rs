#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    #[serde(default = "default_port")]
    pub port: u16,
    #[serde(default = "default_bind")]
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
    "metascore".into()
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
    #[serde(default = "default_cb_max_cooldown")]
    pub max_cooldown_seconds: u64,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            threshold: default_cb_threshold(),
            window_seconds: default_cb_window(),
            cooldown_seconds: default_cb_cooldown(),
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

