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
        }
    }

    /// Classify complexity from a feature vector using the configured backend.
    pub fn classify_complexity(&self, features: &[f32]) -> f64 {
        self.backend.classify_complexity(features)
    }

    pub fn select_model(&self) -> &str {
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
                    &self.fallbacks[self.current_index - 1]
                }
            }
        }
    }

    /// Select a model using complexity-aware routing, consulting the provider
    /// registry for `is_local` when available.
    pub fn select_for_complexity(&self, complexity: f64, registry: Option<&ProviderRegistry>) -> &str {
        if self.config.mode == "primary" {
            return &self.primary;
        }

        let primary_is_local = match registry {
            Some(reg) => reg.get_by_model(&self.primary).is_some_and(|p| p.is_local),
            None => is_local_model_heuristic(&self.primary),
        };

        if self.config.local_first && primary_is_local
            && complexity < self.config.confidence_threshold {
                return &self.primary;
            }

        if complexity >= self.config.confidence_threshold && !self.fallbacks.is_empty() {
            return &self.fallbacks[0];
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
    use super::*;

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
        assert!(score >= 0.0 && score <= 1.0);
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
        assert_eq!(router.select_for_complexity(0.99, None), "ollama/qwen3:8b");
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
        };
        let router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            config,
            Box::new(HeuristicBackend),
        );

        assert_eq!(
            router.select_for_complexity(0.3, None),
            "ollama/qwen3:8b",
            "low complexity should use local primary"
        );

        assert_eq!(
            router.select_for_complexity(0.9, None),
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
        };
        let router = ModelRouter::new(
            "ollama/local-model".into(),
            vec!["cloud/expensive".into()],
            config,
            Box::new(HeuristicBackend),
        );

        assert_eq!(
            router.select_for_complexity(0.4, None),
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
        };
        let router = ModelRouter::new(
            "custom/model".into(),
            vec!["cloud/fallback".into()],
            config,
            Box::new(HeuristicBackend),
        );

        // Without registry, "custom/model" is not local by heuristic
        assert_eq!(router.select_for_complexity(0.3, None), "custom/model");

        // With registry marking it as local
        let mut reg = ProviderRegistry::new();
        reg.register(Provider {
            name: "custom".into(),
            url: "http://localhost:5000".into(),
            tier: ModelTier::T1,
            api_key_env: "CUSTOM_API_KEY".into(),
            format: ApiFormat::OpenAiCompletions,
            chat_path: "/v1/chat/completions".into(),
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        });
        assert_eq!(
            router.select_for_complexity(0.3, Some(&reg)),
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
}
