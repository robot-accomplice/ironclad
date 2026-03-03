use ironclad_core::config::RoutingConfig;
use ironclad_core::{IroncladError, Result};

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

/// Build a feature vector for complexity scoring.
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
    fn model_override_takes_precedence() {
        let mut router = ModelRouter::new(
            "ollama/qwen3:8b".into(),
            vec!["openai/gpt-4o".into()],
            test_config(),
            Box::new(HeuristicBackend),
        );

        router.set_override("anthropic/claude-sonnet".into());
        assert_eq!(router.select_model(), "anthropic/claude-sonnet");
        assert_eq!(router.get_override(), Some("anthropic/claude-sonnet"));

        router.clear_override();
        assert_eq!(router.select_model(), "ollama/qwen3:8b");
    }

    #[test]
    fn complexity_classification() {
        let features = extract_features("hello world", 0, 0);
        let score = classify_complexity(&features);
        assert!((0.0..=1.0).contains(&score));

        let long_msg = "x".repeat(2000);
        let heavy = extract_features(&long_msg, 5, 10);
        let heavy_score = classify_complexity(&heavy);
        assert!(heavy_score > 0.5);
    }
}
