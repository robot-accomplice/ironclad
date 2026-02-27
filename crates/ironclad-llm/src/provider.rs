use std::collections::HashMap;

use ironclad_core::config::ProviderConfig;
use ironclad_core::{ApiFormat, ModelTier};

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub url: String,
    pub tier: ModelTier,
    pub api_key_env: String,
    pub format: ApiFormat,
    pub chat_path: String,
    pub embedding_path: Option<String>,
    pub embedding_model: Option<String>,
    pub embedding_dimensions: Option<usize>,
    pub is_local: bool,
    pub cost_per_input_token: f64,
    pub cost_per_output_token: f64,
    pub auth_header: String,
    pub extra_headers: HashMap<String, String>,
    pub tpm_limit: Option<u64>,
    pub rpm_limit: Option<u64>,
    pub auth_mode: String,
    pub oauth_client_id: Option<String>,
    pub api_key_ref: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProviderRegistry {
    providers: HashMap<String, Provider>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self {
            providers: HashMap::new(),
        }
    }

    pub fn register(&mut self, provider: Provider) {
        self.providers.insert(provider.name.clone(), provider);
    }

    pub fn get(&self, name: &str) -> Option<&Provider> {
        self.providers.get(name)
    }

    /// Extract provider prefix from "provider/model" format and look up.
    pub fn get_by_model(&self, model: &str) -> Option<&Provider> {
        let prefix = model.split('/').next()?;
        self.providers.get(prefix)
    }

    pub fn list(&self) -> Vec<&Provider> {
        self.providers.values().collect()
    }

    pub fn from_config(providers: &HashMap<String, ProviderConfig>) -> Self {
        let mut registry = Self::new();

        for (name, cfg) in providers {
            let tier = parse_tier(&cfg.tier);

            let format = cfg
                .format
                .as_deref()
                .map(parse_api_format)
                .unwrap_or_else(|| infer_api_format(name));

            let api_key_env = cfg
                .api_key_env
                .clone()
                .unwrap_or_else(|| format!("{}_API_KEY", name.to_uppercase()));

            let chat_path = cfg
                .chat_path
                .clone()
                .unwrap_or_else(|| default_chat_path(format));

            let is_local = cfg.is_local.unwrap_or_else(|| infer_is_local(name));

            let auth_header = cfg
                .auth_header
                .clone()
                .unwrap_or_else(|| "Authorization".into());

            let extra_headers = cfg.extra_headers.clone().unwrap_or_default();

            let auth_mode = cfg.auth_mode.clone().unwrap_or_else(|| "api_key".into());

            registry.register(Provider {
                name: name.clone(),
                url: cfg.url.clone(),
                tier,
                api_key_env,
                format,
                chat_path,
                embedding_path: cfg.embedding_path.clone(),
                embedding_model: cfg.embedding_model.clone(),
                embedding_dimensions: cfg.embedding_dimensions,
                is_local,
                cost_per_input_token: cfg.cost_per_input_token.unwrap_or(0.0),
                cost_per_output_token: cfg.cost_per_output_token.unwrap_or(0.0),
                auth_header,
                extra_headers,
                tpm_limit: cfg.tpm_limit,
                rpm_limit: cfg.rpm_limit,
                auth_mode,
                oauth_client_id: cfg.oauth_client_id.clone(),
                api_key_ref: cfg.api_key_ref.clone(),
            });
        }

        registry
    }
}

impl Default for ProviderRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_tier(s: &str) -> ModelTier {
    match s {
        "T1" => ModelTier::T1,
        "T2" => ModelTier::T2,
        "T3" => ModelTier::T3,
        "T4" => ModelTier::T4,
        _ => ModelTier::T2,
    }
}

pub fn parse_api_format(s: &str) -> ApiFormat {
    match s.to_lowercase().as_str() {
        "anthropic" => ApiFormat::AnthropicMessages,
        "google" => ApiFormat::GoogleGenerativeAi,
        "openai-responses" => ApiFormat::OpenAiResponses,
        _ => ApiFormat::OpenAiCompletions,
    }
}

fn default_chat_path(format: ApiFormat) -> String {
    match format {
        ApiFormat::AnthropicMessages => "/v1/messages".into(),
        ApiFormat::GoogleGenerativeAi => String::new(),
        _ => "/v1/chat/completions".into(),
    }
}

fn infer_api_format(provider_name: &str) -> ApiFormat {
    let lower = provider_name.to_lowercase();
    if lower.contains("anthropic") || lower.contains("claude") {
        ApiFormat::AnthropicMessages
    } else if lower.contains("google") || lower.contains("gemini") {
        ApiFormat::GoogleGenerativeAi
    } else {
        ApiFormat::OpenAiCompletions
    }
}

fn infer_is_local(provider_name: &str) -> bool {
    let lower = provider_name.to_lowercase();
    lower.contains("ollama")
        || lower.contains("local")
        || lower.contains("llama-cpp")
        || lower.contains("llama_cpp")
        || lower.contains("sglang")
        || lower.contains("vllm")
        || lower.contains("docker-model-runner")
        || lower.contains("docker_model_runner")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_provider(name: &str, url: &str, tier: ModelTier, format: ApiFormat) -> Provider {
        Provider {
            name: name.into(),
            url: url.into(),
            tier,
            api_key_env: format!("{}_API_KEY", name.to_uppercase()),
            format,
            chat_path: default_chat_path(format),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: infer_is_local(name),
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        }
    }

    #[test]
    fn register_and_lookup() {
        let mut reg = ProviderRegistry::new();
        reg.register(test_provider(
            "ollama",
            "http://localhost:11434",
            ModelTier::T1,
            ApiFormat::OpenAiCompletions,
        ));

        assert!(reg.get("ollama").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn lookup_by_model_string() {
        let mut reg = ProviderRegistry::new();
        reg.register(test_provider(
            "openai",
            "https://api.openai.com",
            ModelTier::T3,
            ApiFormat::OpenAiCompletions,
        ));

        let provider = reg.get_by_model("openai/gpt-5.3-codex").unwrap();
        assert_eq!(provider.name, "openai");
        assert!(reg.get_by_model("unknown/model").is_none());
    }

    #[test]
    fn from_config_maps_tiers_and_formats() {
        let mut providers = HashMap::new();
        providers.insert(
            "anthropic".to_string(),
            ProviderConfig::new("https://api.anthropic.com", "T3"),
        );
        providers.insert(
            "ollama".to_string(),
            ProviderConfig::new("http://localhost:11434", "T1"),
        );
        providers.insert(
            "google".to_string(),
            ProviderConfig::new("https://generativelanguage.googleapis.com", "T3"),
        );

        let reg = ProviderRegistry::from_config(&providers);
        assert_eq!(reg.list().len(), 3);

        let anthropic = reg.get("anthropic").unwrap();
        assert_eq!(anthropic.tier, ModelTier::T3);
        assert_eq!(anthropic.format, ApiFormat::AnthropicMessages);
        assert_eq!(anthropic.chat_path, "/v1/messages");

        let ollama = reg.get("ollama").unwrap();
        assert_eq!(ollama.tier, ModelTier::T1);
        assert_eq!(ollama.format, ApiFormat::OpenAiCompletions);
        assert!(ollama.is_local);

        let google = reg.get("google").unwrap();
        assert_eq!(google.format, ApiFormat::GoogleGenerativeAi);
        assert!(!google.is_local);
    }

    #[test]
    fn config_format_overrides_inference() {
        let mut providers = HashMap::new();
        let mut cfg = ProviderConfig::new("http://custom-api.example.com", "T2");
        cfg.format = Some("anthropic".into());
        cfg.api_key_env = Some("CUSTOM_KEY".into());
        cfg.chat_path = Some("/api/v2/chat".into());
        cfg.is_local = Some(true);
        cfg.auth_header = Some("x-api-key".into());
        let mut headers = HashMap::new();
        headers.insert("x-custom".into(), "value".into());
        cfg.extra_headers = Some(headers);
        cfg.cost_per_input_token = Some(0.001);
        cfg.cost_per_output_token = Some(0.002);
        providers.insert("custom".to_string(), cfg);

        let reg = ProviderRegistry::from_config(&providers);
        let p = reg.get("custom").unwrap();
        assert_eq!(p.format, ApiFormat::AnthropicMessages);
        assert_eq!(p.api_key_env, "CUSTOM_KEY");
        assert_eq!(p.chat_path, "/api/v2/chat");
        assert!(p.is_local);
        assert_eq!(p.auth_header, "x-api-key");
        assert_eq!(p.extra_headers.get("x-custom").unwrap(), "value");
        assert!((p.cost_per_input_token - 0.001).abs() < f64::EPSILON);
        assert!((p.cost_per_output_token - 0.002).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_api_format_variants() {
        assert_eq!(parse_api_format("openai"), ApiFormat::OpenAiCompletions);
        assert_eq!(parse_api_format("anthropic"), ApiFormat::AnthropicMessages);
        assert_eq!(parse_api_format("google"), ApiFormat::GoogleGenerativeAi);
        assert_eq!(
            parse_api_format("openai-responses"),
            ApiFormat::OpenAiResponses
        );
        assert_eq!(parse_api_format("OPENAI"), ApiFormat::OpenAiCompletions);
        assert_eq!(parse_api_format("unknown"), ApiFormat::OpenAiCompletions);
    }

    #[test]
    fn parse_tier_variants() {
        assert_eq!(parse_tier("T1"), ModelTier::T1);
        assert_eq!(parse_tier("T2"), ModelTier::T2);
        assert_eq!(parse_tier("T3"), ModelTier::T3);
        assert_eq!(parse_tier("T4"), ModelTier::T4);
        assert_eq!(parse_tier("unknown"), ModelTier::T2);
    }
}
