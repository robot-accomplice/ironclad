use std::collections::HashMap;
use tracing::{debug, info};

/// Feature vector representing a model's capabilities.
#[derive(Debug, Clone)]
pub struct ModelVector {
    pub model_name: String,
    pub context_window: f64,
    pub cost_per_1k_input: f64,
    pub cost_per_1k_output: f64,
    pub quality_score: f64,
    pub speed_score: f64,
    pub supports_vision: bool,
    pub supports_tools: bool,
    pub supports_streaming: bool,
}

impl ModelVector {
    /// Convert to a normalized feature array for similarity computation.
    pub fn to_features(&self) -> Vec<f64> {
        vec![
            self.context_window / 200_000.0,
            1.0 - (self.cost_per_1k_input * 100.0).min(1.0),
            1.0 - (self.cost_per_1k_output * 100.0).min(1.0),
            self.quality_score,
            self.speed_score,
            if self.supports_vision { 1.0 } else { 0.0 },
            if self.supports_tools { 1.0 } else { 0.0 },
            if self.supports_streaming { 1.0 } else { 0.0 },
        ]
    }
}

/// Query requirements vector for model selection.
#[derive(Debug, Clone)]
pub struct QueryRequirements {
    pub min_context: f64,
    pub max_cost_per_1k: f64,
    pub min_quality: f64,
    pub speed_priority: f64,
    pub needs_vision: bool,
    pub needs_tools: bool,
    pub needs_streaming: bool,
}

impl QueryRequirements {
    pub fn to_features(&self) -> Vec<f64> {
        vec![
            self.min_context / 200_000.0,
            1.0 - (self.max_cost_per_1k * 100.0).min(1.0),
            1.0 - (self.max_cost_per_1k * 100.0).min(1.0),
            self.min_quality,
            self.speed_priority,
            if self.needs_vision { 1.0 } else { 0.0 },
            if self.needs_tools { 1.0 } else { 0.0 },
            if self.needs_streaming { 1.0 } else { 0.0 },
        ]
    }
}

/// Registry of model vectors for similarity-based selection.
pub struct ModelVectorRegistry {
    models: HashMap<String, ModelVector>,
}

impl ModelVectorRegistry {
    pub fn new() -> Self {
        Self {
            models: HashMap::new(),
        }
    }

    pub fn register(&mut self, model: ModelVector) {
        debug!(name = %model.model_name, "registered model vector");
        self.models.insert(model.model_name.clone(), model);
    }

    pub fn get(&self, name: &str) -> Option<&ModelVector> {
        self.models.get(name)
    }

    /// Select the best model for the given requirements using cosine similarity.
    pub fn select_best(&self, requirements: &QueryRequirements) -> Option<&ModelVector> {
        let req_features = requirements.to_features();

        let mut best: Option<(&ModelVector, f64)> = None;

        for model in self.models.values() {
            if !meets_hard_constraints(model, requirements) {
                continue;
            }

            let model_features = model.to_features();
            let similarity = cosine_similarity(&req_features, &model_features);

            if !best.as_ref().is_some_and(|(_, s)| similarity <= *s) {
                best = Some((model, similarity));
            }
        }

        if let Some((model, sim)) = &best {
            info!(selected = %model.model_name, similarity = sim, "UniRoute selected model");
        }

        best.map(|(m, _)| m)
    }

    /// Rank all models by similarity to requirements.
    pub fn rank(&self, requirements: &QueryRequirements) -> Vec<(&ModelVector, f64)> {
        let req_features = requirements.to_features();

        let mut ranked: Vec<(&ModelVector, f64)> = self
            .models
            .values()
            .map(|m| {
                let sim = cosine_similarity(&req_features, &m.to_features());
                (m, sim)
            })
            .collect();

        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        ranked
    }

    pub fn count(&self) -> usize {
        self.models.len()
    }
}

impl Default for ModelVectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

fn meets_hard_constraints(model: &ModelVector, req: &QueryRequirements) -> bool {
    if req.needs_vision && !model.supports_vision {
        return false;
    }
    if req.needs_tools && !model.supports_tools {
        return false;
    }
    if req.needs_streaming && !model.supports_streaming {
        return false;
    }
    if model.context_window < req.min_context {
        return false;
    }
    if model.quality_score < req.min_quality {
        return false;
    }
    if req.max_cost_per_1k > 0.0 && model.cost_per_1k_input > req.max_cost_per_1k {
        return false;
    }
    true
}

fn cosine_similarity(a: &[f64], b: &[f64]) -> f64 {
    let dot: f64 = a.iter().zip(b.iter()).map(|(x, y)| x * y).sum();
    let mag_a: f64 = a.iter().map(|x| x * x).sum::<f64>().sqrt();
    let mag_b: f64 = b.iter().map(|x| x * x).sum::<f64>().sqrt();

    if mag_a == 0.0 || mag_b == 0.0 {
        return 0.0;
    }

    dot / (mag_a * mag_b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpt4_vector() -> ModelVector {
        ModelVector {
            model_name: "openai/gpt-4o".into(),
            context_window: 128_000.0,
            cost_per_1k_input: 0.005,
            cost_per_1k_output: 0.015,
            quality_score: 0.9,
            speed_score: 0.7,
            supports_vision: true,
            supports_tools: true,
            supports_streaming: true,
        }
    }

    fn local_vector() -> ModelVector {
        ModelVector {
            model_name: "ollama/qwen3:8b".into(),
            context_window: 32_000.0,
            cost_per_1k_input: 0.0,
            cost_per_1k_output: 0.0,
            quality_score: 0.6,
            speed_score: 0.95,
            supports_vision: false,
            supports_tools: true,
            supports_streaming: true,
        }
    }

    #[test]
    fn cosine_similarity_identical() {
        let a = vec![1.0, 0.0, 0.0];
        let sim = cosine_similarity(&a, &a);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = vec![1.0, 0.0];
        let b = vec![0.0, 1.0];
        let sim = cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_zero_vector() {
        let a = vec![0.0, 0.0];
        let b = vec![1.0, 1.0];
        assert!((cosine_similarity(&a, &b) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn model_vector_features() {
        let v = gpt4_vector();
        let f = v.to_features();
        assert_eq!(f.len(), 8);
        assert!(f[0] > 0.0 && f[0] <= 1.0);
    }

    #[test]
    fn registry_select_best() {
        let mut reg = ModelVectorRegistry::new();
        reg.register(gpt4_vector());
        reg.register(local_vector());

        let req = QueryRequirements {
            min_context: 10_000.0,
            max_cost_per_1k: 0.01,
            min_quality: 0.5,
            speed_priority: 0.9,
            needs_vision: false,
            needs_tools: true,
            needs_streaming: true,
        };

        let best = reg.select_best(&req);
        assert!(best.is_some());
    }

    #[test]
    fn registry_vision_filter() {
        let mut reg = ModelVectorRegistry::new();
        reg.register(gpt4_vector());
        reg.register(local_vector());

        let req = QueryRequirements {
            min_context: 1_000.0,
            max_cost_per_1k: 1.0,
            min_quality: 0.5,
            speed_priority: 0.5,
            needs_vision: true,
            needs_tools: false,
            needs_streaming: false,
        };

        let best = reg.select_best(&req).unwrap();
        assert_eq!(best.model_name, "openai/gpt-4o");
    }

    #[test]
    fn registry_rank() {
        let mut reg = ModelVectorRegistry::new();
        reg.register(gpt4_vector());
        reg.register(local_vector());

        let req = QueryRequirements {
            min_context: 1_000.0,
            max_cost_per_1k: 1.0,
            min_quality: 0.5,
            speed_priority: 0.5,
            needs_vision: false,
            needs_tools: false,
            needs_streaming: false,
        };

        let ranked = reg.rank(&req);
        assert_eq!(ranked.len(), 2);
        assert!(ranked[0].1 >= ranked[1].1);
    }

    #[test]
    fn empty_registry() {
        let reg = ModelVectorRegistry::new();
        let req = QueryRequirements {
            min_context: 0.0,
            max_cost_per_1k: 1.0,
            min_quality: 0.0,
            speed_priority: 0.5,
            needs_vision: false,
            needs_tools: false,
            needs_streaming: false,
        };
        assert!(reg.select_best(&req).is_none());
    }
}
