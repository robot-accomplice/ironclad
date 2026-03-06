use std::collections::HashMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};

use super::{AreaResult, MigrationArea, SafetyVerdict, copy_dir_recursive, scan_directory_safety};

// ── Legacy data structures ───────────────────────────────────────────

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacyConfig {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_url: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub system_prompt: Option<String>,
    #[serde(default)]
    pub channels: Option<LegacyChannels>,
    #[serde(default, deserialize_with = "deserialize_cron")]
    pub cron: Option<Vec<LegacyCronJob>>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Legacy's `cron` field can be either a list of jobs or an object like
/// `{"enabled": true}`. Accept both without failing deserialization.
fn deserialize_cron<'de, D>(deserializer: D) -> Result<Option<Vec<LegacyCronJob>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value: Option<serde_json::Value> = Option::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::Array(arr)) => {
            let jobs: Vec<LegacyCronJob> =
                serde_json::from_value(serde_json::Value::Array(arr)).unwrap_or_default();
            Ok(Some(jobs))
        }
        Some(serde_json::Value::Object(_)) => Ok(None),
        _ => Ok(None),
    }
}

#[derive(Debug, Deserialize, Serialize, Default)]
pub(crate) struct LegacyChannels {
    #[serde(default)]
    pub telegram: Option<LegacyTelegramChannel>,
    #[serde(default)]
    pub whatsapp: Option<LegacyWhatsappChannel>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacyTelegramChannel {
    #[serde(default, alias = "botToken")]
    pub token: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacyWhatsappChannel {
    #[serde(default)]
    pub token: Option<String>,
    #[serde(default, alias = "phoneNumberId")]
    pub phone_id: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacyCronJob {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub schedule: Option<serde_json::Value>,
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: HashMap<String, serde_json::Value>,
}

/// Wrapper for `~/.legacy/cron/jobs.json` which uses `{"version": N, "jobs": [...]}`
#[derive(Debug, Deserialize)]
pub(crate) struct LegacyJobsFile {
    #[serde(default)]
    pub jobs: Vec<LegacyCronJob>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacySession {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub agent_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<String>,
    #[serde(default)]
    pub messages: Option<Vec<LegacyMessage>>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct LegacyMessage {
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub timestamp: Option<String>,
}

/// Real Legacy JSONL line wrapper: `{"type":"message","message":{...}}`
#[derive(Debug, Deserialize)]
struct LegacyJSONLLine {
    #[serde(default, rename = "type")]
    line_type: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    timestamp: Option<String>,
    #[serde(default)]
    message: Option<LegacyJSONLMessage>,
}

#[derive(Debug, Deserialize)]
struct LegacyJSONLMessage {
    #[serde(default)]
    role: Option<String>,
    #[serde(default)]
    content: Option<serde_json::Value>,
    #[serde(default)]
    timestamp: Option<serde_json::Value>,
}

impl LegacyJSONLMessage {
    fn into_message(self, line_ts: Option<&str>) -> Option<LegacyMessage> {
        let role = self.role?;
        let content = match self.content? {
            serde_json::Value::String(s) => s,
            serde_json::Value::Array(arr) => arr
                .iter()
                .filter_map(|v| v.get("text").and_then(|t| t.as_str()))
                .collect::<Vec<_>>()
                .join("\n"),
            _ => return None,
        };
        if content.is_empty() {
            return None;
        }
        let ts = self
            .timestamp
            .and_then(|v| match v {
                serde_json::Value::String(s) => Some(s),
                serde_json::Value::Number(n) => {
                    Some(chrono::DateTime::from_timestamp_millis(n.as_i64()?)?.to_rfc3339())
                }
                _ => None,
            })
            .or_else(|| line_ts.map(String::from));
        Some(LegacyMessage {
            role: Some(role),
            content: Some(content),
            timestamp: ts,
        })
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 1. Config transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_config(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let ks_path = ic_root.join("keystore.enc");
    let config_path = oc_root.join("legacy.json");
    if !config_path.exists() {
        return err(
            MigrationArea::Config,
            format!("legacy.json not found at {}", config_path.display()),
        );
    }

    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to read legacy.json: {e}"),
            );
        }
    };
    let oc_cfg: LegacyConfig = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to parse legacy.json: {e}"),
            );
        }
    };

    let mut warnings = Vec::new();
    let mut toml = Vec::new();

    // Convenience accessors into the `extra` flattened fields
    let agents_defaults = oc_cfg.extra.get("agents").and_then(|a| a.get("defaults"));
    let gateway = oc_cfg.extra.get("gateway");

    // [agent]
    toml.push("[agent]".into());

    // Prefer explicit JSON name, then agent name from SOUL.md, then "Migrated Agent"
    let soul_name = {
        let soul_path = oc_root.join("workspace").join("SOUL.md");
        if soul_path.exists() {
            fs::read_to_string(&soul_path).ok().and_then(|s| {
                // Look for "I am <Name>" in the Identity section
                s.lines().find_map(|l| {
                    let trimmed = l.trim();
                    if let Some(rest) = trimmed.strip_prefix("I am ") {
                        let name = rest
                            .split([',', '.', '\u{2014}', '-'])
                            .next()
                            .unwrap_or(rest)
                            .trim();
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                    None
                })
            })
        } else {
            None
        }
    };
    let name = oc_cfg
        .name
        .as_deref()
        .or(soul_name.as_deref())
        .unwrap_or("Migrated Agent");
    let id = name.to_lowercase().replace(' ', "-");
    toml.push(format!("name = {}", qt(name)));
    toml.push(format!("id = {}", qt(&id)));
    toml.push(format!(
        "workspace = {}",
        qt(&ic_root.join("workspace").to_string_lossy())
    ));
    toml.push(String::new());

    // [server]
    toml.push("[server]".into());
    let bind = gateway
        .and_then(|g| g.get("bind"))
        .and_then(|v| v.as_str())
        .unwrap_or("loopback");
    let host = if bind == "loopback" {
        "127.0.0.1"
    } else {
        "0.0.0.0"
    };
    let port = gateway
        .and_then(|g| g.get("port"))
        .and_then(|v| v.as_u64())
        .unwrap_or(18789);
    toml.push(format!("host = {}", qt(host)));
    toml.push(format!("port = {port}"));

    // Migrate gateway auth token → keystore
    if let Some(token) = gateway
        .and_then(|g| g.get("auth"))
        .and_then(|a| a.get("token"))
        .and_then(|v| v.as_str())
        && !token.is_empty()
    {
        match store_in_keystore("gateway_auth_token", token, &ks_path) {
            Ok(()) => {
                toml.push(format!(
                    "api_key_ref = {}",
                    qt("keystore:gateway_auth_token")
                ));
                warnings
                    .push("Gateway auth token stored in keystore as \"gateway_auth_token\"".into());
            }
            Err(e) => {
                toml.push(format!("api_key_env = {}", qt("IRONCLAD_API_KEY")));
                warnings.push(format!(
                    "Keystore unavailable ({e}); set IRONCLAD_API_KEY to your gateway token"
                ));
            }
        }
    }
    toml.push(String::new());

    // [database]
    toml.push("[database]".into());
    toml.push(format!(
        "path = {}",
        qt(&ic_root.join("state.db").to_string_lossy())
    ));
    toml.push(String::new());

    // Parse the rich `models.providers` structure from legacy.json.
    let oc_providers: Vec<(String, serde_json::Value)> = oc_cfg
        .extra
        .get("models")
        .and_then(|m| m.get("providers"))
        .and_then(|p| p.as_object())
        .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();

    // [models] — prefer agents.defaults.model.{primary, fallbacks} if present
    toml.push("[models]".into());

    let agent_primary = agents_defaults
        .and_then(|d| d.get("model"))
        .and_then(|m| m.get("primary"))
        .and_then(|v| v.as_str());
    let agent_fallbacks: Vec<String> = agents_defaults
        .and_then(|d| d.get("model"))
        .and_then(|m| m.get("fallbacks"))
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    if let Some(primary) = agent_primary {
        toml.push(format!("primary = {}", qt(primary)));
        if !agent_fallbacks.is_empty() {
            let refs: Vec<String> = agent_fallbacks.iter().map(|r| qt(r)).collect();
            toml.push(format!("fallbacks = [{}]", refs.join(", ")));
        }
    } else if !oc_providers.is_empty() {
        // Fall back to first model from first provider
        let mut all_model_refs: Vec<String> = Vec::new();
        let mut primary_set = false;

        for (prov_name, prov) in &oc_providers {
            if let Some(models) = prov.get("models").and_then(|m| m.as_array()) {
                for model in models {
                    if let Some(id) = model.get("id").and_then(|v| v.as_str()) {
                        let model_ref = format!("{prov_name}/{id}");
                        if !primary_set {
                            toml.push(format!("primary = {}", qt(&model_ref)));
                            primary_set = true;
                        } else {
                            all_model_refs.push(model_ref);
                        }
                    }
                }
            }
        }

        if !primary_set {
            if let Some(model) = &oc_cfg.model {
                toml.push(format!("primary = {}", qt(model)));
            } else {
                toml.push("primary = \"gpt-4\"".into());
                warnings.push("No model specified in Legacy config, defaulting to gpt-4".into());
            }
        }

        if !all_model_refs.is_empty() {
            let refs: Vec<String> = all_model_refs.iter().map(|r| qt(r)).collect();
            toml.push(format!("fallbacks = [{}]", refs.join(", ")));
        }
    } else if let Some(model) = &oc_cfg.model {
        toml.push(format!("primary = {}", qt(model)));
    } else {
        toml.push("primary = \"gpt-4\"".into());
        warnings.push("No model specified in Legacy config, defaulting to gpt-4".into());
    }
    if let Some(temp) = oc_cfg.temperature {
        toml.push(format!("temperature = {temp}"));
    }
    if let Some(max) = oc_cfg.max_tokens {
        toml.push(format!("max_tokens = {max}"));
    }
    toml.push(String::new());

    // [providers.*] — from models.providers (rich format)
    if !oc_providers.is_empty() {
        for (prov_name, prov) in &oc_providers {
            toml.push(format!("[providers.{prov_name}]"));

            if let Some(url) = prov.get("baseUrl").and_then(|v| v.as_str()) {
                toml.push(format!("url = {}", qt(url)));
            }

            // Map Legacy API format to Ironclad format
            let oc_api = prov
                .get("api")
                .and_then(|v| v.as_str())
                .unwrap_or("openai-completions");
            let (format_str, chat_path, auth_header) = match oc_api {
                "anthropic-messages" => ("anthropic", "/v1/messages", "x-api-key"),
                "google-generative-ai" => (
                    "google",
                    "/models/gemini-2.0-flash:generateContent",
                    "query:key",
                ),
                _ => ("openai", "/v1/chat/completions", "Authorization"),
            };
            toml.push(format!("chat_path = {}", qt(chat_path)));
            toml.push(format!("format = {}", qt(format_str)));
            toml.push(format!("auth_header = {}", qt(auth_header)));

            // Determine tier from provider name and cost
            let is_local = prov_name.starts_with("ollama");
            let first_cost = prov
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first())
                .and_then(|m| m.get("cost"))
                .and_then(|c| c.get("input"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);

            let tier = if is_local {
                "T1"
            } else if first_cost <= 1.0 {
                "T2"
            } else {
                "T3"
            };
            toml.push(format!("tier = {}", qt(tier)));

            if is_local {
                toml.push("is_local = true".into());
            }

            // Costs (per-million in Legacy → per-token in Ironclad)
            let cost_in = prov
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first())
                .and_then(|m| m.get("cost"))
                .and_then(|c| c.get("input"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let cost_out = prov
                .get("models")
                .and_then(|m| m.as_array())
                .and_then(|arr| arr.first())
                .and_then(|m| m.get("cost"))
                .and_then(|c| c.get("output"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            toml.push(format!("cost_per_input_token = {}", cost_in / 1_000_000.0));
            toml.push(format!(
                "cost_per_output_token = {}",
                cost_out / 1_000_000.0
            ));

            // API key → keystore
            if let Some(api_key) = prov.get("apiKey").and_then(|v| v.as_str())
                && !api_key.is_empty()
                && api_key != "ollama"
                && api_key != "ollama-local"
                && api_key != "not-needed"
            {
                let ks_name = format!("{prov_name}_api_key");
                match store_in_keystore(&ks_name, api_key, &ks_path) {
                    Ok(()) => {
                        toml.push(format!(
                            "api_key_ref = {}",
                            qt(&format!("keystore:{ks_name}"))
                        ));
                        warnings.push(format!(
                            "{prov_name} API key stored in encrypted keystore as \"{ks_name}\""
                        ));
                    }
                    Err(e) => {
                        let env_name =
                            format!("{}_API_KEY", prov_name.to_uppercase().replace('-', "_"));
                        toml.push(format!("api_key_env = {}", qt(&env_name)));
                        warnings.push(format!(
                            "Keystore unavailable ({e}); set env var {env_name}=<your-key>"
                        ));
                    }
                }
            }

            toml.push(String::new());
        }
    } else if let Some(provider) = &oc_cfg.provider {
        // Flat fallback: single top-level provider/api_key/api_url
        let key = provider.to_lowercase();
        toml.push(format!("[providers.{key}]"));
        if let Some(url) = &oc_cfg.api_url {
            toml.push(format!("url = {}", qt(url)));
        }
        if let Some(api_key) = &oc_cfg.api_key {
            let ks_name = format!("{}_api_key", key);
            match store_in_keystore(&ks_name, api_key, &ks_path) {
                Ok(()) => {
                    toml.push(format!(
                        "api_key_ref = {}",
                        qt(&format!("keystore:{ks_name}"))
                    ));
                    warnings.push(format!(
                        "API key stored in encrypted keystore as \"{ks_name}\""
                    ));
                }
                Err(e) => {
                    let env_name = format!("{}_API_KEY", provider.to_uppercase());
                    toml.push(format!("api_key_env = {}", qt(&env_name)));
                    warnings.push(format!(
                        "Keystore unavailable ({e}); set env var {env_name}=<your-key>"
                    ));
                }
            }
        }
        toml.push(String::new());
    }

    // [channels.*] — include channel config so ironclad.toml is serve-ready
    if let Some(channels) = &oc_cfg.channels {
        if let Some(tg) = &channels.telegram {
            toml.push("[channels.telegram]".into());
            toml.push(format!("enabled = {}", tg.enabled.unwrap_or(false)));
            if let Some(token) = &tg.token {
                match store_in_keystore("telegram_bot_token", token, &ks_path) {
                    Ok(()) => {
                        toml.push("token_ref = \"keystore:telegram_bot_token\"".into());
                        warnings.push(
                            "Telegram token stored in encrypted keystore as \"telegram_bot_token\""
                                .into(),
                        );
                    }
                    Err(e) => {
                        toml.push("token_env = \"TELEGRAM_BOT_TOKEN\"".into());
                        warnings.push(format!(
                            "Keystore unavailable ({e}); set env var TELEGRAM_BOT_TOKEN=<token>"
                        ));
                    }
                }
            }
            toml.push("poll_timeout_seconds = 30".into());
            toml.push("allowed_chat_ids = []".into());
            toml.push("webhook_mode = false".into());
            toml.push(String::new());
        }
        if let Some(wa) = &channels.whatsapp {
            toml.push("[channels.whatsapp]".into());
            toml.push(format!("enabled = {}", wa.enabled.unwrap_or(false)));
            if let Some(token) = &wa.token {
                match store_in_keystore("whatsapp_token", token, &ks_path) {
                    Ok(()) => {
                        toml.push("token_ref = \"keystore:whatsapp_token\"".into());
                        warnings.push(
                            "WhatsApp token stored in encrypted keystore as \"whatsapp_token\""
                                .into(),
                        );
                    }
                    Err(e) => {
                        toml.push("token_env = \"WHATSAPP_TOKEN\"".into());
                        warnings.push(format!(
                            "Keystore unavailable ({e}); set env var WHATSAPP_TOKEN=<token>"
                        ));
                    }
                }
            }
            if let Some(phone) = &wa.phone_id {
                toml.push(format!("phone_number_id = {}", qt(phone)));
            }
            toml.push(String::new());
        }
    }

    if let Err(e) = fs::create_dir_all(ic_root) {
        return err(
            MigrationArea::Config,
            format!("Failed to create output dir: {e}"),
        );
    }
    if let Err(e) = fs::write(ic_root.join("ironclad.toml"), toml.join("\n")) {
        return err(
            MigrationArea::Config,
            format!("Failed to write ironclad.toml: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Config,
        success: true,
        items_processed: 1,
        warnings,
        error: None,
    }
}

pub(crate) fn export_config(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let ks_path = ic_root.join("keystore.enc");
    let config_path = ic_root.join("ironclad.toml");
    if !config_path.exists() {
        return err(MigrationArea::Config, "ironclad.toml not found".into());
    }
    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to read ironclad.toml: {e}"),
            );
        }
    };
    let tv: toml::Value = match toml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to parse ironclad.toml: {e}"),
            );
        }
    };

    let mut oc = serde_json::Map::new();
    let mut warnings = Vec::new();

    if let Some(name) = tv
        .get("agent")
        .and_then(|a| a.get("name"))
        .and_then(|v| v.as_str())
    {
        oc.insert("name".into(), serde_json::Value::String(name.into()));
    }
    if let Some(models) = tv.get("models").and_then(|v| v.as_table()) {
        if let Some(p) = models.get("primary").and_then(|v| v.as_str()) {
            oc.insert("model".into(), serde_json::Value::String(p.into()));
        }
        if let Some(t) = models.get("temperature").and_then(|v| v.as_float()) {
            oc.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(m) = models.get("max_tokens").and_then(|v| v.as_integer()) {
            oc.insert("max_tokens".into(), serde_json::json!(m));
        }
    }
    if let Some(providers) = tv.get("providers").and_then(|v| v.as_table())
        && let Some((name, prov)) = providers.iter().next()
    {
        oc.insert("provider".into(), serde_json::Value::String(name.clone()));
        if let Some(url) = prov.get("base_url").and_then(|v| v.as_str()) {
            oc.insert("api_url".into(), serde_json::Value::String(url.into()));
        }
        let mut key_resolved = false;
        if let Some(key_ref) = prov.get("api_key_ref").and_then(|v| v.as_str())
            && let Some(ks_name) = key_ref.strip_prefix("keystore:")
        {
            if let Some(val) = read_from_keystore(ks_name, &ks_path) {
                oc.insert("api_key".into(), serde_json::Value::String(val));
                key_resolved = true;
            } else {
                warnings.push(format!(
                    "Keystore key \"{ks_name}\" not found; api_key omitted"
                ));
            }
        }
        if !key_resolved && let Some(key_env) = prov.get("api_key_env").and_then(|v| v.as_str()) {
            if let Ok(val) = std::env::var(key_env) {
                oc.insert("api_key".into(), serde_json::Value::String(val));
            } else {
                warnings.push(format!("Env var {key_env} not set; api_key omitted"));
            }
        }
    }

    // Deep-merge with existing legacy.json if present
    let oc_config_path = oc_root.join("legacy.json");
    let mut merged: serde_json::Map<String, serde_json::Value> = if oc_config_path.exists() {
        fs::read_to_string(&oc_config_path)
            .ok()
            .and_then(|c| serde_json::from_str(&c).ok())
            .unwrap_or_default()
    } else {
        serde_json::Map::new()
    };
    for (k, v) in oc {
        merged.insert(k, v);
    }

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Config,
            format!("Failed to create output dir: {e}"),
        );
    }
    let json = match serde_json::to_string_pretty(&merged) {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Config,
                format!("Failed to serialize config: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(&oc_config_path, &json) {
        return err(
            MigrationArea::Config,
            format!("Failed to write legacy.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Config,
        success: true,
        items_processed: 1,
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 2. Personality transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_personality(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let ws = oc_root.join("workspace");
    let soul_path = ws.join("SOUL.md");
    let agents_path = ws.join("AGENTS.md");
    let out_dir = ic_root.join("workspace");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Personality,
            format!("Failed to create workspace dir: {e}"),
        );
    }

    let mut warnings = Vec::new();
    let mut items = 0;

    if soul_path.exists() {
        match fs::read_to_string(&soul_path) {
            Ok(md) => {
                let toml_str = markdown_to_personality_toml(&md, "os");
                if let Err(e) = fs::write(out_dir.join("OS.toml"), &toml_str) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write OS.toml: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read SOUL.md: {e}"),
                );
            }
        }
    } else {
        warnings.push("SOUL.md not found; OS.toml will use defaults".into());
    }

    if agents_path.exists() {
        match fs::read_to_string(&agents_path) {
            Ok(md) => {
                let toml_str = markdown_to_personality_toml(&md, "firmware");
                if let Err(e) = fs::write(out_dir.join("FIRMWARE.toml"), &toml_str) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write FIRMWARE.toml: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read AGENTS.md: {e}"),
                );
            }
        }
    } else {
        warnings.push("AGENTS.md not found; FIRMWARE.toml will use defaults".into());
    }

    AreaResult {
        area: MigrationArea::Personality,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_personality(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let ic_ws = ic_root.join("workspace");
    let out_ws = oc_root.join("workspace");
    if let Err(e) = fs::create_dir_all(&out_ws) {
        return err(
            MigrationArea::Personality,
            format!("Failed to create workspace dir: {e}"),
        );
    }

    let mut warnings = Vec::new();
    let mut items = 0;

    let os_path = ic_ws.join("OS.toml");
    if os_path.exists() {
        match fs::read_to_string(&os_path) {
            Ok(content) => {
                let md = personality_toml_to_markdown(&content, "SOUL");
                if let Err(e) = fs::write(out_ws.join("SOUL.md"), &md) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write SOUL.md: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read OS.toml: {e}"),
                );
            }
        }
    } else {
        warnings.push("OS.toml not found; SOUL.md will be minimal".into());
    }

    let fw_path = ic_ws.join("FIRMWARE.toml");
    if fw_path.exists() {
        match fs::read_to_string(&fw_path) {
            Ok(content) => {
                let md = personality_toml_to_markdown(&content, "AGENTS");
                if let Err(e) = fs::write(out_ws.join("AGENTS.md"), &md) {
                    return err(
                        MigrationArea::Personality,
                        format!("Failed to write AGENTS.md: {e}"),
                    );
                }
                items += 1;
            }
            Err(e) => {
                return err(
                    MigrationArea::Personality,
                    format!("Failed to read FIRMWARE.toml: {e}"),
                );
            }
        }
    } else {
        warnings.push("FIRMWARE.toml not found; AGENTS.md will be minimal".into());
    }

    AreaResult {
        area: MigrationArea::Personality,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn markdown_to_personality_toml(md: &str, kind: &str) -> String {
    let mut lines = vec![
        format!("# Converted from Legacy {kind} markdown"),
        format!("[{kind}]"),
    ];
    // Preserve full original as prompt_text for round-trip fidelity
    lines.push(format!("prompt_text = {}", qt_ml(md)));

    let mut current_section = String::new();
    let mut section_content = Vec::new();

    for line in md.lines() {
        if line.starts_with("# ") || line.starts_with("## ") {
            if !current_section.is_empty() && !section_content.is_empty() {
                let key = current_section.to_lowercase().replace([' ', '-'], "_");
                let val = section_content.join("\n");
                lines.push(format!("{key} = {}", qt_ml(&val)));
                section_content.clear();
            }
            current_section = line.trim_start_matches('#').trim().to_string();
        } else if !line.trim().is_empty() {
            section_content.push(line.to_string());
        }
    }

    if !current_section.is_empty() && !section_content.is_empty() {
        let key = current_section.to_lowercase().replace([' ', '-'], "_");
        let val = section_content.join("\n");
        lines.push(format!("{key} = {}", qt_ml(&val)));
    }

    lines.join("\n") + "\n"
}

pub(crate) fn personality_toml_to_markdown(toml_str: &str, title: &str) -> String {
    let parsed: Result<toml::Value, _> = toml::from_str(toml_str);
    match parsed {
        Ok(toml::Value::Table(table)) => {
            // Check for prompt_text (round-trip fidelity)
            for (_section_key, section_val) in &table {
                if let toml::Value::Table(inner) = section_val
                    && let Some(pt) = inner.get("prompt_text").and_then(|v| v.as_str())
                    && !pt.is_empty()
                {
                    return pt.to_string();
                }
                if let Some(pt) = section_val.as_str()
                    && _section_key == "prompt_text"
                    && !pt.is_empty()
                {
                    return pt.to_string();
                }
            }

            // Generate from structured fields
            let mut lines = vec![format!("# {title}"), String::new()];
            for (_section_key, section_val) in &table {
                if let toml::Value::Table(inner) = section_val {
                    for (key, val) in inner {
                        if key == "prompt_text" {
                            continue;
                        }
                        let heading = titlecase(key);
                        lines.push(format!("## {heading}"));
                        lines.push(String::new());
                        if let Some(s) = val.as_str() {
                            lines.push(s.to_string());
                        } else {
                            lines.push(val.to_string());
                        }
                        lines.push(String::new());
                    }
                }
            }
            lines.join("\n") + "\n"
        }
        _ => format!("# {title}\n\n{toml_str}\n"),
    }
}

pub(crate) fn titlecase(key: &str) -> String {
    key.replace('_', " ")
        .split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

// ═══════════════════════════════════════════════════════════════════════
// 3. Skills transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_skills(oc_root: &Path, ic_root: &Path, no_safety_check: bool) -> AreaResult {
    let skills_dir = oc_root.join("workspace").join("skills");
    if !skills_dir.exists() {
        return AreaResult {
            area: MigrationArea::Skills,
            success: true,
            items_processed: 0,
            warnings: vec!["No skills directory found in Legacy workspace".into()],
            error: None,
        };
    }

    let out_dir = ic_root.join("skills");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Skills,
            format!("Failed to create skills dir: {e}"),
        );
    }

    let mut warnings = Vec::new();

    if !no_safety_check {
        let report = scan_directory_safety(&skills_dir);

        if let SafetyVerdict::Critical(n) = report.verdict {
            return AreaResult {
                area: MigrationArea::Skills,
                success: false,
                items_processed: 0,
                warnings: vec![format!("{n} critical safety finding(s); import blocked")],
                error: Some(
                    "Skills blocked by safety check. Use --no-safety-check to override.".into(),
                ),
            };
        }
        if let SafetyVerdict::Warnings(n) = report.verdict {
            warnings.push(format!(
                "{n} warning(s) found in skill scripts; review recommended"
            ));
        }
    } else {
        warnings.push("Safety checks skipped (--no-safety-check)".into());
    }

    // Read skills.entries from legacy.json for enabled/disabled state
    let skill_entries: HashMap<String, bool> = fs::read_to_string(oc_root.join("legacy.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| {
            v.get("skills")?.get("entries")?.as_object().map(|obj| {
                obj.iter()
                    .map(|(k, v)| {
                        let enabled = v.get("enabled").and_then(|e| e.as_bool()).unwrap_or(true);
                        (k.clone(), enabled)
                    })
                    .collect()
            })
        })
        .unwrap_or_default();

    let mut items = 0;
    let mut skill_names: Vec<(String, bool)> = Vec::new();

    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let src = entry.path();
            let dest = out_dir.join(entry.file_name());
            let name = entry.file_name().to_string_lossy().to_string();

            if name.starts_with('.') || name == "gateway.log" {
                continue;
            }

            if src.is_file() {
                if let Err(e) = fs::copy(&src, &dest) {
                    warnings.push(format!("Failed to copy {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            } else if src.is_dir() {
                if let Err(e) = copy_dir_recursive(&src, &dest) {
                    warnings.push(format!("Failed to copy dir {}: {e}", src.display()));
                } else {
                    // Skills not listed in entries are implicitly enabled
                    let enabled = skill_entries.get(&name).copied().unwrap_or(true);
                    skill_names.push((name, enabled));
                    items += 1;
                }
            }
        }
    }

    // Register skills in the Ironclad database
    let db_path = ic_root.join("state.db");
    if db_path.exists() && !skill_names.is_empty() {
        match ironclad_db::Database::new(db_path.to_string_lossy().as_ref()) {
            Ok(db) => {
                let mut registered = 0u32;
                let mut disabled_count = 0u32;

                for (name, enabled) in &skill_names {
                    // Parse SKILL.md frontmatter for description if available
                    let skill_md = out_dir.join(name).join("SKILL.md");
                    let description = fs::read_to_string(&skill_md)
                        .ok()
                        .and_then(|content| parse_skill_description(&content));

                    let source_path = out_dir.join(name).to_string_lossy().to_string();
                    let content_hash = format!("migrated-{}", chrono::Utc::now().timestamp());

                    // Determine kind from directory contents
                    let kind = if out_dir.join(name).join("SKILL.md").exists() {
                        "instruction"
                    } else {
                        "scripted"
                    };

                    match ironclad_db::skills::register_skill(
                        &db,
                        name,
                        kind,
                        description.as_deref(),
                        &source_path,
                        &content_hash,
                        None,
                        None,
                        None,
                        None,
                        None,
                    ) {
                        Ok(id) => {
                            registered += 1;
                            if !enabled {
                                let conn = db.conn();
                                if let Err(e) = conn.execute(
                                    "UPDATE skills SET enabled = 0 WHERE id = ?1",
                                    rusqlite::params![id],
                                ) {
                                    warnings.push(format!("Failed to disable skill {name}: {e}"));
                                }
                                disabled_count += 1;
                            }
                        }
                        Err(e) => {
                            warnings.push(format!("Failed to register skill {name}: {e}"));
                        }
                    }
                }

                if registered > 0 {
                    warnings.push(format!(
                        "{registered} skill(s) registered in database ({} enabled, {disabled_count} disabled)",
                        registered - disabled_count
                    ));
                }
            }
            Err(e) => {
                warnings.push(format!(
                    "Could not open database to register skills: {e}. \
                     Run `ironclad skills reload` after migration to register them."
                ));
            }
        }
    } else if !skill_names.is_empty() {
        warnings.push(
            "Database not found; skills copied but not registered. \
             Run `ironclad skills reload` after starting the server."
                .into(),
        );
    }

    AreaResult {
        area: MigrationArea::Skills,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

/// Parse the `description` field from SKILL.md YAML frontmatter.
fn parse_skill_description(content: &str) -> Option<String> {
    let content = content.trim();
    if !content.starts_with("---") {
        return None;
    }
    let rest = &content[3..];
    let end = rest.find("---")?;
    let frontmatter = &rest[..end];
    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some(desc) = line.strip_prefix("description:") {
            let desc = desc.trim().trim_matches('"').trim_matches('\'');
            if !desc.is_empty() {
                return Some(desc.to_string());
            }
        }
    }
    None
}

pub(crate) fn export_skills(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let skills_dir = ic_root.join("skills");
    if !skills_dir.exists() {
        return AreaResult {
            area: MigrationArea::Skills,
            success: true,
            items_processed: 0,
            warnings: vec!["No skills directory found in Ironclad workspace".into()],
            error: None,
        };
    }

    let out_dir = oc_root.join("workspace").join("skills");
    if let Err(e) = fs::create_dir_all(&out_dir) {
        return err(
            MigrationArea::Skills,
            format!("Failed to create output skills dir: {e}"),
        );
    }

    let mut items = 0;
    let mut warnings = Vec::new();
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let src = entry.path();
            let dest = out_dir.join(entry.file_name());
            if src.is_file() {
                if let Err(e) = fs::copy(&src, &dest) {
                    warnings.push(format!("Failed to copy {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            } else if src.is_dir() {
                if let Err(e) = copy_dir_recursive(&src, &dest) {
                    warnings.push(format!("Failed to copy dir {}: {e}", src.display()));
                } else {
                    items += 1;
                }
            }
        }
    }

    AreaResult {
        area: MigrationArea::Skills,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 4. Sessions transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_sessions(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let mut all_sessions: Vec<LegacySession> = Vec::new();
    let mut warnings = Vec::new();

    // sessions.json (top-level array)
    let sessions_json = oc_root.join("sessions.json");
    if sessions_json.exists() {
        match fs::read_to_string(&sessions_json) {
            Ok(c) => match serde_json::from_str::<Vec<LegacySession>>(&c) {
                Ok(s) => all_sessions.extend(s),
                Err(e) => warnings.push(format!("Failed to parse sessions.json: {e}")),
            },
            Err(e) => warnings.push(format!("Failed to read sessions.json: {e}")),
        }
    }

    // agents/<agent>/sessions/*.jsonl
    let agents_dir = oc_root.join("agents");
    if agents_dir.exists()
        && let Ok(agents) = fs::read_dir(&agents_dir)
    {
        for agent_entry in agents.flatten() {
            let sess_dir = agent_entry.path().join("sessions");
            if !sess_dir.exists() {
                continue;
            }
            if let Ok(files) = fs::read_dir(&sess_dir) {
                for file in files.flatten() {
                    let path = file.path();
                    match path.extension().and_then(|e| e.to_str()) {
                        Some("jsonl") => {
                            if let Ok(content) = fs::read_to_string(&path) {
                                let mut created_at: Option<String> = None;
                                let mut msgs: Vec<LegacyMessage> = Vec::new();
                                for line in content.lines() {
                                    // Try new wrapper format first (must have an explicit "type" field)
                                    if let Ok(wrapper) =
                                        serde_json::from_str::<LegacyJSONLLine>(line)
                                        && let Some(lt) = wrapper.line_type.as_deref()
                                    {
                                        if lt == "session" {
                                            created_at = wrapper.timestamp.clone();
                                        }
                                        if lt == "message"
                                            && let Some(msg) = wrapper.message.and_then(|m| {
                                                m.into_message(wrapper.timestamp.as_deref())
                                            })
                                        {
                                            msgs.push(msg);
                                        }
                                        continue;
                                    }
                                    // Fallback: simple `{"role":"...","content":"..."}` format
                                    if let Ok(msg) = serde_json::from_str::<LegacyMessage>(line) {
                                        msgs.push(msg);
                                    }
                                }
                                if !msgs.is_empty() {
                                    all_sessions.push(LegacySession {
                                        id: Some(
                                            path.file_stem()
                                                .unwrap_or_default()
                                                .to_string_lossy()
                                                .into(),
                                        ),
                                        agent_id: Some(
                                            agent_entry.file_name().to_string_lossy().into(),
                                        ),
                                        created_at,
                                        messages: Some(msgs),
                                    });
                                }
                            }
                        }
                        Some("json") => {
                            if let Ok(content) = fs::read_to_string(&path)
                                && let Ok(s) = serde_json::from_str::<LegacySession>(&content)
                            {
                                all_sessions.push(s);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    if all_sessions.is_empty() && !sessions_json.exists() {
        return AreaResult {
            area: MigrationArea::Sessions,
            success: true,
            items_processed: 0,
            warnings: vec!["No sessions found to import".into()],
            error: None,
        };
    }

    let db_path = ic_root.join("state.db");
    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let conn = db.conn();
    let mut items = 0;
    for session in &all_sessions {
        let default_id = uuid_v4();
        let sid = session.id.as_deref().unwrap_or(&default_id);
        let agent = session.agent_id.as_deref().unwrap_or("default");
        let default_ts = now_iso();
        let created = session.created_at.as_deref().unwrap_or(&default_ts);

        if let Err(e) = conn.execute(
            "INSERT OR IGNORE INTO sessions (id, agent_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params![sid, agent, created],
        ) {
            warnings.push(format!("Failed to insert session {sid}: {e}"));
            continue;
        }

        if let Some(msgs) = &session.messages {
            for msg in msgs {
                let mid = uuid_v4();
                let role = msg.role.as_deref().unwrap_or("user");
                let content = msg.content.as_deref().unwrap_or("");
                let ts = msg.timestamp.as_deref().unwrap_or(created);
                if let Err(e) = conn.execute(
                    "INSERT OR IGNORE INTO session_messages (id, session_id, role, content, created_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![mid, sid, role, content, ts],
                ) {
                    warnings.push(format!("Failed to import message for session {sid}: {e}"));
                }
            }
        }
        items += 1;
    }

    AreaResult {
        area: MigrationArea::Sessions,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_sessions(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let db_path = ic_root.join("state.db");
    if !db_path.exists() {
        return AreaResult {
            area: MigrationArea::Sessions,
            success: true,
            items_processed: 0,
            warnings: vec!["No database found".into()],
            error: None,
        };
    }

    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let conn = db.conn();
    let mut warnings = Vec::new();
    let mut all: Vec<serde_json::Value> = Vec::new();

    let mut stmt =
        match conn.prepare("SELECT id, agent_id, created_at FROM sessions ORDER BY created_at") {
            Ok(s) => s,
            Err(e) => {
                return err(
                    MigrationArea::Sessions,
                    format!("Failed to query sessions: {e}"),
                );
            }
        };
    let sessions: Vec<(String, String, String)> = match stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    }) {
        Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to iterate sessions: {e}"),
            );
        }
    };

    for (sid, agent_id, created_at) in &sessions {
        let mut msg_stmt = match conn.prepare(
            "SELECT role, content, created_at FROM session_messages WHERE session_id = ?1 ORDER BY created_at"
        ) {
            Ok(s) => s,
            Err(e) => { warnings.push(format!("Failed to query msgs for {sid}: {e}")); continue; }
        };
        let messages: Vec<serde_json::Value> = msg_stmt
            .query_map(rusqlite::params![sid], |row| {
                Ok(serde_json::json!({
                    "role": row.get::<_, String>(0)?,
                    "content": row.get::<_, String>(1)?,
                    "timestamp": row.get::<_, String>(2)?,
                }))
            })
            .map(|iter| iter.filter_map(|r| r.ok()).collect())
            .unwrap_or_default();

        all.push(serde_json::json!({
            "id": sid, "agent_id": agent_id, "created_at": created_at, "messages": messages,
        }));
    }

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Sessions,
            format!("Failed to create output dir: {e}"),
        );
    }
    let sessions_json = match serde_json::to_string_pretty(&all) {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Sessions,
                format!("Failed to serialize sessions.json: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(oc_root.join("sessions.json"), &sessions_json) {
        return err(
            MigrationArea::Sessions,
            format!("Failed to write sessions.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Sessions,
        success: true,
        items_processed: all.len(),
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 5. Cron transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_cron(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let mut jobs = Vec::new();
    let mut warnings = Vec::new();

    // Try both `jobs.json` (flat) and `cron/jobs.json` (subdirectory)
    for candidate in [oc_root.join("jobs.json"), oc_root.join("cron/jobs.json")] {
        if !candidate.exists() {
            continue;
        }
        match fs::read_to_string(&candidate) {
            Ok(c) => {
                // First try the wrapped format `{"version": N, "jobs": [...]}`
                if let Ok(wrapper) = serde_json::from_str::<LegacyJobsFile>(&c) {
                    jobs.extend(wrapper.jobs);
                } else if let Ok(parsed) = serde_json::from_str::<Vec<LegacyCronJob>>(&c) {
                    jobs.extend(parsed);
                } else {
                    warnings.push(format!("Failed to parse {}", candidate.display()));
                }
            }
            Err(e) => warnings.push(format!("Failed to read {}: {e}", candidate.display())),
        }
    }

    let config_path = oc_root.join("legacy.json");
    if config_path.exists()
        && let Ok(c) = fs::read_to_string(&config_path)
        && let Ok(cfg) = serde_json::from_str::<LegacyConfig>(&c)
        && let Some(cj) = cfg.cron
    {
        jobs.extend(cj);
    }

    if jobs.is_empty() {
        return AreaResult {
            area: MigrationArea::Cron,
            success: true,
            items_processed: 0,
            warnings: vec!["No cron jobs found to import".into()],
            error: None,
        };
    }

    let db_path = ic_root.join("state.db");
    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => return err(MigrationArea::Cron, format!("Failed to open database: {e}")),
    };

    let conn = db.conn();
    let mut items = 0;
    let mut seen_names = std::collections::HashSet::new();

    for job in &jobs {
        let name = job.name.as_deref().unwrap_or("unnamed");

        // Skip duplicates within the same import (keep the first/most recent)
        if !seen_names.insert(name.to_string()) {
            warnings.push(format!("Skipping duplicate cron job: {name}"));
            continue;
        }

        let id = job.id.clone().unwrap_or_else(uuid_v4);
        let (schedule_kind, schedule_expr) = match &job.schedule {
            Some(serde_json::Value::String(s)) => ("cron".to_string(), s.clone()),
            Some(serde_json::Value::Object(m)) => {
                let kind = m
                    .get("kind")
                    .and_then(|v| v.as_str())
                    .unwrap_or("cron")
                    .to_string();
                let expr = if kind == "cron" {
                    m.get("expr")
                        .and_then(|v| v.as_str())
                        .unwrap_or("0 * * * *")
                        .to_string()
                } else {
                    m.get("everyMs")
                        .and_then(|v| v.as_u64())
                        .map(|ms| format!("every {}s", ms / 1000))
                        .unwrap_or_else(|| "0 * * * *".into())
                };
                (kind, expr)
            }
            _ => ("cron".to_string(), "0 * * * *".to_string()),
        };
        let enabled = job.enabled.unwrap_or(true);
        let payload = job
            .payload
            .as_ref()
            .map(|p| p.to_string())
            .or_else(|| {
                job.command
                    .as_ref()
                    .map(|c| serde_json::json!({"command": c}).to_string())
            })
            .unwrap_or_else(|| "{}".to_string());

        // Delete any existing job with the same name to avoid duplicates on re-import
        if let Err(e) = conn.execute(
            "DELETE FROM cron_jobs WHERE name = ?1",
            rusqlite::params![name],
        ) {
            warnings.push(format!("Failed to clean existing cron job {name}: {e}"));
        }

        match conn.execute(
            "INSERT INTO cron_jobs (id, name, enabled, schedule_kind, schedule_expr, agent_id, payload_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![id, name, enabled, schedule_kind, schedule_expr, "default", payload],
        ) {
            Ok(_) => items += 1,
            Err(e) => warnings.push(format!("Failed to insert cron job '{name}': {e}")),
        }
    }

    AreaResult {
        area: MigrationArea::Cron,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_cron(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let db_path = ic_root.join("state.db");
    if !db_path.exists() {
        return AreaResult {
            area: MigrationArea::Cron,
            success: true,
            items_processed: 0,
            warnings: vec!["No database found".into()],
            error: None,
        };
    }

    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => return err(MigrationArea::Cron, format!("Failed to open database: {e}")),
    };

    let conn = db.conn();
    let mut stmt = match conn
        .prepare("SELECT name, schedule_expr, payload_json, enabled FROM cron_jobs ORDER BY name")
    {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Cron,
                format!("Failed to query cron jobs: {e}"),
            );
        }
    };

    let jobs: Vec<serde_json::Value> = stmt
        .query_map([], |row| {
            let payload_str: String = row.get(2)?;
            let payload: serde_json::Value = serde_json::from_str(&payload_str).unwrap_or_default();
            let command = payload
                .get("command")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            Ok(serde_json::json!({
                "name": row.get::<_, String>(0)?,
                "schedule": row.get::<_, Option<String>>(1)?.unwrap_or_default(),
                "command": command,
                "enabled": row.get::<_, bool>(3)?,
            }))
        })
        .map(|iter| iter.filter_map(|r| r.ok()).collect())
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "failed to iterate cron job rows during export");
            vec![]
        });

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Cron,
            format!("Failed to create output dir: {e}"),
        );
    }
    let jobs_json = match serde_json::to_string_pretty(&jobs) {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Cron,
                format!("Failed to serialize jobs.json: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(oc_root.join("jobs.json"), &jobs_json) {
        return err(
            MigrationArea::Cron,
            format!("Failed to write jobs.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Cron,
        success: true,
        items_processed: jobs.len(),
        warnings: vec![],
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 6. Channels transformer
// ═══════════════════════════════════════════════════════════════════════

pub(crate) fn import_channels(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let ks_path = ic_root.join("keystore.enc");
    let config_path = oc_root.join("legacy.json");
    if !config_path.exists() {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No legacy.json found; skipping channel import".into()],
            error: None,
        };
    }
    let content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Channels,
                format!("Failed to read legacy.json: {e}"),
            );
        }
    };
    let oc_cfg: LegacyConfig = match serde_json::from_str(&content) {
        Ok(c) => c,
        Err(e) => {
            return err(
                MigrationArea::Channels,
                format!("Failed to parse legacy.json: {e}"),
            );
        }
    };

    let mut items = 0;
    let mut warnings = Vec::new();
    let mut lines = vec!["[channels]".to_string()];

    if let Some(channels) = &oc_cfg.channels {
        if let Some(tg) = &channels.telegram {
            lines.push(String::new());
            lines.push("[channels.telegram]".into());
            lines.push(format!("enabled = {}", tg.enabled.unwrap_or(false)));
            if let Some(token) = &tg.token {
                match store_in_keystore("telegram_bot_token", token, &ks_path) {
                    Ok(()) => {
                        lines.push("token_ref = \"keystore:telegram_bot_token\"".into());
                        warnings.push(
                            "Telegram token stored in encrypted keystore as \"telegram_bot_token\""
                                .into(),
                        );
                    }
                    Err(e) => {
                        lines.push("token_env = \"TELEGRAM_BOT_TOKEN\"".into());
                        warnings.push(format!(
                            "Keystore unavailable ({e}); set env var TELEGRAM_BOT_TOKEN=<token>"
                        ));
                    }
                }
            }
            items += 1;
        }
        if let Some(wa) = &channels.whatsapp {
            lines.push(String::new());
            lines.push("[channels.whatsapp]".into());
            lines.push(format!("enabled = {}", wa.enabled.unwrap_or(false)));
            if let Some(token) = &wa.token {
                match store_in_keystore("whatsapp_token", token, &ks_path) {
                    Ok(()) => {
                        lines.push("token_ref = \"keystore:whatsapp_token\"".into());
                        warnings.push(
                            "WhatsApp token stored in encrypted keystore as \"whatsapp_token\""
                                .into(),
                        );
                    }
                    Err(e) => {
                        lines.push("token_env = \"WHATSAPP_TOKEN\"".into());
                        warnings.push(format!(
                            "Keystore unavailable ({e}); set env var WHATSAPP_TOKEN=<token>"
                        ));
                    }
                }
            }
            if let Some(phone) = &wa.phone_id {
                lines.push(format!("phone_id = {}", qt(phone)));
            }
            items += 1;
        }
    }

    if items == 0 {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel configuration found in Legacy config".into()],
            error: None,
        };
    }

    if let Err(e) = fs::create_dir_all(ic_root) {
        return err(
            MigrationArea::Channels,
            format!("Failed to create dir: {e}"),
        );
    }
    if let Err(e) = fs::write(ic_root.join("channels.toml"), lines.join("\n") + "\n") {
        return err(
            MigrationArea::Channels,
            format!("Failed to write channels.toml: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Channels,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_channels(ic_root: &Path, oc_root: &Path) -> AreaResult {
    let ks_path = ic_root.join("keystore.enc");
    let channels_path = ic_root.join("channels.toml");
    let config_path = ic_root.join("ironclad.toml");
    let mut warnings = Vec::new();

    let channel_toml = if channels_path.exists() {
        match fs::read_to_string(&channels_path) {
            Ok(c) => c,
            Err(e) => {
                return err(
                    MigrationArea::Channels,
                    format!("Failed to read channels.toml: {e}"),
                );
            }
        }
    } else if config_path.exists() {
        match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(e) => {
                return err(
                    MigrationArea::Channels,
                    format!("Failed to read ironclad.toml: {e}"),
                );
            }
        }
    } else {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel configuration found".into()],
            error: None,
        };
    };

    let parsed: toml::Value = match toml::from_str(&channel_toml) {
        Ok(v) => v,
        Err(_) => {
            return AreaResult {
                area: MigrationArea::Channels,
                success: true,
                items_processed: 0,
                warnings: vec!["Could not parse channel config".into()],
                error: None,
            };
        }
    };

    let mut oc_channels = serde_json::Map::new();
    let mut items = 0;

    if let Some(channels) = parsed.get("channels").and_then(|v| v.as_table()) {
        if let Some(tg) = channels.get("telegram").and_then(|v| v.as_table()) {
            let mut obj = serde_json::Map::new();
            if let Some(e) = tg.get("enabled").and_then(|v| v.as_bool()) {
                obj.insert("enabled".into(), serde_json::Value::Bool(e));
            }
            if !resolve_channel_token_for_export(tg, &mut obj, &mut warnings, "telegram", &ks_path)
            {
                // no token resolved
            }
            oc_channels.insert("telegram".into(), serde_json::Value::Object(obj));
            items += 1;
        }
        if let Some(wa) = channels.get("whatsapp").and_then(|v| v.as_table()) {
            let mut obj = serde_json::Map::new();
            if let Some(e) = wa.get("enabled").and_then(|v| v.as_bool()) {
                obj.insert("enabled".into(), serde_json::Value::Bool(e));
            }
            if !resolve_channel_token_for_export(wa, &mut obj, &mut warnings, "whatsapp", &ks_path)
            {
                // no token resolved
            }
            if let Some(phone) = wa.get("phone_id").and_then(|v| v.as_str()) {
                obj.insert("phone_id".into(), serde_json::Value::String(phone.into()));
            }
            oc_channels.insert("whatsapp".into(), serde_json::Value::Object(obj));
            items += 1;
        }
    }

    if items == 0 {
        return AreaResult {
            area: MigrationArea::Channels,
            success: true,
            items_processed: 0,
            warnings: vec!["No channel definitions found to export".into()],
            error: None,
        };
    }

    // Merge into existing legacy.json
    let oc_config_path = oc_root.join("legacy.json");
    let mut oc_config: serde_json::Map<String, serde_json::Value> = if oc_config_path.exists() {
        match fs::read_to_string(&oc_config_path) {
            Ok(c) => match serde_json::from_str(&c) {
                Ok(map) => map,
                Err(e) => {
                    warnings.push(format!(
                        "Could not parse existing legacy.json: {e}; starting fresh"
                    ));
                    serde_json::Map::new()
                }
            },
            Err(e) => {
                warnings.push(format!(
                    "Could not read existing legacy.json: {e}; starting fresh"
                ));
                serde_json::Map::new()
            }
        }
    } else {
        serde_json::Map::new()
    };
    oc_config.insert("channels".into(), serde_json::Value::Object(oc_channels));

    if let Err(e) = fs::create_dir_all(oc_root) {
        return err(
            MigrationArea::Channels,
            format!("Failed to create output dir: {e}"),
        );
    }
    let serialized = match serde_json::to_string_pretty(&oc_config) {
        Ok(s) => s,
        Err(e) => {
            return err(
                MigrationArea::Channels,
                format!("Failed to serialize legacy.json: {e}"),
            );
        }
    };
    if let Err(e) = fs::write(&oc_config_path, &serialized) {
        return err(
            MigrationArea::Channels,
            format!("Failed to write legacy.json: {e}"),
        );
    }

    AreaResult {
        area: MigrationArea::Channels,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

// ═══════════════════════════════════════════════════════════════════════
// 7. Sub-agents transformer
// ═══════════════════════════════════════════════════════════════════════

const SKIP_AGENT_DIRS: &[&str] = &["duncan", "main"];

fn is_model_wrapper(name: &str) -> bool {
    let lower = name.to_lowercase();
    lower.starts_with("anthropic-")
        || lower.starts_with("google-")
        || lower.starts_with("moonshot-")
        || lower.starts_with("ollama-")
        || lower.starts_with("openai-")
}

fn agent_display_name(name: &str) -> String {
    name.split('-')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn detect_agent_model(agent_dir: &Path) -> String {
    let models_path = agent_dir.join("agent").join("models.json");
    if let Ok(content) = fs::read_to_string(&models_path)
        && let Ok(val) = serde_json::from_str::<serde_json::Value>(&content)
    {
        // Try top-level "primary" field
        if let Some(p) = val.get("primary").and_then(|v| v.as_str())
            && !p.is_empty()
        {
            return p.to_string();
        }
        // Try first model from first provider
        if let Some(providers) = val.get("providers").and_then(|v| v.as_object()) {
            for (prov_name, prov) in providers {
                if let Some(models) = prov.get("models").and_then(|v| v.as_array())
                    && let Some(first) = models.first()
                    && let Some(id) = first.get("id").and_then(|v| v.as_str())
                {
                    return format!("{prov_name}/{id}");
                }
            }
        }
    }
    String::new()
}

pub(crate) fn import_agents(oc_root: &Path, ic_root: &Path) -> AreaResult {
    let agents_dir = oc_root.join("agents");
    if !agents_dir.exists() {
        return AreaResult {
            area: MigrationArea::Agents,
            success: true,
            items_processed: 0,
            warnings: vec!["No agents directory found in Legacy root".into()],
            error: None,
        };
    }

    let db_path = ic_root.join("state.db");
    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Agents,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let mut items = 0;
    let mut warnings = Vec::new();

    let entries = match fs::read_dir(&agents_dir) {
        Ok(e) => e,
        Err(e) => {
            return err(
                MigrationArea::Agents,
                format!("Failed to read agents directory: {e}"),
            );
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let name = match entry.file_name().to_str() {
            Some(n) => n.to_string(),
            None => continue,
        };

        if SKIP_AGENT_DIRS.contains(&name.as_str()) {
            continue;
        }

        let role = if is_model_wrapper(&name) {
            "model-proxy"
        } else {
            "specialist"
        };

        let model = detect_agent_model(&path);

        let session_count = path
            .join("sessions")
            .read_dir()
            .map(|rd| rd.count() as i64)
            .unwrap_or(0);

        let display_name = agent_display_name(&name);

        let agent = ironclad_db::agents::SubAgentRow {
            id: uuid_v4(),
            name: name.clone(),
            display_name: Some(display_name),
            model,
            fallback_models_json: Some("[]".to_string()),
            role: role.to_string(),
            description: None,
            skills_json: None,
            enabled: role == "specialist",
            session_count,
        };

        match ironclad_db::agents::upsert_sub_agent(&db, &agent) {
            Ok(()) => items += 1,
            Err(e) => warnings.push(format!("Failed to import agent '{name}': {e}")),
        }
    }

    AreaResult {
        area: MigrationArea::Agents,
        success: true,
        items_processed: items,
        warnings,
        error: None,
    }
}

pub(crate) fn export_agents(ic_root: &Path, _oc_root: &Path) -> AreaResult {
    let db_path = ic_root.join("state.db");
    if !db_path.exists() {
        return AreaResult {
            area: MigrationArea::Agents,
            success: true,
            items_processed: 0,
            warnings: vec!["No database found".into()],
            error: None,
        };
    }

    let db = match ironclad_db::Database::new(&db_path.to_string_lossy()) {
        Ok(d) => d,
        Err(e) => {
            return err(
                MigrationArea::Agents,
                format!("Failed to open database: {e}"),
            );
        }
    };

    let agents = match ironclad_db::agents::list_sub_agents(&db) {
        Ok(a) => a,
        Err(e) => {
            return err(MigrationArea::Agents, format!("Failed to list agents: {e}"));
        }
    };

    AreaResult {
        area: MigrationArea::Agents,
        success: true,
        items_processed: agents.len(),
        warnings: vec![],
        error: None,
    }
}

// ── Helpers ────────────────────────────────────────────────────────────

pub(crate) fn qt(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

pub(crate) fn qt_ml(s: &str) -> String {
    format!("\"\"\"\n{}\n\"\"\"", s)
}

fn uuid_v4() -> String {
    uuid::Uuid::new_v4().to_string()
}
fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

fn err(area: MigrationArea, msg: String) -> AreaResult {
    AreaResult {
        area,
        success: false,
        items_processed: 0,
        warnings: vec![],
        error: Some(msg),
    }
}

/// Store a credential in the encrypted keystore during migration.
/// Creates and auto-unlocks the keystore if it doesn't exist yet.
/// The `ks_path` should be derived from the Ironclad root directory
/// (e.g., `ic_root.join("keystore.enc")`) to ensure test isolation.
fn store_in_keystore(key: &str, value: &str, ks_path: &Path) -> Result<(), String> {
    let ks = ironclad_core::keystore::Keystore::new(ks_path.to_path_buf());
    ks.unlock_machine().map_err(|e| e.to_string())?;
    ks.set(key, value).map_err(|e| e.to_string())
}

/// Read a credential from the keystore (for export back to Legacy format).
/// The `ks_path` should be derived from the Ironclad root directory.
fn read_from_keystore(key: &str, ks_path: &Path) -> Option<String> {
    let ks = ironclad_core::keystore::Keystore::new(ks_path.to_path_buf());
    if ks.unlock_machine().is_err() {
        return None;
    }
    ks.get(key)
}

/// Resolve a channel token for export: try `token_ref` (keystore) first, then `token_env`.
/// Inserts the resolved value as `"token"` into `obj`. Returns true if a value was inserted.
fn resolve_channel_token_for_export(
    channel_table: &toml::map::Map<String, toml::Value>,
    obj: &mut serde_json::Map<String, serde_json::Value>,
    warnings: &mut Vec<String>,
    channel_name: &str,
    ks_path: &Path,
) -> bool {
    if let Some(token_ref) = channel_table.get("token_ref").and_then(|v| v.as_str())
        && let Some(ks_name) = token_ref.strip_prefix("keystore:")
    {
        if let Some(val) = read_from_keystore(ks_name, ks_path) {
            obj.insert("token".into(), serde_json::Value::String(val));
            return true;
        }
        warnings.push(format!(
            "Keystore key \"{ks_name}\" not found; {channel_name} token omitted"
        ));
    }
    if let Some(env) = channel_table.get("token_env").and_then(|v| v.as_str()) {
        if let Ok(tok) = std::env::var(env) {
            obj.insert("token".into(), serde_json::Value::String(tok));
            return true;
        }
        warnings.push(format!(
            "Env var {env} not set; {channel_name} token omitted"
        ));
    }
    false
}

// ═══════════════════════════════════════════════════════════════════════
// Tests
// ═══════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn setup_legacy(dir: &Path) {
        fs::create_dir_all(dir.join("workspace/skills")).unwrap();
        fs::create_dir_all(dir.join("agents/duncan/sessions")).unwrap();

        let config = serde_json::json!({
            "name": "Duncan Idaho",
            "model": "gpt-4",
            "provider": "openai",
            "api_url": "https://api.openai.com/v1",
            "temperature": 0.7,
            "max_tokens": 4096,
            "channels": {
                "telegram": { "enabled": true, "token": "tg-token" },
                "whatsapp": { "enabled": false, "token": "wa-token", "phone_id": "12345" }
            },
            "cron": [
                { "name": "heartbeat", "schedule": {"kind": "cron", "expr": "*/5 * * * *"}, "command": "ping", "enabled": true },
                { "name": "cleanup", "schedule": "0 3 * * *", "command": "cleanup", "enabled": false }
            ]
        });
        fs::write(
            dir.join("legacy.json"),
            serde_json::to_string_pretty(&config).unwrap(),
        )
        .unwrap();

        fs::write(
            dir.join("workspace/SOUL.md"),
            "# Soul\n\n## Identity\nI am Duncan Idaho.\n\n## Traits\nLoyal, fierce, skilled.\n",
        )
        .unwrap();
        fs::write(
            dir.join("workspace/AGENTS.md"),
            "# Agents\n\n## Capabilities\nFighting, strategy, leadership.\n",
        )
        .unwrap();

        fs::write(
            dir.join("workspace/skills/greet.sh"),
            "#!/bin/bash\necho hello\n",
        )
        .unwrap();
        fs::write(dir.join("workspace/skills/math.py"), "print(2+2)\n").unwrap();

        let session = serde_json::json!([{
            "id": "sess-001", "agent_id": "duncan", "created_at": "2025-01-01T00:00:00Z",
            "messages": [
                { "role": "user", "content": "Hello", "timestamp": "2025-01-01T00:00:01Z" },
                { "role": "assistant", "content": "Hi there!", "timestamp": "2025-01-01T00:00:02Z" }
            ]
        }]);
        fs::write(
            dir.join("sessions.json"),
            serde_json::to_string_pretty(&session).unwrap(),
        )
        .unwrap();

        let jsonl = "{\"role\":\"user\",\"content\":\"JSONL msg\",\"timestamp\":\"2025-01-02T00:00:00Z\"}\n{\"role\":\"assistant\",\"content\":\"Reply\",\"timestamp\":\"2025-01-02T00:00:01Z\"}";
        fs::write(dir.join("agents/duncan/sessions/sess-002.jsonl"), jsonl).unwrap();
    }

    fn setup_ironclad(dir: &Path) {
        fs::create_dir_all(dir.join("workspace")).unwrap();
        fs::create_dir_all(dir.join("skills")).unwrap();

        fs::write(dir.join("ironclad.toml"), "[agent]\nname = \"Duncan Idaho\"\nid = \"duncan\"\nworkspace = \"/tmp/workspace\"\n\n[server]\nhost = \"127.0.0.1\"\nport = 18789\n\n[database]\npath = \"/tmp/ironclad.db\"\n\n[models]\nprimary = \"gpt-4\"\nfallback = \"gpt-3.5-turbo\"\ntemperature = 0.7\nmax_tokens = 4096\n").unwrap();
        fs::write(dir.join("channels.toml"), "[channels.telegram]\nenabled = true\ntoken_env = \"TELEGRAM_BOT_TOKEN\"\n\n[channels.whatsapp]\nenabled = false\ntoken_env = \"WHATSAPP_TOKEN\"\nphone_id = \"12345\"\n").unwrap();
        fs::write(dir.join("workspace/OS.toml"), "[os]\nprompt_text = \"\"\"\\n# Soul\\n\\n## Identity\\nI am Duncan.\\n\"\"\"\nidentity = \"I am Duncan.\"\n").unwrap();
        fs::write(
            dir.join("workspace/FIRMWARE.toml"),
            "[firmware]\ncapabilities = \"Fighting, strategy.\"\n",
        )
        .unwrap();
        fs::write(dir.join("skills/greet.gosh"), "echo hello\n").unwrap();
        fs::write(dir.join("skills/math.py"), "print(2+2)\n").unwrap();
    }

    // ── Config ─────────────────────────────────────────────────

    #[test]
    fn import_config_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_config(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
        let content = fs::read_to_string(ic.path().join("ironclad.toml")).unwrap();
        assert!(content.contains("Duncan Idaho"));
        assert!(content.contains("gpt-4"));
        assert!(
            content.contains("[channels.telegram]"),
            "ironclad.toml must include channel config"
        );
        assert!(content.contains("enabled = true"));
    }

    #[test]
    fn import_config_missing_file() {
        let r = import_config(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(!r.success);
    }

    #[test]
    fn export_config_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_config(ic.path(), oc.path());
        assert!(r.success);
        let content = fs::read_to_string(oc.path().join("legacy.json")).unwrap();
        assert!(content.contains("Duncan Idaho"));
        assert!(content.contains("gpt-4"));
    }

    #[test]
    fn config_roundtrip() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let oc2 = TempDir::new().unwrap();
        setup_legacy(oc.path());
        assert!(import_config(oc.path(), ic.path()).success);
        assert!(export_config(ic.path(), oc2.path()).success);
        let exported: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(oc2.path().join("legacy.json")).unwrap())
                .unwrap();
        assert_eq!(exported["name"], "Duncan Idaho");
        assert_eq!(exported["model"], "gpt-4");
    }

    #[test]
    fn export_config_merge_preserves_unknown_fields() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        fs::write(
            oc.path().join("legacy.json"),
            r#"{"custom_field":"preserved","name":"old"}"#,
        )
        .unwrap();
        let r = export_config(ic.path(), oc.path());
        assert!(r.success);
        let exported: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(oc.path().join("legacy.json")).unwrap())
                .unwrap();
        assert_eq!(exported["custom_field"], "preserved");
        assert_eq!(exported["name"], "Duncan Idaho");
    }

    // ── Personality ────────────────────────────────────────────

    #[test]
    fn import_personality_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_personality(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("workspace/OS.toml").exists());
        assert!(ic.path().join("workspace/FIRMWARE.toml").exists());
    }

    #[test]
    fn export_personality_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_personality(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(oc.path().join("workspace/SOUL.md").exists());
        assert!(oc.path().join("workspace/AGENTS.md").exists());
    }

    #[test]
    fn personality_roundtrip_via_prompt_text() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let oc2 = TempDir::new().unwrap();
        setup_legacy(oc.path());
        assert!(import_personality(oc.path(), ic.path()).success);
        assert!(export_personality(ic.path(), oc2.path()).success);
        let original = fs::read_to_string(oc.path().join("workspace/SOUL.md")).unwrap();
        let exported = fs::read_to_string(oc2.path().join("workspace/SOUL.md")).unwrap();
        assert_eq!(original.trim(), exported.trim());
    }

    #[test]
    fn personality_missing_files_warns() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        fs::create_dir_all(oc.path().join("workspace")).unwrap();
        let r = import_personality(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
        assert_eq!(r.warnings.len(), 2);
    }

    // ── Skills ─────────────────────────────────────────────────

    #[test]
    fn import_skills_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_skills(oc.path(), ic.path(), true);
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("skills/greet.sh").exists());
    }

    #[test]
    fn import_skills_no_dir() {
        let r = import_skills(Path::new("/nonexistent"), Path::new("/tmp"), true);
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_skills_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_skills(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    // ── Sessions ───────────────────────────────────────────────

    #[test]
    fn import_sessions_from_json() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert!(r.items_processed >= 1);
        assert!(ic.path().join("state.db").exists());
    }

    #[test]
    fn import_sessions_from_jsonl() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        fs::remove_file(oc.path().join("sessions.json")).unwrap();
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
    }

    #[test]
    fn export_sessions_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        import_sessions(oc.path(), ic.path());
        let out = TempDir::new().unwrap();
        let r = export_sessions(ic.path(), out.path());
        assert!(r.success);
        assert!(r.items_processed >= 1);
        assert!(out.path().join("sessions.json").exists());
    }

    #[test]
    fn sessions_no_data() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let r = import_sessions(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Cron ───────────────────────────────────────────────────

    #[test]
    fn import_cron_from_config() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_cron(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    #[test]
    fn import_cron_from_jobs_json() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let jobs = serde_json::json!([{ "name": "daily", "schedule": "0 0 * * *", "command": "report", "enabled": true }]);
        fs::write(
            oc.path().join("jobs.json"),
            serde_json::to_string(&jobs).unwrap(),
        )
        .unwrap();
        let r = import_cron(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 1);
    }

    #[test]
    fn export_cron_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        import_cron(oc.path(), ic.path());
        let out = TempDir::new().unwrap();
        let r = export_cron(ic.path(), out.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(out.path().join("jobs.json").exists());
    }

    #[test]
    fn cron_no_data() {
        let r = import_cron(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Channels ───────────────────────────────────────────────

    #[test]
    fn import_channels_succeeds() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        setup_legacy(oc.path());
        let r = import_channels(oc.path(), ic.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
        assert!(ic.path().join("channels.toml").exists());
    }

    #[test]
    fn export_channels_succeeds() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        setup_ironclad(ic.path());
        let r = export_channels(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 2);
    }

    #[test]
    fn channels_no_config() {
        let r = import_channels(Path::new("/nonexistent"), Path::new("/tmp"));
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    // ── Personality conversion ─────────────────────────────────

    #[test]
    fn markdown_to_toml_has_prompt_text() {
        let md = "# Title\n\n## Identity\nI am Duncan.\n";
        let toml = markdown_to_personality_toml(md, "os");
        assert!(toml.contains("[os]"));
        assert!(toml.contains("prompt_text"));
        assert!(toml.contains("identity"));
    }

    #[test]
    fn toml_to_markdown_uses_prompt_text() {
        let toml_str = "[os]\nprompt_text = \"\"\"\n# Soul\n\n## Identity\nI am Duncan.\n\"\"\"\n";
        let md = personality_toml_to_markdown(toml_str, "SOUL");
        assert!(md.contains("# Soul"));
        assert!(md.contains("Duncan"));
    }

    // ── Helpers ────────────────────────────────────────────────

    #[test]
    fn qt_escapes() {
        assert_eq!(qt("hello"), "\"hello\"");
        assert_eq!(qt("he\"llo"), "\"he\\\"llo\"");
        assert_eq!(qt("a\\b"), "\"a\\\\b\"");
    }

    #[test]
    fn qt_empty_string() {
        assert_eq!(qt(""), "\"\"");
    }

    #[test]
    fn qt_ml_wraps_in_triple_quotes() {
        let result = qt_ml("hello\nworld");
        assert!(result.starts_with("\"\"\""));
        assert!(result.ends_with("\"\"\""));
        assert!(result.contains("hello\nworld"));
    }

    // ── titlecase ─────────────────────────────────────────────

    #[test]
    fn titlecase_converts_underscored_keys() {
        assert_eq!(titlecase("prompt_text"), "Prompt Text");
        assert_eq!(titlecase("identity"), "Identity");
        assert_eq!(titlecase("core_values"), "Core Values");
    }

    #[test]
    fn titlecase_empty_string() {
        assert_eq!(titlecase(""), "");
    }

    #[test]
    fn titlecase_single_word() {
        assert_eq!(titlecase("traits"), "Traits");
    }

    #[test]
    fn titlecase_multiple_underscores() {
        assert_eq!(titlecase("my_great_key_name"), "My Great Key Name");
    }

    // ── parse_skill_description ────────────────────────────────

    #[test]
    fn parse_skill_description_extracts_from_frontmatter() {
        let content = "---\nname: my-skill\ndescription: A cool skill\n---\n# Body\nContent here.";
        assert_eq!(
            parse_skill_description(content),
            Some("A cool skill".to_string())
        );
    }

    #[test]
    fn parse_skill_description_handles_quoted_desc() {
        let content = "---\ndescription: \"Quoted description\"\n---\nBody";
        assert_eq!(
            parse_skill_description(content),
            Some("Quoted description".to_string())
        );
    }

    #[test]
    fn parse_skill_description_returns_none_without_frontmatter() {
        assert_eq!(parse_skill_description("Just text"), None);
        assert_eq!(parse_skill_description(""), None);
    }

    #[test]
    fn parse_skill_description_returns_none_for_empty_desc() {
        let content = "---\nname: test\ndescription: \n---\nBody";
        assert_eq!(parse_skill_description(content), None);
    }

    #[test]
    fn parse_skill_description_no_closing_frontmatter() {
        let content = "---\nname: test\ndescription: missing close";
        assert_eq!(parse_skill_description(content), None);
    }

    // ── is_model_wrapper ──────────────────────────────────────

    #[test]
    fn is_model_wrapper_detects_provider_prefixed_names() {
        assert!(is_model_wrapper("anthropic-claude"));
        assert!(is_model_wrapper("google-gemini"));
        assert!(is_model_wrapper("moonshot-v1"));
        assert!(is_model_wrapper("ollama-qwen"));
        assert!(is_model_wrapper("openai-gpt4"));
        assert!(is_model_wrapper("OpenAI-GPT4"));
    }

    #[test]
    fn is_model_wrapper_rejects_non_provider_names() {
        assert!(!is_model_wrapper("geo-specialist"));
        assert!(!is_model_wrapper("risk-analyst"));
        assert!(!is_model_wrapper("duncan"));
        assert!(!is_model_wrapper(""));
    }

    // ── agent_display_name ────────────────────────────────────

    #[test]
    fn agent_display_name_capitalizes_and_joins() {
        assert_eq!(agent_display_name("geo-specialist"), "Geo Specialist");
        assert_eq!(agent_display_name("risk"), "Risk");
        assert_eq!(agent_display_name("multi-word-name"), "Multi Word Name");
    }

    #[test]
    fn agent_display_name_empty_segments() {
        // Leading/trailing hyphens produce empty segments
        assert_eq!(agent_display_name("-test-"), " Test ");
    }

    // ── err helper ──────────────────────────────────────────────

    #[test]
    fn err_helper_sets_fields_correctly() {
        let result = err(MigrationArea::Config, "something failed".to_string());
        assert!(!result.success);
        assert_eq!(result.items_processed, 0);
        assert!(result.warnings.is_empty());
        assert_eq!(result.error, Some("something failed".to_string()));
    }

    // ── markdown_to_personality_toml edge cases ──────────────────

    #[test]
    fn markdown_to_personality_toml_empty_input() {
        let toml = markdown_to_personality_toml("", "soul");
        assert!(toml.contains("[soul]"));
        assert!(toml.contains("prompt_text"));
    }

    #[test]
    fn markdown_to_personality_toml_with_multiple_sections() {
        let md = "# Soul\n\n## Identity\nI am Duncan.\n\n## Traits\nLoyal and fierce.\n\n## Values\nHonor above all.\n";
        let toml = markdown_to_personality_toml(md, "soul");
        assert!(toml.contains("[soul]"));
        assert!(toml.contains("identity"));
        assert!(toml.contains("traits"));
        assert!(toml.contains("values"));
    }

    // ── personality_toml_to_markdown edge cases ──────────────────

    #[test]
    fn personality_toml_to_markdown_invalid_toml_wraps_raw() {
        let invalid = "this is not valid toml {{{{";
        let md = personality_toml_to_markdown(invalid, "Test");
        assert!(md.contains("# Test"));
        assert!(md.contains(invalid));
    }

    #[test]
    fn personality_toml_to_markdown_structured_fields_without_prompt_text() {
        let toml_str = "[soul]\nidentity = \"I am Duncan.\"\ntraits = \"Loyal, fierce.\"\n";
        let md = personality_toml_to_markdown(toml_str, "SOUL");
        assert!(md.contains("# SOUL"));
        assert!(md.contains("## Identity"));
        assert!(md.contains("I am Duncan."));
        assert!(md.contains("## Traits"));
    }

    // ── detect_agent_model ──────────────────────────────────────

    #[test]
    fn detect_agent_model_returns_empty_for_nonexistent_dir() {
        assert_eq!(detect_agent_model(Path::new("/nonexistent/agent")), "");
    }

    #[test]
    fn detect_agent_model_reads_primary_field() {
        let dir = TempDir::new().unwrap();
        let agent_dir = dir.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let models = serde_json::json!({"primary": "openai/gpt-4o"});
        fs::write(
            agent_dir.join("models.json"),
            serde_json::to_string(&models).unwrap(),
        )
        .unwrap();
        assert_eq!(detect_agent_model(dir.path()), "openai/gpt-4o");
    }

    #[test]
    fn detect_agent_model_reads_from_providers() {
        let dir = TempDir::new().unwrap();
        let agent_dir = dir.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        let models = serde_json::json!({
            "providers": {
                "openai": {
                    "models": [{"id": "gpt-4"}]
                }
            }
        });
        fs::write(
            agent_dir.join("models.json"),
            serde_json::to_string(&models).unwrap(),
        )
        .unwrap();
        assert_eq!(detect_agent_model(dir.path()), "openai/gpt-4");
    }

    #[test]
    fn detect_agent_model_returns_empty_for_empty_json() {
        let dir = TempDir::new().unwrap();
        let agent_dir = dir.path().join("agent");
        fs::create_dir_all(&agent_dir).unwrap();
        fs::write(agent_dir.join("models.json"), "{}").unwrap();
        assert_eq!(detect_agent_model(dir.path()), "");
    }
}
