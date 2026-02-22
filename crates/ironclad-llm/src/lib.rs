pub mod accuracy;
pub mod cache;
pub mod capacity;
pub mod cascade;
pub mod circuit;
pub mod client;
pub mod compression;
pub mod dedup;
pub mod embedding;
pub mod format;
pub mod ml_router;
pub mod oauth;
pub mod provider;
pub mod router;
pub mod tier;
pub mod tiered;
pub mod uniroute;

pub use accuracy::{QualityTracker, select_for_quality_target};
pub use cache::{CachedResponse, ExportedCacheEntry, SemanticCache};
pub use capacity::CapacityTracker;
pub use cascade::{CascadeOptimizer, CascadeOutcome, CascadeStrategy};
pub use circuit::{CircuitBreakerRegistry, CircuitState};
pub use client::LlmClient;
pub use compression::{CompressionEstimate, PromptCompressor};
pub use dedup::DedupTracker;
pub use embedding::{EmbeddingClient, EmbeddingConfig};
pub use ml_router::{LogisticBackend, PreferenceCollector, PreferenceRecord};
pub use oauth::OAuthManager;
pub use provider::{Provider, ProviderRegistry};
pub use router::{ModelRouter, classify_complexity, extract_features};
pub use tiered::{ConfidenceEvaluator, EscalationTracker, InferenceTier, TieredResult};
pub use uniroute::{ModelVector, ModelVectorRegistry, QueryRequirements};

use ironclad_core::{IroncladConfig, Result};
use router::HeuristicBackend;

pub struct LlmService {
    pub cache: SemanticCache,
    pub breakers: CircuitBreakerRegistry,
    pub dedup: DedupTracker,
    pub router: ModelRouter,
    pub client: LlmClient,
    pub providers: ProviderRegistry,
    pub capacity: CapacityTracker,
    pub embedding: EmbeddingClient,
}

impl LlmService {
    pub fn new(config: &IroncladConfig) -> Result<Self> {
        let cache = SemanticCache::with_threshold(
            config.cache.enabled,
            config.cache.exact_match_ttl_seconds,
            config.cache.max_entries,
            config.cache.semantic_threshold as f32,
        );

        let breakers = CircuitBreakerRegistry::new(&config.circuit_breaker);

        let dedup = DedupTracker::default();

        let routing_config = config.models.routing.clone();

        let router = ModelRouter::new(
            config.models.primary.clone(),
            config.models.fallbacks.clone(),
            routing_config,
            Box::new(HeuristicBackend),
        );

        let client = LlmClient::new()?;

        let providers = ProviderRegistry::from_config(&config.providers);

        let capacity = CapacityTracker::new(60);
        for provider in providers.list() {
            capacity.register(&provider.name, provider.tpm_limit, provider.rpm_limit);
        }

        let embedding_config = Self::resolve_embedding_config(&config.memory, &providers);
        let embedding = EmbeddingClient::new(embedding_config)?;

        Ok(Self {
            cache,
            breakers,
            dedup,
            router,
            client,
            providers,
            capacity,
            embedding,
        })
    }

    fn resolve_embedding_config(
        memory: &ironclad_core::config::MemoryConfig,
        providers: &ProviderRegistry,
    ) -> Option<EmbeddingConfig> {
        let provider_name = memory.embedding_provider.as_deref()?;
        let provider = providers.get(provider_name)?;
        let embedding_path = provider.embedding_path.as_deref()?;

        let model = memory
            .embedding_model
            .clone()
            .or_else(|| provider.embedding_model.clone())?;

        let dimensions = provider.embedding_dimensions.unwrap_or(768);

        Some(EmbeddingConfig {
            base_url: provider.url.clone(),
            embedding_path: embedding_path.to_string(),
            model,
            dimensions,
            format: provider.format,
            api_key_env: provider.api_key_env.clone(),
            auth_header: provider.auth_header.clone(),
            extra_headers: provider.extra_headers.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_service_construction() {
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
fallbacks = ["openai/gpt-4o"]

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"

[providers.openai]
url = "https://api.openai.com"
tier = "T3"
"#;
        let config = IroncladConfig::from_str(toml).unwrap();
        let service = LlmService::new(&config).unwrap();

        assert_eq!(service.router.select_model(), "ollama/qwen3:8b");
        assert_eq!(service.cache.size(), 0);
        assert!(service.providers.get("ollama").is_some());
        assert!(service.providers.get("openai").is_some());
        assert!(!service.embedding.has_provider());
    }

    #[test]
    fn llm_service_with_embedding_provider() {
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
embedding_provider = "ollama"

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"
embedding_path = "/api/embed"
embedding_model = "nomic-embed-text"
embedding_dimensions = 768
"#;
        let config = IroncladConfig::from_str(toml).unwrap();
        let service = LlmService::new(&config).unwrap();
        assert!(service.embedding.has_provider());
        assert_eq!(service.embedding.dimensions(), 768);
    }

    #[test]
    fn resolve_embedding_config_no_provider() {
        let memory = ironclad_core::config::MemoryConfig::default();
        let providers = ProviderRegistry::new();
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_missing_provider() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("nonexistent".into()),
            ..Default::default()
        };
        let providers = ProviderRegistry::new();
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_provider_no_embedding_path() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("anthropic".into()),
            ..Default::default()
        };
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert(
            "anthropic".to_string(),
            ironclad_core::config::ProviderConfig::new("https://api.anthropic.com", "T3"),
        );
        let providers = ProviderRegistry::from_config(&providers_cfg);
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_uses_memory_model_override() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("openai".into()),
            embedding_model: Some("text-embedding-3-large".into()),
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("https://api.openai.com", "T3");
        cfg.embedding_path = Some("/v1/embeddings".into());
        cfg.embedding_model = Some("text-embedding-3-small".into());
        cfg.embedding_dimensions = Some(1536);
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("openai".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.model, "text-embedding-3-large");
        assert_eq!(result.dimensions, 1536);
    }

    #[test]
    fn resolve_embedding_config_falls_back_to_provider_model() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("ollama".into()),
            embedding_model: None,
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("http://localhost:11434", "T1");
        cfg.embedding_path = Some("/api/embed".into());
        cfg.embedding_model = Some("nomic-embed-text".into());
        cfg.embedding_dimensions = Some(768);
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("ollama".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.model, "nomic-embed-text");
        assert_eq!(result.dimensions, 768);
        assert_eq!(result.base_url, "http://localhost:11434");
        assert_eq!(result.embedding_path, "/api/embed");
    }

    #[test]
    fn resolve_embedding_config_default_dimensions() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("custom".into()),
            embedding_model: Some("my-model".into()),
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("http://localhost:8080", "T1");
        cfg.embedding_path = Some("/embed".into());
        cfg.embedding_model = Some("my-model".into());
        // No dimensions set — should default to 768
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("custom".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.dimensions, 768);
    }
}
