pub mod accuracy;
pub mod cache;
pub mod capacity;
pub mod cascade;
pub mod circuit;
pub mod client;
pub mod compression;
pub mod dedup;
pub mod format;
pub mod ml_router;
pub mod provider;
pub mod router;
pub mod tier;
pub mod tiered;
pub mod uniroute;

pub use accuracy::{QualityTracker, select_for_quality_target};
pub use cache::{CachedResponse, SemanticCache};
pub use capacity::CapacityTracker;
pub use cascade::{CascadeOptimizer, CascadeOutcome, CascadeStrategy};
pub use circuit::{CircuitBreakerRegistry, CircuitState};
pub use client::LlmClient;
pub use compression::{CompressionEstimate, PromptCompressor};
pub use dedup::DedupTracker;
pub use ml_router::{LogisticBackend, PreferenceCollector, PreferenceRecord};
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

        Ok(Self {
            cache,
            breakers,
            dedup,
            router,
            client,
            providers,
            capacity,
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
