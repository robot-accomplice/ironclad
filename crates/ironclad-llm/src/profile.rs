//! Per-model composite profiles and metascore computation.
//!
//! A [`ModelProfile`] captures both static attributes (from provider config)
//! and dynamic observations (from runtime tracking). The [`select_by_metascore`] method
//! produces a transparent, weighted score suitable for routing decisions.

use ironclad_core::ModelTier;
use serde::Serialize;

use crate::accuracy::QualityTracker;
use crate::capacity::CapacityTracker;
use crate::circuit::CircuitBreakerRegistry;
use crate::provider::ProviderRegistry;
use crate::router::ModelRouter;

/// Composite profile for a single model, combining static config with runtime
/// observations. Built on-demand from the current system state.
#[derive(Debug, Clone, Serialize)]
pub struct ModelProfile {
    pub model_name: String,
    /// Static: whether the provider is local (e.g. Ollama, llama.cpp).
    pub is_local: bool,
    /// Static: per-token cost from provider config.
    pub cost_per_input_token: f64,
    pub cost_per_output_token: f64,
    /// Static: provider tier (T1–T4).
    pub tier: ModelTier,
    /// Dynamic: estimated quality from sliding-window observations [0, 1].
    /// None if no observations yet (cold-start).
    pub estimated_quality: Option<f64>,
    /// Dynamic: circuit breaker health [0 = open/blocked, 1 = healthy].
    pub availability: f64,
    /// Dynamic: capacity headroom — fraction of TPM budget remaining [0, 1].
    pub capacity_headroom: f64,
    /// Dynamic: number of quality observations recorded.
    pub observation_count: usize,
}

/// Transparent breakdown of the metascore computation.
#[derive(Debug, Clone, Serialize)]
pub struct MetascoreBreakdown {
    /// Quality/efficacy dimension [0, 1].
    pub efficacy: f64,
    /// Normalized cost score (1.0 = free, 0.0 = very expensive) [0, 1].
    pub cost: f64,
    /// Availability × capacity headroom [0, 1].
    pub availability: f64,
    /// Locality preference adjusted for task complexity [0, 1].
    pub locality: f64,
    /// Confidence penalty for cold-start models (few observations).
    pub confidence: f64,
    /// Weighted final score.
    pub final_score: f64,
}

/// Default quality prior for models with no observations yet.
/// Slightly pessimistic to encourage routing to observed models.
const DEFAULT_QUALITY_PRIOR: f64 = 0.5;

/// Observation count threshold below which a confidence penalty applies.
const CONFIDENCE_THRESHOLD: usize = 10;

impl ModelProfile {
    /// Compute the metascore for this model given a task complexity [0, 1]
    /// and whether cost-awareness is active.
    ///
    /// Returns a breakdown with component scores and the weighted final score.
    pub fn metascore(&self, complexity: f64, cost_aware: bool) -> MetascoreBreakdown {
        self.metascore_with_cost_weight(complexity, cost_aware, None)
    }

    /// Compute the metascore with an explicit cost-weight override.
    ///
    /// When `cost_weight_override` is `Some(w)`, it directly sets the cost
    /// weight in \[0.0, 1.0\] and efficacy absorbs the complementary share.
    /// When `None`, falls back to the `cost_aware` boolean behavior.
    pub fn metascore_with_cost_weight(
        &self,
        complexity: f64,
        cost_aware: bool,
        cost_weight_override: Option<f64>,
    ) -> MetascoreBreakdown {
        // Efficacy: use observed quality or default prior.
        let raw_quality = self.estimated_quality.unwrap_or(DEFAULT_QUALITY_PRIOR);
        let efficacy = raw_quality.clamp(0.0, 1.0);

        // Cost: sigmoid-normalized inverse. Free → 1.0, expensive → ~0.0.
        // Combined per-1k cost = (in + out) * 1000.
        let combined_per_1k = (self.cost_per_input_token + self.cost_per_output_token) * 1000.0;
        let cost = 1.0 / (1.0 + combined_per_1k * 20.0);

        // Availability: product of breaker health and capacity headroom.
        let availability = self.availability * self.capacity_headroom;

        // Locality: local models get a bonus for simple tasks, cloud for complex.
        let locality = if self.is_local {
            (1.0 - complexity * 0.4).max(0.0) // local advantage fades with complexity
        } else {
            (complexity * 0.4).min(1.0) // cloud advantage grows with complexity
        };

        // Confidence: penalize cold-start models with few observations.
        let confidence = if self.observation_count >= CONFIDENCE_THRESHOLD {
            1.0
        } else {
            0.6 + 0.4 * (self.observation_count as f64 / CONFIDENCE_THRESHOLD as f64)
        };

        // Weight selection.
        // If a continuous cost_weight_override is provided, it controls the
        // cost/efficacy tradeoff directly. Availability and locality keep
        // fixed shares; efficacy absorbs whatever cost gives up.
        let (w_eff, w_cost, w_avail, w_local) = if let Some(cw) = cost_weight_override {
            let cw = cw.clamp(0.0, 1.0);
            // Reserve 0.30 for availability + 0.10 for locality = 0.40 fixed.
            // Remaining 0.60 split between efficacy and cost per the weight.
            let w_cost = 0.60 * cw;
            let w_eff = 0.60 * (1.0 - cw);
            (w_eff, w_cost, 0.30, 0.10)
        } else if cost_aware {
            (0.35, 0.30, 0.25, 0.10)
        } else {
            (0.45, 0.15, 0.30, 0.10)
        };

        let raw_score =
            w_eff * efficacy + w_cost * cost + w_avail * availability + w_local * locality;
        let final_score = raw_score * confidence;

        MetascoreBreakdown {
            efficacy,
            cost,
            availability,
            locality,
            confidence,
            final_score,
        }
    }
}

/// Build model profiles for all candidate models from the current system state.
///
/// Iterates the router's primary + fallback models, looks up each model's
/// provider, and combines static config (cost, tier, locality) with dynamic
/// observations (quality, capacity, breakers). Models with no matching
/// provider are silently skipped.
pub fn build_model_profiles(
    router: &ModelRouter,
    providers: &ProviderRegistry,
    quality: &QualityTracker,
    capacity: &CapacityTracker,
    breakers: &CircuitBreakerRegistry,
) -> Vec<ModelProfile> {
    let mut profiles = Vec::new();

    // Collect all candidate model names: primary + fallbacks.
    let mut candidates = vec![router.primary().to_string()];
    candidates.extend(router.fallbacks().iter().cloned());

    for model_name in &candidates {
        let provider = match providers.get_by_model(model_name) {
            Some(p) => p,
            None => continue, // no provider configured for this model
        };

        let prefix = model_name.split('/').next().unwrap_or("unknown");

        let is_blocked = breakers.is_blocked(prefix);
        let availability = if is_blocked { 0.0 } else { 1.0 };

        let headroom = capacity.headroom(prefix);
        let estimated_quality = quality.estimated_quality(model_name);
        let observation_count = quality.observation_count(model_name);

        profiles.push(ModelProfile {
            model_name: model_name.clone(),
            is_local: provider.is_local,
            cost_per_input_token: provider.cost_per_input_token,
            cost_per_output_token: provider.cost_per_output_token,
            tier: provider.tier,
            estimated_quality,
            availability,
            capacity_headroom: headroom,
            observation_count,
        });
    }

    profiles
}

/// Select the best model from profiles using metascore ranking.
///
/// Returns the model name and its full breakdown, or None if no models are
/// available (all blocked or empty registry).
pub fn select_by_metascore(
    profiles: &[ModelProfile],
    complexity: f64,
    cost_aware: bool,
) -> Option<(String, MetascoreBreakdown)> {
    profiles
        .iter()
        .filter(|p| p.availability > 0.0)
        .map(|p| {
            let breakdown = p.metascore(complexity, cost_aware);
            (p.model_name.clone(), breakdown)
        })
        .max_by(|a, b| {
            a.1.final_score
                .partial_cmp(&b.1.final_score)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local_profile(quality: Option<f64>, obs: usize) -> ModelProfile {
        ModelProfile {
            model_name: "ollama/qwen3:8b".into(),
            is_local: true,
            cost_per_input_token: 0.0,
            cost_per_output_token: 0.0,
            tier: ModelTier::T1,
            estimated_quality: quality,
            availability: 1.0,
            capacity_headroom: 1.0,
            observation_count: obs,
        }
    }

    fn cloud_profile(quality: Option<f64>, obs: usize) -> ModelProfile {
        ModelProfile {
            model_name: "openai/gpt-4o".into(),
            is_local: false,
            cost_per_input_token: 0.0025,
            cost_per_output_token: 0.01,
            tier: ModelTier::T3,
            estimated_quality: quality,
            availability: 1.0,
            capacity_headroom: 1.0,
            observation_count: obs,
        }
    }

    #[test]
    fn metascore_local_simple_task() {
        let profile = local_profile(Some(0.8), 50);
        let breakdown = profile.metascore(0.1, false);
        assert!(
            breakdown.final_score > 0.5,
            "local model with good quality on simple task: {}",
            breakdown.final_score
        );
        assert!(
            breakdown.locality > 0.5,
            "local should have high locality on simple task"
        );
    }

    #[test]
    fn metascore_cloud_complex_task() {
        let profile = cloud_profile(Some(0.9), 50);
        let breakdown = profile.metascore(0.9, false);
        assert!(
            breakdown.final_score > 0.4,
            "cloud model with high quality on complex task: {}",
            breakdown.final_score
        );
        assert!(
            breakdown.locality > 0.2,
            "cloud should have higher locality on complex task"
        );
    }

    #[test]
    fn metascore_cold_start_penalty() {
        let cold = local_profile(None, 0);
        let warm = local_profile(Some(0.7), 20);
        let cold_score = cold.metascore(0.5, false);
        let warm_score = warm.metascore(0.5, false);
        assert!(
            cold_score.final_score < warm_score.final_score,
            "cold-start should score lower: cold={} warm={}",
            cold_score.final_score,
            warm_score.final_score
        );
        assert!(
            cold_score.confidence < 1.0,
            "cold-start confidence penalty should apply"
        );
    }

    #[test]
    fn metascore_blocked_model_filtered() {
        let blocked = ModelProfile {
            availability: 0.0,
            ..local_profile(Some(0.9), 50)
        };
        let profiles = vec![blocked, cloud_profile(Some(0.7), 30)];
        let result = select_by_metascore(&profiles, 0.5, false);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "openai/gpt-4o");
    }

    #[test]
    fn metascore_cost_aware_prefers_cheap() {
        let local = local_profile(Some(0.7), 30);
        let cloud = cloud_profile(Some(0.75), 30);
        let profiles = vec![local, cloud];

        let normal = select_by_metascore(&profiles, 0.3, false);
        let cost = select_by_metascore(&profiles, 0.3, true);

        // Both should pick local for a simple task, but the cost-aware selection
        // should have a wider gap due to the free local model's cost advantage.
        assert_eq!(normal.as_ref().unwrap().0, "ollama/qwen3:8b");
        assert_eq!(cost.as_ref().unwrap().0, "ollama/qwen3:8b");
    }

    #[test]
    fn metascore_empty_profiles() {
        let result = select_by_metascore(&[], 0.5, false);
        assert!(result.is_none());
    }

    #[test]
    fn metascore_all_blocked() {
        let profiles = vec![
            ModelProfile {
                availability: 0.0,
                ..local_profile(Some(0.9), 50)
            },
            ModelProfile {
                availability: 0.0,
                ..cloud_profile(Some(0.9), 50)
            },
        ];
        let result = select_by_metascore(&profiles, 0.5, false);
        assert!(result.is_none());
    }

    #[test]
    fn metascore_deterministic_tiebreak() {
        // Two identical profiles — ensure stable ordering
        let a = ModelProfile {
            model_name: "alpha/model".into(),
            ..local_profile(Some(0.7), 30)
        };
        let b = ModelProfile {
            model_name: "beta/model".into(),
            ..local_profile(Some(0.7), 30)
        };

        let result1 = select_by_metascore(&[a.clone(), b.clone()], 0.5, false);
        let result2 = select_by_metascore(&[a, b], 0.5, false);
        assert_eq!(result1.unwrap().0, result2.unwrap().0);
    }

    #[test]
    fn metascore_breakdown_components_bounded() {
        let profile = cloud_profile(Some(0.85), 25);
        let b = profile.metascore(0.5, true);
        assert!((0.0..=1.0).contains(&b.efficacy));
        assert!((0.0..=1.0).contains(&b.cost));
        assert!((0.0..=1.0).contains(&b.availability));
        assert!((0.0..=1.0).contains(&b.locality));
        assert!((0.0..=1.0).contains(&b.confidence));
        assert!(b.final_score >= 0.0, "final score should be non-negative");
    }
}
