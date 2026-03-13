/// Model triage: validates the full resolution chain for every configured model
/// and enabled channel — keystore presence → value non-blank → API probe.

#[derive(Debug, Clone)]
pub(super) struct ModelTriageReport {
    pub models: Vec<ModelProbeResult>,
    pub channels: Vec<ChannelProbeResult>,
}

#[derive(Debug, Clone)]
pub(super) struct ModelProbeResult {
    pub model_id: String,
    pub provider: String,
    pub role: ModelRole,
    pub key_status: KeyStatus,
    pub reachable: Option<bool>,
    pub probe_detail: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModelRole {
    Primary,
    Fallback,
    Routing,
    Embedding,
}

impl std::fmt::Display for ModelRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Primary => write!(f, "primary"),
            Self::Fallback => write!(f, "fallback"),
            Self::Routing => write!(f, "routing"),
            Self::Embedding => write!(f, "embedding"),
        }
    }
}

#[derive(Debug, Clone)]
pub(super) enum KeyStatus {
    /// Local provider — no API key required.
    NotRequired,
    /// Key found via keystore (explicit ref or conventional name).
    Keystore { key_name: String },
    /// Key found via environment variable.
    EnvVar { env_name: String },
    /// OAuth auth mode — token resolved at request time.
    OAuth,
    /// Keystore reference configured but key is missing from keystore.
    KeystoreMissing { key_name: String },
    /// Keystore key exists but has an empty/blank value.
    KeystoreBlank { key_name: String },
    /// Environment variable is not set.
    EnvMissing { env_name: String },
    /// Environment variable is set but empty/blank.
    EnvBlank { env_name: String },
    /// Provider not found in config at all.
    ProviderNotConfigured,
}

impl KeyStatus {
    pub fn is_healthy(&self) -> bool {
        matches!(
            self,
            Self::NotRequired | Self::Keystore { .. } | Self::EnvVar { .. } | Self::OAuth
        )
    }

    pub fn summary(&self) -> &'static str {
        match self {
            Self::NotRequired => "local (no key needed)",
            Self::Keystore { .. } => "keystore ✓",
            Self::EnvVar { .. } => "env var ✓",
            Self::OAuth => "OAuth ✓",
            Self::KeystoreMissing { .. } => "keystore key MISSING",
            Self::KeystoreBlank { .. } => "keystore key BLANK",
            Self::EnvMissing { .. } => "env var NOT SET",
            Self::EnvBlank { .. } => "env var EMPTY",
            Self::ProviderNotConfigured => "provider NOT CONFIGURED",
        }
    }

    pub fn severity(&self) -> &'static str {
        match self {
            Self::NotRequired | Self::Keystore { .. } | Self::EnvVar { .. } | Self::OAuth => "ok",
            Self::KeystoreMissing { .. }
            | Self::KeystoreBlank { .. }
            | Self::EnvMissing { .. }
            | Self::EnvBlank { .. } => "high",
            Self::ProviderNotConfigured => "medium",
        }
    }

    pub fn remediation(&self) -> String {
        match self {
            Self::NotRequired | Self::Keystore { .. } | Self::EnvVar { .. } | Self::OAuth => {
                String::new()
            }
            Self::KeystoreMissing { key_name } => {
                format!("ironclad keystore set {key_name} \"<YOUR_KEY>\"")
            }
            Self::KeystoreBlank { key_name } => {
                format!("ironclad keystore set {key_name} \"<YOUR_KEY>\" (current value is blank)")
            }
            Self::EnvMissing { env_name } => {
                format!("export {env_name}=\"<YOUR_KEY>\"")
            }
            Self::EnvBlank { env_name } => {
                format!("export {env_name}=\"<YOUR_KEY>\" (current value is blank)")
            }
            Self::ProviderNotConfigured => {
                "Add a [providers.<name>] section in ironclad.toml".to_string()
            }
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct ChannelProbeResult {
    pub channel: String,
    pub enabled: bool,
    pub key_status: KeyStatus,
    pub reachable: Option<bool>,
    pub probe_detail: Option<String>,
}

impl ModelTriageReport {
    pub fn total_unhealthy(&self) -> usize {
        self.models.iter().filter(|m| !m.key_status.is_healthy()).count()
            + self.channels.iter().filter(|c| c.enabled && !c.key_status.is_healthy()).count()
    }

    pub fn total_unreachable(&self) -> usize {
        self.models.iter().filter(|m| m.reachable == Some(false)).count()
            + self.channels.iter().filter(|c| c.reachable == Some(false)).count()
    }
}

/// Run the full model triage: resolve keys and probe each configured model
/// and channel. This is a synchronous function that does NOT make network
/// calls — the optional `probe` parameter controls whether we do HTTP probes.
pub(super) fn run_model_triage(
    config: &ironclad_core::IroncladConfig,
    probe: bool,
) -> ModelTriageReport {
    let keystore = ironclad_core::Keystore::new(ironclad_core::Keystore::default_path());
    let keystore_available = keystore.unlock_machine().is_ok();

    let mut models = Vec::new();

    // Collect all model IDs we need to check.
    let mut model_roles: Vec<(String, ModelRole)> = Vec::new();
    model_roles.push((config.models.primary.clone(), ModelRole::Primary));
    for fb in &config.models.fallbacks {
        model_roles.push((fb.clone(), ModelRole::Fallback));
    }
    if let Some(ref canary) = config.models.routing.canary_model {
        if !canary.trim().is_empty() {
            model_roles.push((canary.clone(), ModelRole::Routing));
        }
    }
    // Embedding provider
    if let Some((embed_provider, _)) = resolve_embedding_provider(config) {
        model_roles.push((embed_provider, ModelRole::Embedding));
    }

    // Deduplicate: if the same model appears in multiple roles, keep the
    // highest-priority role (Primary > Fallback > Routing > Embedding).
    let mut seen = std::collections::HashSet::new();
    model_roles.retain(|(model, _)| seen.insert(model.clone()));

    for (model_id, role) in &model_roles {
        let provider_name = extract_provider_name(model_id);
        let provider_cfg = config.providers.get(&provider_name);

        let key_status = match provider_cfg {
            None => KeyStatus::ProviderNotConfigured,
            Some(p) => resolve_key_status(
                &provider_name,
                p.is_local.unwrap_or(false),
                p.api_key_ref.as_deref(),
                p.api_key_env.as_deref(),
                &keystore,
                keystore_available,
            ),
        };

        let (reachable, probe_detail) = if probe && key_status.is_healthy() {
            if let Some(p) = provider_cfg {
                probe_provider(&provider_name, &p.url, p.is_local.unwrap_or(false), &key_status)
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        models.push(ModelProbeResult {
            model_id: model_id.clone(),
            provider: provider_name,
            role: *role,
            key_status,
            reachable,
            probe_detail,
        });
    }

    // Channel triage
    let mut channels = Vec::new();

    // Telegram
    if let Some(ref tg) = config.channels.telegram {
        let key_status = if tg.enabled {
            resolve_channel_key_status(
                &tg.token_ref,
                &tg.token_env,
                "telegram_bot_token",
                &keystore,
                keystore_available,
            )
        } else {
            KeyStatus::NotRequired
        };
        let (reachable, probe_detail) = if probe && tg.enabled && key_status.is_healthy() {
            probe_telegram_token(&key_status, &keystore, &tg.token_ref, &tg.token_env)
        } else {
            (None, None)
        };
        channels.push(ChannelProbeResult {
            channel: "Telegram".to_string(),
            enabled: tg.enabled,
            key_status,
            reachable,
            probe_detail,
        });
    }

    // WhatsApp
    if let Some(ref wa) = config.channels.whatsapp {
        let key_status = if wa.enabled {
            resolve_channel_key_status(
                &wa.token_ref,
                &wa.token_env,
                "whatsapp_token",
                &keystore,
                keystore_available,
            )
        } else {
            KeyStatus::NotRequired
        };
        channels.push(ChannelProbeResult {
            channel: "WhatsApp".to_string(),
            enabled: wa.enabled,
            key_status,
            reachable: None,
            probe_detail: None,
        });
    }

    // Discord
    if let Some(ref dc) = config.channels.discord {
        let key_status = if dc.enabled {
            resolve_channel_key_status(
                &dc.token_ref,
                &dc.token_env,
                "discord_bot_token",
                &keystore,
                keystore_available,
            )
        } else {
            KeyStatus::NotRequired
        };
        channels.push(ChannelProbeResult {
            channel: "Discord".to_string(),
            enabled: dc.enabled,
            key_status,
            reachable: None,
            probe_detail: None,
        });
    }

    // Signal — uses a local signal-cli daemon, no cloud token required.
    if let Some(ref sg) = config.channels.signal {
        let (reachable, probe_detail) = if probe && sg.enabled {
            probe_url_reachable(&sg.daemon_url)
        } else {
            (None, None)
        };
        channels.push(ChannelProbeResult {
            channel: "Signal".to_string(),
            enabled: sg.enabled,
            key_status: KeyStatus::NotRequired,
            reachable,
            probe_detail,
        });
    }

    ModelTriageReport { models, channels }
}

/// Extract the provider name from a model ID like "anthropic/claude-3.5-sonnet".
fn extract_provider_name(model_id: &str) -> String {
    model_id
        .split('/')
        .next()
        .unwrap_or(model_id)
        .to_string()
}

/// Resolve the embedding provider/model from config.
fn resolve_embedding_provider(
    config: &ironclad_core::IroncladConfig,
) -> Option<(String, String)> {
    // Check each provider for embedding_model config.
    for (name, p) in &config.providers {
        if p.embedding_model.is_some() && p.embedding_path.is_some() {
            let model_label = format!(
                "{}/{}",
                name,
                p.embedding_model.as_deref().unwrap_or("default")
            );
            return Some((model_label, name.clone()));
        }
    }
    None
}

/// Resolve key status for an LLM provider using the same priority cascade
/// as the runtime: local → keystore explicit → keystore conventional → env var.
fn resolve_key_status(
    provider_name: &str,
    is_local: bool,
    api_key_ref: Option<&str>,
    api_key_env: Option<&str>,
    keystore: &ironclad_core::Keystore,
    keystore_available: bool,
) -> KeyStatus {
    if is_local {
        return KeyStatus::NotRequired;
    }

    // Explicit keystore reference: "keystore:<name>"
    if let Some(ks_ref) = api_key_ref {
        if let Some(ks_name) = ks_ref.strip_prefix("keystore:") {
            if !keystore_available {
                return KeyStatus::KeystoreMissing {
                    key_name: ks_name.to_string(),
                };
            }
            return match keystore.get(ks_name) {
                Some(val) if val.trim().is_empty() => KeyStatus::KeystoreBlank {
                    key_name: ks_name.to_string(),
                },
                Some(_) => KeyStatus::Keystore {
                    key_name: ks_name.to_string(),
                },
                None => KeyStatus::KeystoreMissing {
                    key_name: ks_name.to_string(),
                },
            };
        }
    }

    // Conventional keystore name: "{provider_name}_api_key"
    let conventional = format!("{provider_name}_api_key");
    if keystore_available {
        if let Some(val) = keystore.get(&conventional) {
            if val.trim().is_empty() {
                return KeyStatus::KeystoreBlank {
                    key_name: conventional,
                };
            }
            return KeyStatus::Keystore {
                key_name: conventional,
            };
        }
    }

    // Environment variable
    if let Some(env_name) = api_key_env {
        if !env_name.is_empty() {
            return match std::env::var(env_name) {
                Ok(val) if val.trim().is_empty() => KeyStatus::EnvBlank {
                    env_name: env_name.to_string(),
                },
                Ok(_) => KeyStatus::EnvVar {
                    env_name: env_name.to_string(),
                },
                Err(_) => KeyStatus::EnvMissing {
                    env_name: env_name.to_string(),
                },
            };
        }
    }

    // Nothing configured at all
    KeyStatus::EnvMissing {
        env_name: format!("{}_API_KEY", provider_name.to_ascii_uppercase()),
    }
}

/// Resolve key status for a channel token using the same priority as
/// `resolve_token()` in lib.rs.
fn resolve_channel_key_status(
    token_ref: &Option<String>,
    token_env: &str,
    conventional_ks_name: &str,
    keystore: &ironclad_core::Keystore,
    keystore_available: bool,
) -> KeyStatus {
    // Explicit keystore reference
    if let Some(r) = token_ref {
        if let Some(ks_name) = r.strip_prefix("keystore:") {
            if !keystore_available {
                return KeyStatus::KeystoreMissing {
                    key_name: ks_name.to_string(),
                };
            }
            return match keystore.get(ks_name) {
                Some(val) if val.trim().is_empty() => KeyStatus::KeystoreBlank {
                    key_name: ks_name.to_string(),
                },
                Some(_) => KeyStatus::Keystore {
                    key_name: ks_name.to_string(),
                },
                None => KeyStatus::KeystoreMissing {
                    key_name: ks_name.to_string(),
                },
            };
        }
    }

    // Conventional keystore name for channel
    if keystore_available {
        if let Some(val) = keystore.get(conventional_ks_name) {
            if val.trim().is_empty() {
                return KeyStatus::KeystoreBlank {
                    key_name: conventional_ks_name.to_string(),
                };
            }
            return KeyStatus::Keystore {
                key_name: conventional_ks_name.to_string(),
            };
        }
    }

    // Environment variable fallback
    if !token_env.is_empty() {
        return match std::env::var(token_env) {
            Ok(val) if val.trim().is_empty() => KeyStatus::EnvBlank {
                env_name: token_env.to_string(),
            },
            Ok(_) => KeyStatus::EnvVar {
                env_name: token_env.to_string(),
            },
            Err(_) => KeyStatus::EnvMissing {
                env_name: token_env.to_string(),
            },
        };
    }

    KeyStatus::EnvMissing {
        env_name: format!("{}_TOKEN", conventional_ks_name.to_ascii_uppercase()),
    }
}

/// Lightweight HTTP probe to test if a provider's API endpoint is reachable
/// and the key is valid. Uses a minimal GET request (model list or root).
fn probe_provider(
    provider_name: &str,
    base_url: &str,
    is_local: bool,
    key_status: &KeyStatus,
) -> (Option<bool>, Option<String>) {
    let key_value = match key_status {
        KeyStatus::Keystore { key_name } => {
            let ks = ironclad_core::Keystore::new(ironclad_core::Keystore::default_path());
            let _ = ks.unlock_machine();
            ks.get(key_name)
        }
        KeyStatus::EnvVar { env_name } => std::env::var(env_name).ok(),
        KeyStatus::NotRequired => None,
        _ => return (None, None),
    };

    // For local providers, just check if the endpoint responds at all.
    if is_local {
        return probe_url_reachable(base_url);
    }

    // For remote providers, try a lightweight authenticated endpoint.
    let probe_url = match provider_name {
        "openai" | "openrouter" | "moonshot" => format!("{base_url}/v1/models"),
        "anthropic" => {
            // Anthropic has no lightweight list endpoint; use a HEAD-style
            // approach by just checking connectivity.
            return probe_url_reachable(base_url);
        }
        "google" => format!("{base_url}/v1beta/models"),
        _ => return probe_url_reachable(base_url),
    };

    let Some(key) = key_value else {
        return probe_url_reachable(base_url);
    };

    // Use a short-lived blocking HTTP client for the probe.
    let client = match ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .get(&probe_url)
        .set("Authorization", &format!("Bearer {key}"))
        .call()
    {
        Ok(resp) => {
            let status = resp.status();
            if status == 200 {
                (Some(true), Some(format!("{probe_url} → 200 OK")))
            } else {
                (
                    Some(false),
                    Some(format!("{probe_url} → {status} (key may be invalid)")),
                )
            }
        }
        Err(ureq::Error::Status(status, _)) => {
            if status == 401 || status == 403 {
                (
                    Some(false),
                    Some(format!(
                        "{probe_url} → {status} (authentication failed — key is likely invalid)"
                    )),
                )
            } else {
                (
                    Some(false),
                    Some(format!("{probe_url} → HTTP {status}")),
                )
            }
        }
        Err(e) => (
            Some(false),
            Some(format!("{probe_url} → network error: {e}")),
        ),
    };
    client
}

/// Probe Telegram's `getMe` endpoint to validate a bot token.
fn probe_telegram_token(
    key_status: &KeyStatus,
    keystore: &ironclad_core::Keystore,
    token_ref: &Option<String>,
    token_env: &str,
) -> (Option<bool>, Option<String>) {
    let token = match key_status {
        KeyStatus::Keystore { key_name } => keystore.get(key_name),
        KeyStatus::EnvVar { env_name } => std::env::var(env_name).ok(),
        _ => return (None, None),
    };
    let Some(token) = token else {
        return (None, None);
    };
    let url = format!("https://api.telegram.org/bot{token}/getMe");
    match ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .get(&url)
        .call()
    {
        Ok(resp) if resp.status() == 200 => {
            (Some(true), Some("Telegram getMe → 200 OK (token valid)".to_string()))
        }
        Ok(resp) => (
            Some(false),
            Some(format!(
                "Telegram getMe → {} (token likely invalid)",
                resp.status()
            )),
        ),
        Err(ureq::Error::Status(401, _)) => (
            Some(false),
            Some("Telegram getMe → 401 Unauthorized (token is invalid/revoked)".to_string()),
        ),
        Err(e) => (
            Some(false),
            Some(format!("Telegram getMe → network error: {e}")),
        ),
    }
}

/// Simple reachability check — can we connect to the base URL at all?
fn probe_url_reachable(base_url: &str) -> (Option<bool>, Option<String>) {
    match ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .get(base_url)
        .call()
    {
        Ok(_) => (Some(true), Some(format!("{base_url} → reachable"))),
        Err(ureq::Error::Status(status, _)) => {
            // Any HTTP response (even 404) means the server is reachable.
            (Some(true), Some(format!("{base_url} → reachable (HTTP {status})")))
        }
        Err(e) => (
            Some(false),
            Some(format!("{base_url} → unreachable: {e}")),
        ),
    }
}
