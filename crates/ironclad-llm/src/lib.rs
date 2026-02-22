pub mod cache;
pub mod circuit;
pub mod client;
pub mod dedup;
pub mod format;
pub mod provider;
pub mod router;
pub mod tier;

pub use cache::{CachedResponse, SemanticCache};
pub use circuit::{CircuitBreakerRegistry, CircuitState};
pub use client::LlmClient;
pub use dedup::DedupTracker;
pub use provider::{Provider, ProviderRegistry};
pub use router::{ModelRouter, classify_complexity, extract_features};

use ironclad_core::config::RoutingConfig;
use ironclad_core::{IroncladConfig, Result};
use router::HeuristicBackend;

pub struct LlmService {
    pub cache: SemanticCache,
    pub breakers: CircuitBreakerRegistry,
    pub dedup: DedupTracker,
    pub router: ModelRouter,
    pub client: LlmClient,
    pub providers: ProviderRegistry,
}

impl LlmService {
    pub fn new(config: &IroncladConfig) -> Result<Self> {
        let cache = SemanticCache::new(
            config.cache.enabled,
            config.cache.exact_match_ttl_seconds,
            config.cache.max_entries,
        );

        let breakers = CircuitBreakerRegistry::new(&config.circuit_breaker);

        let dedup = DedupTracker::default();

        let routing_config = RoutingConfig {
            mode: config.models.routing.mode.clone(),
            confidence_threshold: config.models.routing.confidence_threshold,
            local_first: config.models.routing.local_first,
        };

        let router = ModelRouter::new(
            config.models.primary.clone(),
            config.models.fallbacks.clone(),
            routing_config,
            Box::new(HeuristicBackend),
        );

        let client = LlmClient::new()?;

        let providers = ProviderRegistry::from_config(&config.providers);

        Ok(Self {
            cache,
            breakers,
            dedup,
            router,
            client,
            providers,
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
    }
}
