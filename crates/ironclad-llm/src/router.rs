use ironclad_core::config::RoutingConfig;
use ironclad_core::{IroncladError, Result};

use crate::provider::ProviderRegistry;

/// Backend for complexity classification (heuristic or future ML).
pub trait RouterBackend: Send + Sync + std::fmt::Debug {
    fn classify_complexity(&self, features: &[f32]) -> f64;
}

/// Heuristic complexity scoring: weighted sum of message length, tool calls, depth.
#[derive(Debug, Default)]
pub struct HeuristicBackend;

impl RouterBackend for HeuristicBackend {
    fn classify_complexity(&self, features: &[f32]) -> f64 {
        heuristic_classify_complexity(features)
    }
}

#[derive(Debug)]
pub struct ModelRouter {
    primary: String,
    fallbacks: Vec<String>,
    current_index: usize,
    config: RoutingConfig,
    backend: Box<dyn RouterBackend>,
    model_override: Option<String>,
}

impl ModelRouter {
    pub fn new(
        primary: String,
        fallbacks: Vec<String>,
        config: RoutingConfig,
        backend: Box<dyn RouterBackend>,
    ) -> Self {
        Self {
            primary,
            fallbacks,
            current_index: 0,
            config,
            backend,
            model_override: None,
        }
    }

    /// Classify complexity from a feature vector using the configured backend.
    pub fn classify_complexity(&self, features: &[f32]) -> f64 {
        self.backend.classify_complexity(features)
    }

    pub fn select_model(&self) -> &str {
        if let Some(ref ovr) = self.model_override {
            return ovr;
        }
        match self.config.mode.as_str() {
            "primary" => &self.primary,
            "round-robin" => {
                let all_count = 1 + self.fallbacks.len();
                let idx = self.current_index % all_count;
                if idx == 0 {
                    &self.primary
                } else {
                    &self.fallbacks[idx - 1]
                }
            }
            _ => {
                if self.current_index == 0 {
                    &self.primary
                } else {
                    self.fallbacks
                        .get(self.current_index - 1)
                        .unwrap_or(&self.primary)
                }
            }
        }
    }

    /// Force all future model selection to use this model until cleared.
    pub fn set_override(&mut self, model: String) {
        self.model_override = Some(model);
    }

    /// Remove any manual model override, returning to normal routing.
    pub fn clear_override(&mut self) {
        self.model_override = None;
    }

    /// Returns the current model override, if set.
    pub fn get_override(&self) -> Option<&str> {
        self.model_override.as_deref()
    }

    pub fn primary(&self) -> &str {
        &self.primary
    }

    pub fn fallbacks(&self) -> &[String] {
        &self.fallbacks
    }

    /// Select a model using complexity-aware routing, consulting the provider
    /// registry for `is_local`, the capacity tracker for throughput headroom,
    /// and the circuit breaker registry to skip blocked providers.
    pub fn select_for_complexity(
        &self,
        complexity: f64,
        registry: Option<&ProviderRegistry>,
        capacity: Option<&super::capacity::CapacityTracker>,
        breakers: Option<&super::circuit::CircuitBreakerRegistry>,
    ) -> &str {
        if let Some(ref ovr) = self.model_override {
            return ovr;
        }
        if self.config.mode == "primary" {
            return &self.primary;
        }

        let is_provider_blocked = |model: &str| -> bool {
            if let Some(br) = breakers {
                let prefix = model.split('/').next().unwrap_or(model);
                br.is_blocked(prefix)
            } else {
                false
            }
        };

        let primary_is_local = match registry {
            Some(reg) => reg.get_by_model(&self.primary).is_some_and(|p| p.is_local),
            None => is_local_model_heuristic(&self.primary),
        };

        if self.config.local_first
            && primary_is_local
            && complexity < self.config.confidence_threshold
            && !is_provider_blocked(&self.primary)
        {
            return &self.primary;
        }

        let selected =
            if complexity >= self.config.confidence_threshold && !self.fallbacks.is_empty() {
                &self.fallbacks[0]
            } else {
                &self.primary
            };

        if is_provider_blocked(selected) {
            for fb in &self.fallbacks {
                if !is_provider_blocked(fb) {
                    return fb;
                }
            }
            if !is_provider_blocked(&self.primary) {
                return &self.primary;
            }
        }

        if let Some(cap) = capacity {
            let provider_name = selected.split('/').next().unwrap_or(selected);
            if cap.is_near_capacity(provider_name) {
                for fb in &self.fallbacks {
                    let fb_provider = fb.split('/').next().unwrap_or(fb);
                    if !cap.is_near_capacity(fb_provider) && !is_provider_blocked(fb) {
                        return fb;
                    }
                }
            }
        }

        selected
    }

    /// Select the cheapest qualified model that has capacity and is not blocked.
    /// Falls back to complexity-based selection if no cost data is available.
    pub fn select_cheapest_qualified(
        &self,
        complexity: f64,
        registry: &ProviderRegistry,
        capacity: Option<&super::capacity::CapacityTracker>,
        breakers: Option<&super::circuit::CircuitBreakerRegistry>,
        estimated_input_tokens: u32,
        estimated_output_tokens: u32,
    ) -> &str {
        if let Some(ref ovr) = self.model_override {
            return ovr;
        }
        let mut candidates: Vec<(&str, f64)> = Vec::new();

        let primary_cost = estimate_cost(
            &self.primary,
            registry,
            estimated_input_tokens,
            estimated_output_tokens,
        );
        candidates.push((&self.primary, primary_cost));

        for fb in &self.fallbacks {
            let cost = estimate_cost(
                fb,
                registry,
                estimated_input_tokens,
                estimated_output_tokens,
            );
            candidates.push((fb, cost));
        }

        // Filter out providers with tripped circuit breakers
        if let Some(br) = breakers {
            candidates.retain(|(model, _)| {
                let prefix = model.split('/').next().unwrap_or(model);
                !br.is_blocked(prefix)
            });
        }

        if let Some(cap) = capacity {
            candidates.retain(|(model, _)| {
                let provider_name = model.split('/').next().unwrap_or(model);
                !cap.is_near_capacity(provider_name)
            });
        }

        if complexity >= self.config.confidence_threshold {
            let cloud: Vec<_> = candidates
                .iter()
                .filter(|(model, _)| {
                    registry
                        .get_by_model(model)
                        .map(|p| !p.is_local)
                        .unwrap_or(true)
                })
                .cloned()
                .collect();
            if !cloud.is_empty() {
                return cloud
                    .iter()
                    .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|(model, _)| *model)
                    .unwrap_or(&self.primary);
            }
        }

        if let Some((model, _)) = candidates
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        {
            return model;
        }

        &self.primary
    }

    pub fn advance_fallback(&mut self) -> Result<&str> {
        if self.current_index >= self.fallbacks.len() {
            return Err(IroncladError::Llm("all fallback models exhausted".into()));
        }
        self.current_index += 1;
        Ok(self.select_model())
    }

    pub fn reset(&mut self) {
        self.current_index = 0;
    }

    pub fn current_index(&self) -> usize {
        self.current_index
    }

    pub fn config(&self) -> &RoutingConfig {
        &self.config
    }

    /// Synchronize router runtime state with updated config models/routing.
    /// Keeps any explicit model override intact.
    pub fn sync_runtime(&mut self, primary: String, fallbacks: Vec<String>, config: RoutingConfig) {
        self.primary = primary;
        self.fallbacks = fallbacks;
        self.config = config;
        self.current_index = 0;
    }
}

/// Estimate the cost of a request for a given model.
fn estimate_cost(
    model: &str,
    registry: &ProviderRegistry,
    input_tokens: u32,
    output_tokens: u32,
) -> f64 {
    match registry.get_by_model(model) {
        Some(provider) => {
            provider.cost_per_input_token * input_tokens as f64
                + provider.cost_per_output_token * output_tokens as f64
        }
        None => f64::MAX,
    }
}

/// Heuristic fallback when no provider registry is available.
pub fn is_local_model_heuristic(model: &str) -> bool {
    model.starts_with("ollama/") || model.starts_with("local/") || model.starts_with("llama-cpp/")
}

/// Build a feature vector for ML-based routing.
/// Returns [message_length_feature, tool_call_feature, depth_feature].
pub fn extract_features(
    message: &str,
    tool_call_count: usize,
    conversation_depth: usize,
) -> Vec<f32> {
    vec![
        message.len() as f32,
        tool_call_count as f32,
        conversation_depth as f32,
    ]
}

/// Simple heuristic complexity score in [0.0, 1.0].
/// Weighted sum: msg_len/1000 * 0.3 + tool_calls/5 * 0.3 + depth/10 * 0.4
/// Kept for backward compatibility; delegates to HeuristicBackend.
pub fn classify_complexity(features: &[f32]) -> f64 {
    HeuristicBackend.classify_complexity(features)
}

fn heuristic_classify_complexity(features: &[f32]) -> f64 {
    if features.len() < 3 {
        return 0.0;
    }
    let msg_component = (features[0] as f64 / 1000.0) * 0.3;
    let tool_component = (features[1] as f64 / 5.0) * 0.3;
    let depth_component = (features[2] as f64 / 10.0) * 0.4;
    (msg_component + tool_component + depth_component).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::provider::Provider;

    fn test_config() -> RoutingConfig {
        RoutingConfig::default()
    }

    #[test]
    fn select_primary_model() {
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );
        assert_eq!(router.select_model(), "ollama/qwen3:8b");
        assert_eq!(router.current_index(), 0);
    }

    #[test]
    fn advance_through_fallbacks() {
        let mut router = ModelRouter::new(
            "primary".into(),
            vec!["fallback1".into(), "fallback2".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );

        let first = router.advance_fallback().unwrap();
        assert_eq!(first, "fallback1");
        assert_eq!(router.current_index(), 1);

        let second = router.advance_fallback().unwrap();
        assert_eq!(second, "fallback2");
        assert_eq!(router.current_index(), 2);
    }

    #[test]
    fn exhaustion_error() {
        let mut router = ModelRouter::new(
            "primary".into(),
            vec!["fallback1".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );

        router.advance_fallback().unwrap();
        let err = router.advance_fallback().unwrap_err();
        assert!(err.to_string().contains("exhausted"));

        router.reset();
        assert_eq!(router.current_index(), 0);
        assert_eq!(router.select_model(), "primary");
    }

    #[test]
    fn complexity_classification() {
        let features = extract_features("hello world", 0, 0);
        let score = classify_complexity(&features);
        assert!((0.0..=1.0).contains(&score));
        assert!(
            score < 0.1,
            "short message with no tools should be low complexity"
        );

        let long_msg = "x".repeat(2000);
        let heavy = extract_features(&long_msg, 5, 10);
        let heavy_score = classify_complexity(&heavy);
        assert!(
            heavy_score > 0.5,
            "long message with many tools should be high complexity"
        );

        let maxed = extract_features(&"x".repeat(5000), 10, 20);
        let maxed_score = classify_complexity(&maxed);
        assert!(
            (maxed_score - 1.0).abs() < f64::EPSILON,
            "should clamp to 1.0"
        );
    }

    // 9C: Edge case — empty features array
    #[test]
    fn classify_complexity_empty_features_returns_zero() {
        let score = classify_complexity(&[]);
        assert!((score - 0.0).abs() < f64::EPSILON);
        let score_short = classify_complexity(&[1.0]);
        assert!((score_short - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn primary_mode_always_selects_primary() {
        let config = RoutingConfig {
            mode: "primary".into(),
            ..Default::default()
        };
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            config,
            Box::new(HeuristicBackend),
        );
        assert_eq!(router.select_model(), "ollama/qwen3:8b");
        assert_eq!(
            router.select_for_complexity(0.99, None, None, None),
            "ollama/qwen3:8b"
        );
    }

    #[test]
    fn round_robin_mode_cycles() {
        let config = RoutingConfig {
            mode: "round-robin".into(),
            ..Default::default()
        };
        let mut router = ModelRouter::new(
            "primary".into(),
            vec!["fb1".into(), "fb2".into()],
            config,
            Box::new(HeuristicBackend),
        );

        assert_eq!(router.select_model(), "primary");
        router.current_index = 1;
        assert_eq!(router.select_model(), "fb1");
        router.current_index = 2;
        assert_eq!(router.select_model(), "fb2");
        router.current_index = 3;
        assert_eq!(router.select_model(), "primary");
    }

    #[test]
    fn ml_mode_complexity_routing() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.7,
            local_first: true,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            config,
            Box::new(HeuristicBackend),
        );

        assert_eq!(
            router.select_for_complexity(0.3, None, None, None),
            "ollama/qwen3:8b",
            "low complexity should use local primary"
        );

        assert_eq!(
            router.select_for_complexity(0.9, None, None, None),
            "openai/gpt-4o",
            "high complexity should promote to fallback"
        );
    }

    #[test]
    fn local_first_prefers_local_model() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.5,
            local_first: true,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "ollama/local-model".into(),
            vec!["cloud/expensive".into()],
            config,
            Box::new(HeuristicBackend),
        );

        assert_eq!(
            router.select_for_complexity(0.4, None, None, None),
            "ollama/local-model",
        );
    }

    #[test]
    fn registry_overrides_heuristic_local_detection() {
        use crate::provider::{Provider, ProviderRegistry};
        use ironclad_core::{ApiFormat, ModelTier};
        use std::collections::HashMap;

        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.5,
            local_first: true,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "custom/model".into(),
            vec!["cloud/fallback".into()],
            config,
            Box::new(HeuristicBackend),
        );

        // Without registry, "custom/model" is not local by heuristic
        assert_eq!(
            router.select_for_complexity(0.3, None, None, None),
            "custom/model"
        );

        // With registry marking it as local
        let mut reg = ProviderRegistry::new();
        reg.register(Provider {
            name: "custom".into(),
            url: "http://localhost:5000".into(),
            tier: ModelTier::T1,
            api_key_env: "CUSTOM_API_KEY".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        assert_eq!(
            router.select_for_complexity(0.3, Some(&reg), None, None),
            "custom/model",
            "registry is_local=true keeps it on primary"
        );
    }

    #[test]
    fn config_accessor() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.85,
            local_first: false,
            ..Default::default()
        };
        let router = ModelRouter::new("p".into(), vec![], config, Box::new(HeuristicBackend));
        assert_eq!(router.config().mode, "ml");
        assert!((router.config().confidence_threshold - 0.85).abs() < f64::EPSILON);
        assert!(!router.config().local_first);
    }

    #[test]
    fn is_local_model_detection() {
        assert!(is_local_model_heuristic("ollama/qwen3:8b"));
        assert!(is_local_model_heuristic("local/my-model"));
        assert!(is_local_model_heuristic("llama-cpp/gguf-model"));
        assert!(!is_local_model_heuristic("openai/gpt-4o"));
        assert!(!is_local_model_heuristic("anthropic/claude"));
    }

    fn cost_test_registry() -> ProviderRegistry {
        use ironclad_core::{ApiFormat, ModelTier};

        let mut reg = ProviderRegistry::new();
        reg.register(Provider {
            name: "cheap".into(),
            url: "http://cheap.example.com".into(),
            tier: ModelTier::T2,
            api_key_env: "CHEAP_KEY".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: false,
            cost_per_input_token: 0.00001,
            cost_per_output_token: 0.00002,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg.register(Provider {
            name: "expensive".into(),
            url: "http://expensive.example.com".into(),
            tier: ModelTier::T3,
            api_key_env: "EXPENSIVE_KEY".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: false,
            cost_per_input_token: 0.001,
            cost_per_output_token: 0.002,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
            tpm_limit: Some(100_000),
            rpm_limit: Some(60),
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg.register(Provider {
            name: "ollama".into(),
            url: "http://localhost:11434".into(),
            tier: ModelTier::T1,
            api_key_env: "OLLAMA_KEY".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            embedding_path: None,
            embedding_model: None,
            embedding_dimensions: None,
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
            tpm_limit: None,
            rpm_limit: None,
            auth_mode: "api_key".into(),
            oauth_client_id: None,
            api_key_ref: None,
        });
        reg
    }

    #[test]
    fn select_cheapest_qualified_prefers_cheaper() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.9,
            local_first: false,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "expensive/gpt-5".into(),
            vec!["cheap/lite-model".into()],
            config,
            Box::new(HeuristicBackend),
        );
        let reg = cost_test_registry();

        let selected = router.select_cheapest_qualified(0.3, &reg, None, None, 1000, 500);
        assert_eq!(
            selected, "cheap/lite-model",
            "should pick the cheaper provider for low complexity"
        );
    }

    #[test]
    fn select_cheapest_qualified_skips_near_capacity() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.9,
            local_first: false,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "cheap/lite-model".into(),
            vec!["expensive/gpt-5".into()],
            config,
            Box::new(HeuristicBackend),
        );
        let reg = cost_test_registry();

        let cap = super::super::capacity::CapacityTracker::new(60);
        cap.register("cheap", Some(100), None);
        cap.record("cheap", 95);

        let selected = router.select_cheapest_qualified(0.3, &reg, Some(&cap), None, 1000, 500);
        assert_eq!(
            selected, "expensive/gpt-5",
            "should skip cheap provider when near capacity"
        );
    }

    #[test]
    fn select_cheapest_qualified_high_complexity_prefers_cloud() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.5,
            local_first: false,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "ollama/local-model".into(),
            vec!["cheap/cloud-lite".into(), "expensive/gpt-5".into()],
            config,
            Box::new(HeuristicBackend),
        );
        let reg = cost_test_registry();

        let selected = router.select_cheapest_qualified(0.8, &reg, None, None, 1000, 500);
        assert_eq!(
            selected, "cheap/cloud-lite",
            "high complexity should filter to cloud and pick cheapest"
        );
    }

    #[test]
    fn estimate_cost_unknown_provider() {
        let reg = ProviderRegistry::new();
        let cost = estimate_cost("unknown/model", &reg, 1000, 500);
        assert_eq!(cost, f64::MAX);
    }

    #[test]
    fn estimate_cost_calculates_correctly() {
        let reg = cost_test_registry();
        let cost = estimate_cost("expensive/gpt-5", &reg, 1000, 500);
        let expected = 1000.0 * 0.001 + 500.0 * 0.002;
        assert!(
            (cost - expected).abs() < f64::EPSILON,
            "cost should be {expected}, got {cost}"
        );
    }

    #[test]
    fn model_override_takes_precedence() {
        let mut router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );
        assert_eq!(router.select_model(), "ollama/qwen3:8b");
        assert!(router.get_override().is_none());

        router.set_override("anthropic/claude-sonnet".into());
        assert_eq!(router.select_model(), "anthropic/claude-sonnet");
        assert_eq!(router.get_override(), Some("anthropic/claude-sonnet"));
    }

    #[test]
    fn model_override_applies_to_complexity_routing() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.5,
            local_first: true,
            ..Default::default()
        };
        let mut router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            config,
            Box::new(HeuristicBackend),
        );

        router.set_override("anthropic/claude-sonnet".into());
        assert_eq!(
            router.select_for_complexity(0.1, None, None, None),
            "anthropic/claude-sonnet",
        );
        assert_eq!(
            router.select_for_complexity(0.99, None, None, None),
            "anthropic/claude-sonnet",
        );
    }

    #[test]
    fn model_override_applies_to_cheapest_qualified() {
        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.9,
            local_first: false,
            ..Default::default()
        };
        let mut router = ModelRouter::new(
            "expensive/gpt-5".into(),
            vec!["cheap/lite-model".into()],
            config,
            Box::new(HeuristicBackend),
        );
        let reg = cost_test_registry();

        router.set_override("ollama/override".into());
        assert_eq!(
            router.select_cheapest_qualified(0.3, &reg, None, None, 1000, 500),
            "ollama/override",
        );
    }

    #[test]
    fn clear_override_restores_normal_routing() {
        let mut router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );

        router.set_override("anthropic/claude-sonnet".into());
        assert_eq!(router.select_model(), "anthropic/claude-sonnet");

        router.clear_override();
        assert_eq!(router.select_model(), "ollama/qwen3:8b");
        assert!(router.get_override().is_none());
    }

    #[test]
    fn primary_and_fallbacks_accessors() {
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into(), "anthropic/claude".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );
        assert_eq!(router.primary(), "ollama/qwen3:8b");
        assert_eq!(router.fallbacks(), &["openai/gpt-4o", "anthropic/claude"]);
    }

    #[test]
    fn complexity_routing_skips_blocked_provider() {
        use crate::circuit::CircuitBreakerRegistry;
        use ironclad_core::config::CircuitBreakerConfig;

        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.5,
            local_first: true,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into(), "anthropic/claude".into()],
            config,
            Box::new(HeuristicBackend),
        );

        let cb_config = CircuitBreakerConfig {
            threshold: 1,
            window_seconds: 60,
            cooldown_seconds: 300,
            credit_cooldown_seconds: 300,
            max_cooldown_seconds: 900,
        };
        let mut breakers = CircuitBreakerRegistry::new(&cb_config);

        // High complexity normally selects first fallback (openai)
        assert_eq!(
            router.select_for_complexity(0.9, None, None, Some(&breakers)),
            "openai/gpt-4o"
        );

        // Block openai — should skip to anthropic
        breakers.record_credit_error("openai");
        assert_eq!(
            router.select_for_complexity(0.9, None, None, Some(&breakers)),
            "anthropic/claude"
        );

        // Block anthropic too — falls back to primary
        breakers.record_credit_error("anthropic");
        assert_eq!(
            router.select_for_complexity(0.9, None, None, Some(&breakers)),
            "ollama/qwen3:8b"
        );
    }

    #[test]
    fn cheapest_routing_filters_blocked_providers() {
        use crate::circuit::CircuitBreakerRegistry;
        use ironclad_core::config::CircuitBreakerConfig;

        let config = RoutingConfig {
            mode: "ml".into(),
            confidence_threshold: 0.9,
            local_first: false,
            ..Default::default()
        };
        let router = ModelRouter::new(
            "expensive/gpt-5".into(),
            vec!["cheap/lite-model".into()],
            config,
            Box::new(HeuristicBackend),
        );
        let reg = cost_test_registry();

        let cb_config = CircuitBreakerConfig {
            threshold: 1,
            window_seconds: 60,
            cooldown_seconds: 300,
            credit_cooldown_seconds: 300,
            max_cooldown_seconds: 900,
        };
        let mut breakers = CircuitBreakerRegistry::new(&cb_config);

        // Normally selects cheap
        assert_eq!(
            router.select_cheapest_qualified(0.3, &reg, None, Some(&breakers), 1000, 500),
            "cheap/lite-model"
        );

        // Block cheap — falls back to expensive
        breakers.record_credit_error("cheap");
        assert_eq!(
            router.select_cheapest_qualified(0.3, &reg, None, Some(&breakers), 1000, 500),
            "expensive/gpt-5"
        );
    }
}
