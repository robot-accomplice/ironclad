use serde::{Deserialize, Serialize};

pub use ironclad_db::efficiency::{
    RecommendationModelStats as ModelStats, RecommendationUserProfile as UserProfile,
};

// ── Data types ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum RecommendationCategory {
    QueryCrafting,
    ModelSelection,
    SessionManagement,
    MemoryLeverage,
    CostOptimization,
    ToolUsage,
    Configuration,
}

impl RecommendationCategory {
    pub fn label(&self) -> &'static str {
        match self {
            Self::QueryCrafting => "Query Crafting",
            Self::ModelSelection => "Model Selection",
            Self::SessionManagement => "Session Management",
            Self::MemoryLeverage => "Memory Leverage",
            Self::CostOptimization => "Cost Optimization",
            Self::ToolUsage => "Tool Usage",
            Self::Configuration => "Configuration",
        }
    }

    pub fn icon(&self) -> &'static str {
        match self {
            Self::QueryCrafting => "pencil",
            Self::ModelSelection => "cpu",
            Self::SessionManagement => "chat",
            Self::MemoryLeverage => "memory",
            Self::CostOptimization => "dollar",
            Self::ToolUsage => "wrench",
            Self::Configuration => "gear",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, PartialOrd)]
pub enum Priority {
    Low,
    Medium,
    High,
}

impl Priority {
    fn ordinal(&self) -> u8 {
        match self {
            Self::Low => 0,
            Self::Medium => 1,
            Self::High => 2,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub metric: String,
    pub value: String,
    pub context: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Impact {
    pub monthly_savings: Option<f64>,
    pub quality_change: Option<String>,
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recommendation {
    pub category: RecommendationCategory,
    pub priority: Priority,
    pub title: String,
    pub explanation: String,
    pub action: String,
    pub evidence: Vec<Evidence>,
    pub estimated_impact: Option<Impact>,
}

// ── Rule trait ────────────────────────────────────────────────

pub trait RecommendationRule: Send + Sync {
    fn name(&self) -> &str;
    fn category(&self) -> RecommendationCategory;
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation>;
}

// ── Engine ────────────────────────────────────────────────────

pub struct RecommendationEngine {
    rules: Vec<Box<dyn RecommendationRule>>,
}

impl Default for RecommendationEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl RecommendationEngine {
    pub fn new() -> Self {
        Self {
            rules: vec![
                Box::new(SpecificityCorrelation),
                Box::new(FollowUpPatterns),
                Box::new(ParetoOptimalModels),
                Box::new(ComplexityMismatch),
                Box::new(ModelStrengths),
                Box::new(SessionLengthSweet),
                Box::new(StaleSessionCost),
                Box::new(MemoryUnderutilized),
                Box::new(MemoryOverloaded),
                Box::new(SystemPromptROI),
                Box::new(CachingOpportunity),
                Box::new(ToolCostAwareness),
                Box::new(FallbackChainTuning),
                Box::new(HighCostPerTurn),
            ],
        }
    }

    pub fn generate(&self, profile: &UserProfile) -> Vec<Recommendation> {
        let mut recs: Vec<Recommendation> = self
            .rules
            .iter()
            .filter_map(|r| r.evaluate(profile))
            .collect();
        recs.sort_by(|a, b| b.priority.ordinal().cmp(&a.priority.ordinal()));
        recs
    }
}

// ── Query Crafting rules ──────────────────────────────────────

struct SpecificityCorrelation;

impl RecommendationRule for SpecificityCorrelation {
    fn name(&self) -> &str {
        "specificity_correlation"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::QueryCrafting
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 10 {
            return None;
        }
        if profile.avg_tokens_per_turn < 50.0 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Medium,
                title: "Short queries may reduce response quality".into(),
                explanation: "Your average query is quite short. Longer, more specific prompts \
                    tend to produce higher-quality responses with fewer follow-up turns."
                    .into(),
                action: "Try including context, constraints, and desired format in your queries."
                    .into(),
                evidence: vec![Evidence {
                    metric: "avg_tokens_per_turn".into(),
                    value: format!("{:.0}", profile.avg_tokens_per_turn),
                    context: "tokens per user message (recommended: 80+)".into(),
                }],
                estimated_impact: Some(Impact {
                    monthly_savings: None,
                    quality_change: Some("Potential quality improvement".into()),
                    description: "More specific queries reduce back-and-forth turns".into(),
                }),
            })
        } else {
            None
        }
    }
}

struct FollowUpPatterns;

impl RecommendationRule for FollowUpPatterns {
    fn name(&self) -> &str {
        "follow_up_patterns"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::QueryCrafting
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_sessions < 3 {
            return None;
        }
        if profile.avg_session_length > 20.0 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Medium,
                title: "Sessions with many turns may indicate unclear initial queries".into(),
                explanation: format!(
                    "Your sessions average {:.0} turns. Quality often degrades after 10-15 turns \
                     due to context window pressure. Starting fresh sessions with clear, complete \
                     instructions often yields better results.",
                    profile.avg_session_length
                ),
                action: "Consider starting new sessions instead of extending long conversations."
                    .into(),
                evidence: vec![Evidence {
                    metric: "avg_session_length".into(),
                    value: format!("{:.1}", profile.avg_session_length),
                    context: "average turns per session".into(),
                }],
                estimated_impact: None,
            })
        } else {
            None
        }
    }
}

// ── Model Selection rules ─────────────────────────────────────

struct ParetoOptimalModels;

impl RecommendationRule for ParetoOptimalModels {
    fn name(&self) -> &str {
        "pareto_optimal_models"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::ModelSelection
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.model_stats.len() < 2 {
            return None;
        }
        let mut dominated: Vec<(String, String)> = Vec::new();
        let models: Vec<(&String, &ModelStats)> = profile.model_stats.iter().collect();
        for (i, (name_a, stats_a)) in models.iter().enumerate() {
            for (name_b, stats_b) in models.iter().skip(i + 1) {
                if stats_b.avg_cost <= stats_a.avg_cost
                    && stats_b.avg_output_density >= stats_a.avg_output_density
                    && (stats_b.avg_cost < stats_a.avg_cost
                        || stats_b.avg_output_density > stats_a.avg_output_density)
                {
                    dominated.push(((*name_a).clone(), (*name_b).clone()));
                } else if stats_a.avg_cost <= stats_b.avg_cost
                    && stats_a.avg_output_density >= stats_b.avg_output_density
                    && (stats_a.avg_cost < stats_b.avg_cost
                        || stats_a.avg_output_density > stats_b.avg_output_density)
                {
                    dominated.push(((*name_b).clone(), (*name_a).clone()));
                }
            }
        }

        if dominated.is_empty() {
            return None;
        }

        let (worse, better) = &dominated[0];
        Some(Recommendation {
            category: self.category(),
            priority: Priority::High,
            title: format!("{worse} is dominated by {better} on cost/quality"),
            explanation: format!(
                "{better} is both cheaper and produces higher output density than {worse}. \
                 Consider consolidating traffic to the dominant model."
            ),
            action: format!("Route {worse} traffic to {better} for better cost-efficiency."),
            evidence: vec![
                Evidence {
                    metric: "dominated_model".into(),
                    value: worse.clone(),
                    context: "higher cost AND lower quality".into(),
                },
                Evidence {
                    metric: "dominant_model".into(),
                    value: better.clone(),
                    context: "lower cost AND higher quality".into(),
                },
            ],
            estimated_impact: profile.model_stats.get(worse).map(|s| Impact {
                monthly_savings: Some(s.avg_cost * s.turns as f64 * 0.3),
                quality_change: Some("quality improvement expected".into()),
                description: "Eliminate dominated model usage".into(),
            }),
        })
    }
}

struct ComplexityMismatch;

impl RecommendationRule for ComplexityMismatch {
    fn name(&self) -> &str {
        "complexity_mismatch"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::ModelSelection
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        let expensive_models: Vec<(&String, &ModelStats)> = profile
            .model_stats
            .iter()
            .filter(|(_, s)| s.avg_cost > 0.02 && s.avg_output_density < 0.15)
            .collect();

        if expensive_models.is_empty() {
            return None;
        }

        let (model, stats) = expensive_models[0];
        Some(Recommendation {
            category: self.category(),
            priority: Priority::High,
            title: format!("{model} may be overkill for simple queries"),
            explanation: format!(
                "{model} costs ${:.4}/turn but has low output density ({:.3}), suggesting \
                 queries may be too simple for this model tier.",
                stats.avg_cost, stats.avg_output_density
            ),
            action: "Route simpler queries to a lighter, cheaper model.".into(),
            evidence: vec![
                Evidence {
                    metric: "avg_cost".into(),
                    value: format!("${:.4}", stats.avg_cost),
                    context: format!("per turn for {model}"),
                },
                Evidence {
                    metric: "avg_output_density".into(),
                    value: format!("{:.3}", stats.avg_output_density),
                    context: "low density suggests simple queries".into(),
                },
            ],
            estimated_impact: Some(Impact {
                monthly_savings: Some(stats.avg_cost * stats.turns as f64 * 0.5),
                quality_change: Some("similar quality at lower cost".into()),
                description: "Switch simple queries to cheaper model".into(),
            }),
        })
    }
}

struct ModelStrengths;

impl RecommendationRule for ModelStrengths {
    fn name(&self) -> &str {
        "model_strengths"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::ModelSelection
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.models_used.len() < 2 || profile.total_turns < 20 {
            return None;
        }

        let mut best_density: Option<(&String, f64)> = None;
        let mut best_cost: Option<(&String, f64)> = None;

        for (name, stats) in &profile.model_stats {
            if stats.turns < 5 {
                continue;
            }
            match &best_density {
                None => best_density = Some((name, stats.avg_output_density)),
                Some((_, d)) if stats.avg_output_density > *d => {
                    best_density = Some((name, stats.avg_output_density));
                }
                _ => {}
            }
            match &best_cost {
                None => best_cost = Some((name, stats.avg_cost)),
                Some((_, c)) if stats.avg_cost < *c => {
                    best_cost = Some((name, stats.avg_cost));
                }
                _ => {}
            }
        }

        if let (Some((quality_model, _)), Some((cost_model, _))) = (&best_density, &best_cost)
            && quality_model != cost_model
        {
            return Some(Recommendation {
                category: self.category(),
                priority: Priority::Low,
                title: "Different models excel at different dimensions".into(),
                explanation: format!(
                    "{} produces the highest output density while {} is the most cost-efficient. \
                     Consider routing complex queries to the quality model and simple ones to \
                     the cost model.",
                    quality_model, cost_model
                ),
                action: "Implement complexity-based routing between models.".into(),
                evidence: vec![
                    Evidence {
                        metric: "best_quality_model".into(),
                        value: (*quality_model).clone(),
                        context: "highest output density".into(),
                    },
                    Evidence {
                        metric: "best_cost_model".into(),
                        value: (*cost_model).clone(),
                        context: "lowest cost per turn".into(),
                    },
                ],
                estimated_impact: None,
            });
        }
        None
    }
}

// ── Session Management rules ──────────────────────────────────

struct SessionLengthSweet;

impl RecommendationRule for SessionLengthSweet {
    fn name(&self) -> &str {
        "session_length_sweet"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::SessionManagement
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_sessions < 3 {
            return None;
        }
        if profile.avg_session_length > 15.0 && profile.avg_session_length <= 20.0 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Low,
                title: "Sessions approaching optimal length ceiling".into(),
                explanation: format!(
                    "Average session length is {:.0} turns. Research suggests quality peaks around \
                     8-12 turns before context pressure degrades output.",
                    profile.avg_session_length
                ),
                action:
                    "Monitor quality in longer sessions and consider proactive session splitting."
                        .into(),
                evidence: vec![Evidence {
                    metric: "avg_session_length".into(),
                    value: format!("{:.1}", profile.avg_session_length),
                    context: "turns per session (sweet spot: 8-12)".into(),
                }],
                estimated_impact: None,
            })
        } else {
            None
        }
    }
}

struct StaleSessionCost;

impl RecommendationRule for StaleSessionCost {
    fn name(&self) -> &str {
        "stale_session_cost"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::SessionManagement
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_sessions < 5 || profile.avg_session_length <= 25.0 {
            return None;
        }
        let wasted_token_pct = (profile.avg_session_length - 12.0) / profile.avg_session_length;
        let estimated_waste = profile.total_cost * wasted_token_pct * 0.3;

        Some(Recommendation {
            category: self.category(),
            priority: Priority::High,
            title: "Long sessions accumulate expensive history tokens".into(),
            explanation: format!(
                "Sessions averaging {:.0} turns means each new turn carries ~{:.0}% \
                 redundant history context, driving up input token costs.",
                profile.avg_session_length,
                wasted_token_pct * 100.0
            ),
            action: "Enable automatic session archiving after 12-15 turns and start fresh.".into(),
            evidence: vec![
                Evidence {
                    metric: "avg_session_length".into(),
                    value: format!("{:.0}", profile.avg_session_length),
                    context: "turns per session".into(),
                },
                Evidence {
                    metric: "estimated_waste".into(),
                    value: format!("${:.4}", estimated_waste),
                    context: "estimated wasted cost on stale context".into(),
                },
            ],
            estimated_impact: Some(Impact {
                monthly_savings: Some(estimated_waste),
                quality_change: Some("quality may improve with fresh context".into()),
                description: "Reduce input token waste from stale history".into(),
            }),
        })
    }
}

// ── Memory Leverage rules ─────────────────────────────────────

struct MemoryUnderutilized;

impl RecommendationRule for MemoryUnderutilized {
    fn name(&self) -> &str {
        "memory_underutilized"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::MemoryLeverage
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 20 {
            return None;
        }
        if profile.memory_retrieval_rate < 0.2 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Medium,
                title: "Memory system is underutilized".into(),
                explanation: format!(
                    "Only {:.0}% of turns retrieve stored memories. The memory system can reduce \
                     repetitive context and improve personalization.",
                    profile.memory_retrieval_rate * 100.0
                ),
                action:
                    "Store frequently referenced facts in semantic memory to reduce prompt repetition."
                        .into(),
                evidence: vec![Evidence {
                    metric: "memory_retrieval_rate".into(),
                    value: format!("{:.0}%", profile.memory_retrieval_rate * 100.0),
                    context: "percentage of turns using memory retrieval".into(),
                }],
                estimated_impact: Some(Impact {
                    monthly_savings: None,
                    quality_change: Some("better personalization and consistency".into()),
                    description: "Leverage memory for repeated context".into(),
                }),
            })
        } else {
            None
        }
    }
}

struct MemoryOverloaded;

impl RecommendationRule for MemoryOverloaded {
    fn name(&self) -> &str {
        "memory_overloaded"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::MemoryLeverage
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 20 {
            return None;
        }
        if profile.memory_retrieval_rate > 0.9 && profile.avg_tokens_per_turn > 2000.0 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Medium,
                title: "Memory tokens may be crowding out conversation history".into(),
                explanation: format!(
                    "Memory is retrieved on {:.0}% of turns with an average of {:.0} tokens/turn. \
                     Excessive memory injection can crowd out useful conversation history.",
                    profile.memory_retrieval_rate * 100.0,
                    profile.avg_tokens_per_turn
                ),
                action:
                    "Review and prune stored memories; increase relevance threshold for retrieval."
                        .into(),
                evidence: vec![
                    Evidence {
                        metric: "memory_retrieval_rate".into(),
                        value: format!("{:.0}%", profile.memory_retrieval_rate * 100.0),
                        context: "memory retrieval rate".into(),
                    },
                    Evidence {
                        metric: "avg_tokens_per_turn".into(),
                        value: format!("{:.0}", profile.avg_tokens_per_turn),
                        context: "high token count may indicate memory bloat".into(),
                    },
                ],
                estimated_impact: Some(Impact {
                    monthly_savings: Some(profile.total_cost * 0.15),
                    quality_change: Some("better balance of memory vs history".into()),
                    description: "Reduce memory token overhead".into(),
                }),
            })
        } else {
            None
        }
    }
}

// ── Cost Optimization rules ───────────────────────────────────

struct SystemPromptROI;

impl RecommendationRule for SystemPromptROI {
    fn name(&self) -> &str {
        "system_prompt_roi"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::CostOptimization
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 10 || profile.avg_tokens_per_turn < 1500.0 {
            return None;
        }
        let system_cost_estimate = profile.total_cost * 0.3;
        Some(Recommendation {
            category: self.category(),
            priority: Priority::Medium,
            title: "Large input tokens suggest heavy system prompt".into(),
            explanation: format!(
                "Average input is {:.0} tokens/turn. If a large system prompt accounts for a \
                 significant portion, consider trimming non-essential instructions.",
                profile.avg_tokens_per_turn
            ),
            action: "Audit your system prompt for redundant instructions. Move static context \
                to memory retrieval."
                .into(),
            evidence: vec![Evidence {
                metric: "avg_tokens_per_turn".into(),
                value: format!("{:.0}", profile.avg_tokens_per_turn),
                context: "tokens per turn (system prompt is re-sent each turn)".into(),
            }],
            estimated_impact: Some(Impact {
                monthly_savings: Some(system_cost_estimate * 0.2),
                quality_change: None,
                description: "Reduce per-turn system prompt overhead".into(),
            }),
        })
    }
}

struct CachingOpportunity;

impl RecommendationRule for CachingOpportunity {
    fn name(&self) -> &str {
        "caching_opportunity"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::CostOptimization
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 10 {
            return None;
        }
        if profile.cache_hit_rate < 0.15 {
            let potential_savings = profile.total_cost * 0.2;
            Some(Recommendation {
                category: self.category(),
                priority: Priority::High,
                title: "Low cache hit rate — significant savings possible".into(),
                explanation: format!(
                    "Cache hit rate is only {:.1}%. Enabling or tuning semantic caching for \
                     repeated/similar queries could save ~20% of inference costs.",
                    profile.cache_hit_rate * 100.0
                ),
                action: "Enable semantic caching and review cache TTL settings.".into(),
                evidence: vec![Evidence {
                    metric: "cache_hit_rate".into(),
                    value: format!("{:.1}%", profile.cache_hit_rate * 100.0),
                    context: "current cache hit rate".into(),
                }],
                estimated_impact: Some(Impact {
                    monthly_savings: Some(potential_savings),
                    quality_change: None,
                    description: "Cache repeated queries to avoid redundant inference".into(),
                }),
            })
        } else {
            None
        }
    }
}

struct ToolCostAwareness;

impl RecommendationRule for ToolCostAwareness {
    fn name(&self) -> &str {
        "tool_cost_awareness"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::CostOptimization
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 10 {
            return None;
        }
        if profile.tool_success_rate < 0.7 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::Medium,
                title: "Tool calls have a high failure rate".into(),
                explanation: format!(
                    "Tool success rate is {:.0}%. Failed tool calls still cost tokens for the \
                     request and response. Improving tool reliability reduces wasted spend.",
                    profile.tool_success_rate * 100.0
                ),
                action:
                    "Review failing tool calls. Consider better error handling or input validation."
                        .into(),
                evidence: vec![Evidence {
                    metric: "tool_success_rate".into(),
                    value: format!("{:.0}%", profile.tool_success_rate * 100.0),
                    context: "tool call success rate".into(),
                }],
                estimated_impact: Some(Impact {
                    monthly_savings: Some(
                        profile.total_cost * (1.0 - profile.tool_success_rate) * 0.1,
                    ),
                    quality_change: Some("fewer errors, smoother interactions".into()),
                    description: "Reduce wasted inference from failed tool calls".into(),
                }),
            })
        } else {
            None
        }
    }
}

// ── Configuration rules ───────────────────────────────────────

struct FallbackChainTuning;

impl RecommendationRule for FallbackChainTuning {
    fn name(&self) -> &str {
        "fallback_chain_tuning"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::Configuration
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.models_used.len() < 2 || profile.total_turns < 20 {
            return None;
        }
        let primary = profile.models_used.first()?;
        let primary_stats = profile.model_stats.get(primary)?;
        let total_non_primary: i64 = profile
            .model_stats
            .iter()
            .filter(|(k, _)| *k != primary)
            .map(|(_, s)| s.turns)
            .sum();

        let fallback_rate = total_non_primary as f64 / profile.total_turns as f64;
        if fallback_rate > 0.3 {
            Some(Recommendation {
                category: self.category(),
                priority: Priority::High,
                title: format!("Primary model ({primary}) falls back {:.0}% of the time", fallback_rate * 100.0),
                explanation: format!(
                    "{:.0}% of turns use fallback models instead of {primary}. This suggests \
                     the primary provider may be unreliable or rate-limited.",
                    fallback_rate * 100.0
                ),
                action: "Check circuit breaker status; consider switching primary model or adding API key budget.".into(),
                evidence: vec![
                    Evidence {
                        metric: "primary_turns".into(),
                        value: format!("{}", primary_stats.turns),
                        context: format!("turns on {primary}"),
                    },
                    Evidence {
                        metric: "fallback_turns".into(),
                        value: format!("{total_non_primary}"),
                        context: "turns on fallback models".into(),
                    },
                ],
                estimated_impact: None,
            })
        } else {
            None
        }
    }
}

struct HighCostPerTurn;

impl RecommendationRule for HighCostPerTurn {
    fn name(&self) -> &str {
        "high_cost_per_turn"
    }
    fn category(&self) -> RecommendationCategory {
        RecommendationCategory::Configuration
    }
    fn evaluate(&self, profile: &UserProfile) -> Option<Recommendation> {
        if profile.total_turns < 5 {
            return None;
        }
        let avg_cost = profile.total_cost / profile.total_turns as f64;
        if avg_cost > 0.05 {
            Some(Recommendation {
                category: self.category(),
                priority: if avg_cost > 0.10 {
                    Priority::High
                } else {
                    Priority::Medium
                },
                title: format!("Average cost per turn is ${:.4}", avg_cost),
                explanation: format!(
                    "At ${:.4}/turn, your inference costs are above the typical threshold. \
                     Over {} turns, this totals ${:.2}.",
                    avg_cost, profile.total_turns, profile.total_cost
                ),
                action: "Review model selection, context window size, and caching settings.".into(),
                evidence: vec![
                    Evidence {
                        metric: "avg_cost_per_turn".into(),
                        value: format!("${:.4}", avg_cost),
                        context: "above $0.05/turn threshold".into(),
                    },
                    Evidence {
                        metric: "total_cost".into(),
                        value: format!("${:.4}", profile.total_cost),
                        context: format!("over {} turns", profile.total_turns),
                    },
                ],
                estimated_impact: Some(Impact {
                    monthly_savings: Some(profile.total_cost * 0.3),
                    quality_change: None,
                    description: "Reduce overall inference spend".into(),
                }),
            })
        } else {
            None
        }
    }
}

// ── LLM-powered deep analysis (stub) ─────────────────────────

pub struct LlmRecommendationAnalyzer;

impl LlmRecommendationAnalyzer {
    pub fn build_prompt(profile: &UserProfile, heuristic_recs: &[Recommendation]) -> String {
        let mut prompt = String::from(
            "You are an AI usage optimization expert. Analyze the following user profile and \
             heuristic recommendations, then provide deeper, actionable insights.\n\n",
        );

        prompt.push_str(&format!(
            "## User Profile\n\
             - Total sessions: {}\n\
             - Total turns: {}\n\
             - Total cost: ${:.4}\n\
             - Models used: {}\n\
             - Avg session length: {:.1} turns\n\
             - Avg tokens/turn: {:.0}\n\
             - Cache hit rate: {:.1}%\n\
             - Tool success rate: {:.1}%\n\n",
            profile.total_sessions,
            profile.total_turns,
            profile.total_cost,
            profile.models_used.join(", "),
            profile.avg_session_length,
            profile.avg_tokens_per_turn,
            profile.cache_hit_rate * 100.0,
            profile.tool_success_rate * 100.0,
        ));

        prompt.push_str("## Model Breakdown\n");
        for (name, stats) in &profile.model_stats {
            prompt.push_str(&format!(
                "- {name}: {turns} turns, ${cost:.4}/turn, density={density:.3}, cache={cache:.0}%\n",
                turns = stats.turns,
                cost = stats.avg_cost,
                density = stats.avg_output_density,
                cache = stats.cache_hit_rate * 100.0,
            ));
        }

        prompt.push_str("\n## Heuristic Recommendations Already Generated\n");
        for (i, rec) in heuristic_recs.iter().enumerate() {
            prompt.push_str(&format!(
                "{}. [{:?}] {}: {}\n",
                i + 1,
                rec.priority,
                rec.title,
                rec.explanation
            ));
        }

        prompt.push_str(
            "\n## Your Task\n\
             Provide 3-5 additional recommendations that go beyond the heuristics above. \
             Focus on:\n\
             1. Cross-cutting patterns the heuristics missed\n\
             2. Workflow optimizations specific to the model mix\n\
             3. Cost/quality trade-offs with concrete numbers\n\
             4. Configuration changes with expected impact\n\n\
             Format each recommendation as:\n\
             **Title**: ...\n\
             **Priority**: High/Medium/Low\n\
             **Explanation**: ...\n\
             **Action**: ...\n\
             **Expected Impact**: ...\n",
        );

        prompt
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn base_profile() -> UserProfile {
        UserProfile {
            total_sessions: 10,
            total_turns: 100,
            total_cost: 1.5,
            avg_quality: Some(0.8),
            grade_coverage: 0.5,
            models_used: vec!["claude-4".into(), "gpt-4".into()],
            model_stats: HashMap::from([
                (
                    "claude-4".into(),
                    ModelStats {
                        turns: 60,
                        avg_cost: 0.02,
                        avg_quality: Some(0.85),
                        cache_hit_rate: 0.3,
                        avg_output_density: 0.25,
                    },
                ),
                (
                    "gpt-4".into(),
                    ModelStats {
                        turns: 40,
                        avg_cost: 0.01,
                        avg_quality: Some(0.75),
                        cache_hit_rate: 0.4,
                        avg_output_density: 0.20,
                    },
                ),
            ]),
            avg_session_length: 10.0,
            avg_tokens_per_turn: 200.0,
            tool_success_rate: 0.9,
            cache_hit_rate: 0.35,
            memory_retrieval_rate: 0.5,
        }
    }

    #[test]
    fn engine_produces_sorted_recommendations() {
        let engine = RecommendationEngine::new();
        let mut profile = base_profile();
        profile.cache_hit_rate = 0.05;
        profile.avg_tokens_per_turn = 30.0;

        let recs = engine.generate(&profile);
        assert!(!recs.is_empty());

        for window in recs.windows(2) {
            assert!(
                window[0].priority.ordinal() >= window[1].priority.ordinal(),
                "recommendations not sorted by priority"
            );
        }
    }

    #[test]
    fn specificity_fires_for_short_queries() {
        let rule = SpecificityCorrelation;
        let mut profile = base_profile();
        profile.avg_tokens_per_turn = 25.0;
        assert!(rule.evaluate(&profile).is_some());

        profile.avg_tokens_per_turn = 150.0;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn follow_up_fires_for_long_sessions() {
        let rule = FollowUpPatterns;
        let mut profile = base_profile();
        profile.avg_session_length = 25.0;
        assert!(rule.evaluate(&profile).is_some());

        profile.avg_session_length = 5.0;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn caching_opportunity_fires_for_low_hit_rate() {
        let rule = CachingOpportunity;
        let mut profile = base_profile();
        profile.cache_hit_rate = 0.05;
        let rec = rule.evaluate(&profile);
        assert!(rec.is_some());
        assert_eq!(rec.unwrap().priority, Priority::High);

        profile.cache_hit_rate = 0.5;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn high_cost_fires_above_threshold() {
        let rule = HighCostPerTurn;
        let mut profile = base_profile();
        profile.total_cost = 15.0;
        profile.total_turns = 100;
        let rec = rule.evaluate(&profile);
        assert!(rec.is_some());
        assert_eq!(rec.unwrap().priority, Priority::High);
    }

    #[test]
    fn high_cost_silent_below_threshold() {
        let rule = HighCostPerTurn;
        let mut profile = base_profile();
        profile.total_cost = 1.0;
        profile.total_turns = 100;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn pareto_detects_dominated_model() {
        let rule = ParetoOptimalModels;
        let mut profile = base_profile();
        profile.model_stats.insert(
            "expensive-bad".into(),
            ModelStats {
                turns: 20,
                avg_cost: 0.05,
                avg_quality: None,
                cache_hit_rate: 0.1,
                avg_output_density: 0.10,
            },
        );
        profile.model_stats.insert(
            "cheap-good".into(),
            ModelStats {
                turns: 20,
                avg_cost: 0.01,
                avg_quality: None,
                cache_hit_rate: 0.5,
                avg_output_density: 0.30,
            },
        );
        assert!(rule.evaluate(&profile).is_some());
    }

    #[test]
    fn tool_cost_fires_for_low_success() {
        let rule = ToolCostAwareness;
        let mut profile = base_profile();
        profile.tool_success_rate = 0.5;
        assert!(rule.evaluate(&profile).is_some());

        profile.tool_success_rate = 0.95;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn fallback_fires_for_high_rate() {
        let rule = FallbackChainTuning;
        let mut profile = base_profile();
        profile.model_stats.get_mut("gpt-4").unwrap().turns = 80;
        profile.model_stats.get_mut("claude-4").unwrap().turns = 20;
        profile.total_turns = 100;
        assert!(rule.evaluate(&profile).is_some());
    }

    #[test]
    fn stale_session_fires_for_long_sessions() {
        let rule = StaleSessionCost;
        let mut profile = base_profile();
        profile.avg_session_length = 30.0;
        assert!(rule.evaluate(&profile).is_some());

        profile.avg_session_length = 8.0;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn memory_underutilized_fires() {
        let rule = MemoryUnderutilized;
        let mut profile = base_profile();
        profile.memory_retrieval_rate = 0.05;
        assert!(rule.evaluate(&profile).is_some());

        profile.memory_retrieval_rate = 0.6;
        assert!(rule.evaluate(&profile).is_none());
    }

    #[test]
    fn llm_prompt_contains_profile_data() {
        let profile = base_profile();
        let recs = RecommendationEngine::new().generate(&profile);
        let prompt = LlmRecommendationAnalyzer::build_prompt(&profile, &recs);
        assert!(prompt.contains("Total sessions: 10"));
        assert!(prompt.contains("claude-4"));
        assert!(prompt.contains("Your Task"));
    }

    #[test]
    fn tool_cost_fires_for_zero_success_rate() {
        let rule = ToolCostAwareness;
        let mut profile = base_profile();
        profile.tool_success_rate = 0.0;
        let rec = rule.evaluate(&profile);
        assert!(rec.is_some(), "0% tool success should trigger the rule");
        let rec = rec.unwrap();
        assert!(rec.explanation.contains("0%"));
    }

    #[test]
    fn tool_cost_silent_for_high_success() {
        let rule = ToolCostAwareness;
        let mut profile = base_profile();
        profile.tool_success_rate = 0.85;
        assert!(
            rule.evaluate(&profile).is_none(),
            "85% success rate should not trigger"
        );
    }

    #[test]
    fn tool_cost_silent_for_insufficient_data() {
        let rule = ToolCostAwareness;
        let mut profile = base_profile();
        profile.total_turns = 5;
        profile.tool_success_rate = 0.0;
        assert!(
            rule.evaluate(&profile).is_none(),
            "should not fire with < 10 turns"
        );
    }

    #[test]
    fn engine_no_recs_for_minimal_profile() {
        let engine = RecommendationEngine::new();
        let profile = UserProfile {
            total_sessions: 1,
            total_turns: 2,
            total_cost: 0.001,
            avg_quality: None,
            grade_coverage: 0.0,
            models_used: vec!["test".into()],
            model_stats: HashMap::from([(
                "test".into(),
                ModelStats {
                    turns: 2,
                    avg_cost: 0.0005,
                    avg_quality: None,
                    cache_hit_rate: 0.0,
                    avg_output_density: 0.3,
                },
            )]),
            avg_session_length: 2.0,
            avg_tokens_per_turn: 100.0,
            tool_success_rate: 1.0,
            cache_hit_rate: 0.5,
            memory_retrieval_rate: 0.5,
        };
        let recs = engine.generate(&profile);
        assert!(recs.is_empty(), "minimal profile should produce no recs");
    }
}
