use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ── Core types ──────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RuleCategory {
    Budget,
    Memory,
    Prompt,
    Tools,
    Cost,
    Quality,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tip {
    pub severity: Severity,
    pub category: RuleCategory,
    pub rule_name: String,
    pub message: String,
    pub suggestion: String,
}

pub struct TurnData {
    pub turn_id: String,
    pub token_budget: i64,
    pub system_prompt_tokens: i64,
    pub memory_tokens: i64,
    pub history_tokens: i64,
    pub history_depth: i64,
    pub complexity_level: String,
    pub model: String,
    pub cost: f64,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub tool_call_count: i64,
    pub tool_failure_count: i64,
    pub thinking_length: i64,
    pub has_reasoning: bool,
    pub cached: bool,
}

pub struct SessionData {
    pub turns: Vec<TurnData>,
    pub session_id: String,
    pub grades: Vec<(String, i32)>,
}

// ── Traits ──────────────────────────────────────────────────────

pub trait AnalysisRule: Send + Sync {
    fn name(&self) -> &str;
    fn category(&self) -> RuleCategory;
    fn evaluate_turn(&self, turn: &TurnData, session_avg_cost: Option<f64>) -> Option<Tip>;
}

pub trait SessionAnalysisRule: Send + Sync {
    fn name(&self) -> &str;
    fn category(&self) -> RuleCategory;
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip>;
}

// ── Per-turn rules ──────────────────────────────────────────────

pub struct BudgetPressure;

impl AnalysisRule for BudgetPressure {
    fn name(&self) -> &str {
        "budget_pressure"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Budget
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.token_budget <= 0 {
            return None;
        }
        let used = turn.system_prompt_tokens + turn.memory_tokens + turn.history_tokens;
        let util = used as f64 / turn.token_budget as f64;
        if util > 0.90 {
            Some(Tip {
                severity: if util > 0.95 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                category: RuleCategory::Budget,
                rule_name: self.name().into(),
                message: format!(
                    "Token utilization at {:.0}% of budget ({} / {})",
                    util * 100.0,
                    used,
                    turn.token_budget
                ),
                suggestion: "Consider reducing system prompt size, pruning history, or increasing the token budget.".into(),
            })
        } else {
            None
        }
    }
}

pub struct SystemPromptHeavy;

impl AnalysisRule for SystemPromptHeavy {
    fn name(&self) -> &str {
        "system_prompt_heavy"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Prompt
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.token_budget <= 0 {
            return None;
        }
        let pct = turn.system_prompt_tokens as f64 / turn.token_budget as f64;
        if pct > 0.40 {
            Some(Tip {
                severity: if pct > 0.60 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                category: RuleCategory::Prompt,
                rule_name: self.name().into(),
                message: format!(
                    "System prompt consumes {:.0}% of the token budget ({} tokens)",
                    pct * 100.0,
                    turn.system_prompt_tokens
                ),
                suggestion: "Audit the system prompt for redundancy. Move static instructions to a retrieval layer or condense with structured formatting.".into(),
            })
        } else {
            None
        }
    }
}

pub struct MemoryStarvation;

impl AnalysisRule for MemoryStarvation {
    fn name(&self) -> &str {
        "memory_starvation"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Memory
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.token_budget <= 0 {
            return None;
        }
        let pct = turn.memory_tokens as f64 / turn.token_budget as f64;
        if pct < 0.10 && turn.memory_tokens >= 0 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Memory,
                rule_name: self.name().into(),
                message: format!(
                    "Memory allocation is only {:.0}% of budget ({} tokens)",
                    pct * 100.0,
                    turn.memory_tokens
                ),
                suggestion: "The agent may lack long-term context. Check that memory retrieval is configured and returning relevant results.".into(),
            })
        } else {
            None
        }
    }
}

pub struct ShallowHistory;

impl AnalysisRule for ShallowHistory {
    fn name(&self) -> &str {
        "shallow_history"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.history_depth < 3 && turn.history_depth >= 0 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: format!(
                    "Only {} messages in conversation history",
                    turn.history_depth
                ),
                suggestion: "With shallow history the model may lack conversational context. This is normal for early turns in a session.".into(),
            })
        } else {
            None
        }
    }
}

pub struct HighToolDensity;

impl AnalysisRule for HighToolDensity {
    fn name(&self) -> &str {
        "high_tool_density"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Tools
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.tool_call_count > 3 {
            Some(Tip {
                severity: if turn.tool_call_count > 8 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                category: RuleCategory::Tools,
                rule_name: self.name().into(),
                message: format!(
                    "{} tool calls in a single turn",
                    turn.tool_call_count
                ),
                suggestion: "High tool density increases latency and cost. Consider whether the agent could batch operations or use a more targeted approach.".into(),
            })
        } else {
            None
        }
    }
}

pub struct ToolFailures;

impl AnalysisRule for ToolFailures {
    fn name(&self) -> &str {
        "tool_failures"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Tools
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.tool_failure_count > 0 {
            let rate = if turn.tool_call_count > 0 {
                turn.tool_failure_count as f64 / turn.tool_call_count as f64
            } else {
                1.0
            };
            Some(Tip {
                severity: if rate > 0.5 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                category: RuleCategory::Tools,
                rule_name: self.name().into(),
                message: format!(
                    "{} of {} tool calls failed ({:.0}% failure rate)",
                    turn.tool_failure_count,
                    turn.tool_call_count,
                    rate * 100.0
                ),
                suggestion: "Investigate tool errors. Frequent failures waste tokens on retry loops and degrade response quality.".into(),
            })
        } else {
            None
        }
    }
}

pub struct ExpensiveTurn;

impl AnalysisRule for ExpensiveTurn {
    fn name(&self) -> &str {
        "expensive_turn"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_turn(&self, turn: &TurnData, session_avg_cost: Option<f64>) -> Option<Tip> {
        let avg = session_avg_cost?;
        if avg <= 0.0 {
            return None;
        }
        let ratio = turn.cost / avg;
        if ratio > 2.0 {
            Some(Tip {
                severity: if ratio > 5.0 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "Turn cost ${:.4} is {:.1}x the session average (${:.4})",
                    turn.cost, ratio, avg
                ),
                suggestion: "This turn was unusually expensive. Check for large context windows, expensive model selection, or excessive tool usage.".into(),
            })
        } else {
            None
        }
    }
}

pub struct EmptyReasoning;

impl AnalysisRule for EmptyReasoning {
    fn name(&self) -> &str {
        "empty_reasoning"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.has_reasoning && turn.thinking_length == 0 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: "Model supports reasoning but produced no thinking trace".into(),
                suggestion: "The model may have skipped reasoning for a simple query, or the thinking budget may be too low. No action needed if the response was adequate.".into(),
            })
        } else {
            None
        }
    }
}

pub struct SystemPromptTax;

impl AnalysisRule for SystemPromptTax {
    fn name(&self) -> &str {
        "system_prompt_tax"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.tokens_in <= 0 {
            return None;
        }
        let sys_fraction = turn.system_prompt_tokens as f64 / turn.tokens_in as f64;
        let estimated_sys_cost = turn.cost * sys_fraction;
        if estimated_sys_cost > 0.01 && sys_fraction > 0.30 {
            Some(Tip {
                severity: if estimated_sys_cost > 0.05 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "System prompt accounts for ~${:.4} ({:.0}% of input tokens)",
                    estimated_sys_cost,
                    sys_fraction * 100.0
                ),
                suggestion: "Repeated system prompt tokens add up. Consider prompt caching, compression, or moving static content to retrieval.".into(),
            })
        } else {
            None
        }
    }
}

pub struct HistoryCostDominant;

impl AnalysisRule for HistoryCostDominant {
    fn name(&self) -> &str {
        "history_cost_dominant"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.tokens_in <= 0 {
            return None;
        }
        let hist_fraction = turn.history_tokens as f64 / turn.tokens_in as f64;
        if hist_fraction > 0.60 {
            Some(Tip {
                severity: if hist_fraction > 0.80 {
                    Severity::Warning
                } else {
                    Severity::Info
                },
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "History tokens consume {:.0}% of input tokens ({} / {})",
                    hist_fraction * 100.0,
                    turn.history_tokens,
                    turn.tokens_in
                ),
                suggestion: "Conversation history dominates input cost. Consider summarizing older messages or reducing history window depth.".into(),
            })
        } else {
            None
        }
    }
}

pub struct LargeOutputRatio;

impl AnalysisRule for LargeOutputRatio {
    fn name(&self) -> &str {
        "large_output_ratio"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.tokens_in <= 0 || turn.tokens_out <= 0 {
            return None;
        }
        let ratio = turn.tokens_out as f64 / turn.tokens_in as f64;
        if ratio > 2.0 && turn.tokens_out > 2000 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "Output ({} tokens) is {:.1}x the input ({} tokens)",
                    turn.tokens_out, ratio, turn.tokens_in
                ),
                suggestion: "Large output may indicate verbose responses. Check if the model is being asked for overly detailed answers where conciseness would suffice.".into(),
            })
        } else {
            None
        }
    }
}

pub struct CachedTurnSavings;

impl AnalysisRule for CachedTurnSavings {
    fn name(&self) -> &str {
        "cached_turn_savings"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_turn(&self, turn: &TurnData, _avg: Option<f64>) -> Option<Tip> {
        if turn.cached {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: "This turn was served from cache".into(),
                suggestion: "Cache hit saved inference cost. Frequently cached queries may indicate the system prompt or user patterns are repetitive.".into(),
            })
        } else {
            None
        }
    }
}

// ── Session-aggregate rules ─────────────────────────────────────

pub struct ContextDrift;

impl SessionAnalysisRule for ContextDrift {
    fn name(&self) -> &str {
        "context_drift"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Budget
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let turns = &session.turns;
        if turns.len() < 4 {
            return None;
        }
        let utils: Vec<f64> = turns
            .iter()
            .filter(|t| t.token_budget > 0)
            .map(|t| {
                let used = t.system_prompt_tokens + t.memory_tokens + t.history_tokens;
                used as f64 / t.token_budget as f64
            })
            .collect();
        if utils.len() < 4 {
            return None;
        }
        let half = utils.len() / 2;
        let first_half_avg: f64 = utils[..half].iter().sum::<f64>() / half as f64;
        let second_half_avg: f64 = utils[half..].iter().sum::<f64>() / (utils.len() - half) as f64;
        if second_half_avg > first_half_avg + 0.15 {
            Some(Tip {
                severity: Severity::Warning,
                category: RuleCategory::Budget,
                rule_name: self.name().into(),
                message: format!(
                    "Budget utilization trending upward: {:.0}% → {:.0}%",
                    first_half_avg * 100.0,
                    second_half_avg * 100.0
                ),
                suggestion: "Context is growing across turns. Consider more aggressive history pruning or a summarization step.".into(),
            })
        } else {
            None
        }
    }
}

pub struct FrequentEscalation;

impl SessionAnalysisRule for FrequentEscalation {
    fn name(&self) -> &str {
        "frequent_escalation"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let turns = &session.turns;
        if turns.is_empty() {
            return None;
        }
        let high_complexity = turns
            .iter()
            .filter(|t| t.complexity_level == "L2" || t.complexity_level == "L3")
            .count();
        let pct = high_complexity as f64 / turns.len() as f64;
        if pct > 0.40 {
            Some(Tip {
                severity: Severity::Warning,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: format!(
                    "{:.0}% of turns ({}/{}) at L2/L3 complexity",
                    pct * 100.0,
                    high_complexity,
                    turns.len()
                ),
                suggestion: "Frequent complexity escalation drives up cost. Evaluate whether the escalation triggers are too sensitive or the base model could handle more queries.".into(),
            })
        } else {
            None
        }
    }
}

pub struct CostAcceleration;

impl SessionAnalysisRule for CostAcceleration {
    fn name(&self) -> &str {
        "cost_acceleration"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let turns = &session.turns;
        if turns.len() < 4 {
            return None;
        }
        let costs: Vec<f64> = turns.iter().map(|t| t.cost).collect();
        let half = costs.len() / 2;
        let first_avg: f64 = costs[..half].iter().sum::<f64>() / half as f64;
        let second_avg: f64 = costs[half..].iter().sum::<f64>() / (costs.len() - half) as f64;
        if first_avg > 0.0 && second_avg > first_avg * 1.5 {
            Some(Tip {
                severity: Severity::Warning,
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "Per-turn cost increasing: ${:.4} avg (first half) → ${:.4} avg (second half)",
                    first_avg, second_avg
                ),
                suggestion: "Costs are accelerating across the session. This often signals growing context windows or model escalation. Consider resetting the session or pruning aggressively.".into(),
            })
        } else {
            None
        }
    }
}

pub struct UnderutilizedMemory;

impl SessionAnalysisRule for UnderutilizedMemory {
    fn name(&self) -> &str {
        "underutilized_memory"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Memory
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let turns = &session.turns;
        if turns.is_empty() {
            return None;
        }
        let all_zero = turns.iter().all(|t| t.memory_tokens == 0);
        if all_zero {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Memory,
                rule_name: self.name().into(),
                message: "No memory tokens used across the entire session".into(),
                suggestion: "Memory retrieval produced no content for any turn. Verify that memories exist and the retrieval pipeline is functioning.".into(),
            })
        } else {
            None
        }
    }
}

pub struct ToolSuccessRate;

impl SessionAnalysisRule for ToolSuccessRate {
    fn name(&self) -> &str {
        "tool_success_rate"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Tools
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let total_calls: i64 = session.turns.iter().map(|t| t.tool_call_count).sum();
        let total_failures: i64 = session.turns.iter().map(|t| t.tool_failure_count).sum();
        if total_calls < 5 {
            return None;
        }
        let success_rate = 1.0 - (total_failures as f64 / total_calls as f64);
        if success_rate < 0.80 {
            Some(Tip {
                severity: if success_rate < 0.50 {
                    Severity::Critical
                } else {
                    Severity::Warning
                },
                category: RuleCategory::Tools,
                rule_name: self.name().into(),
                message: format!(
                    "Session-wide tool success rate is {:.0}% ({} failures / {} total calls)",
                    success_rate * 100.0,
                    total_failures,
                    total_calls
                ),
                suggestion: "Chronic tool failures waste tokens and degrade quality. Investigate the most common failure modes.".into(),
            })
        } else {
            None
        }
    }
}

pub struct ModelChurn;

impl SessionAnalysisRule for ModelChurn {
    fn name(&self) -> &str {
        "model_churn"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let models: HashSet<&str> = session
            .turns
            .iter()
            .filter(|t| !t.model.is_empty())
            .map(|t| t.model.as_str())
            .collect();
        if models.len() > 3 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: format!(
                    "{} different models used across {} turns: {}",
                    models.len(),
                    session.turns.len(),
                    models.into_iter().collect::<Vec<_>>().join(", ")
                ),
                suggestion: "Frequent model switching can cause inconsistent tone and behavior. Consider stabilizing the model selection unless complexity-based routing is intentional.".into(),
            })
        } else {
            None
        }
    }
}

// ── Quality-aware session rules ─────────────────────────────────

pub struct QualityDeclining;

impl SessionAnalysisRule for QualityDeclining {
    fn name(&self) -> &str {
        "quality_declining"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        let grades = &session.grades;
        if grades.len() < 4 {
            return None;
        }
        let half = grades.len() / 2;
        let first_avg = grades[..half].iter().map(|(_, g)| *g as f64).sum::<f64>() / half as f64;
        let second_avg = grades[half..].iter().map(|(_, g)| *g as f64).sum::<f64>()
            / (grades.len() - half) as f64;
        if first_avg - second_avg > 0.5 {
            Some(Tip {
                severity: Severity::Warning,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: format!(
                    "Average grade declined from {:.1} to {:.1} over the session",
                    first_avg, second_avg
                ),
                suggestion: "Quality is dropping as the conversation progresses. This may indicate context degradation, model fatigue, or increasingly complex queries. Consider resetting or adjusting the model.".into(),
            })
        } else {
            None
        }
    }
}

pub struct CostQualityMismatch;

impl SessionAnalysisRule for CostQualityMismatch {
    fn name(&self) -> &str {
        "cost_quality_mismatch"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Cost
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        if session.grades.is_empty() || session.turns.len() < 2 {
            return None;
        }
        let grade_map: std::collections::HashMap<&str, Vec<i32>> = {
            let mut m: std::collections::HashMap<&str, Vec<i32>> = std::collections::HashMap::new();
            for (turn_id, grade) in &session.grades {
                if let Some(turn) = session.turns.iter().find(|t| t.turn_id == *turn_id) {
                    m.entry(turn.model.as_str()).or_default().push(*grade);
                }
            }
            m
        };
        let cost_map: std::collections::HashMap<&str, f64> = {
            let mut m: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
            for t in &session.turns {
                *m.entry(t.model.as_str()).or_default() += t.cost;
            }
            m
        };

        if grade_map.len() < 2 {
            return None;
        }

        let most_expensive = cost_map
            .iter()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(m, _)| *m)?;

        let exp_grades = grade_map.get(most_expensive)?;
        let exp_avg = exp_grades.iter().map(|g| *g as f64).sum::<f64>() / exp_grades.len() as f64;

        let best_quality = grade_map
            .iter()
            .filter(|(m, _)| **m != most_expensive)
            .map(|(_, gs)| gs.iter().map(|g| *g as f64).sum::<f64>() / gs.len() as f64)
            .fold(0.0_f64, f64::max);

        if best_quality > exp_avg {
            Some(Tip {
                severity: Severity::Warning,
                category: RuleCategory::Cost,
                rule_name: self.name().into(),
                message: format!(
                    "Most expensive model ({}) has lower quality ({:.1}) than a cheaper alternative ({:.1})",
                    most_expensive, exp_avg, best_quality
                ),
                suggestion: "The highest-cost model isn't producing the best grades. Consider routing more queries to the higher-quality, lower-cost model.".into(),
            })
        } else {
            None
        }
    }
}

pub struct MemoryHelps;

impl SessionAnalysisRule for MemoryHelps {
    fn name(&self) -> &str {
        "memory_helps"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Memory
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        if session.grades.is_empty() {
            return None;
        }
        let mut with_mem: Vec<f64> = Vec::new();
        let mut without_mem: Vec<f64> = Vec::new();
        for (turn_id, grade) in &session.grades {
            if let Some(turn) = session.turns.iter().find(|t| t.turn_id == *turn_id) {
                if turn.memory_tokens > 0 {
                    with_mem.push(*grade as f64);
                } else {
                    without_mem.push(*grade as f64);
                }
            }
        }
        if with_mem.len() < 2 || without_mem.len() < 2 {
            return None;
        }
        let with_avg = with_mem.iter().sum::<f64>() / with_mem.len() as f64;
        let without_avg = without_mem.iter().sum::<f64>() / without_mem.len() as f64;
        if with_avg > without_avg + 0.5 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Memory,
                rule_name: self.name().into(),
                message: format!(
                    "Quality is {:.1} with memory vs {:.1} without — memory retrieval is helping",
                    with_avg, without_avg
                ),
                suggestion: "Memory-augmented turns produce higher quality. Ensure memory retrieval is consistently available and consider expanding memory coverage.".into(),
            })
        } else {
            None
        }
    }
}

pub struct LowCoverageWarning;

impl SessionAnalysisRule for LowCoverageWarning {
    fn name(&self) -> &str {
        "low_coverage_warning"
    }
    fn category(&self) -> RuleCategory {
        RuleCategory::Quality
    }
    fn evaluate_session(&self, session: &SessionData) -> Option<Tip> {
        if session.turns.len() < 50 {
            return None;
        }
        let coverage = session.grades.len() as f64 / session.turns.len() as f64;
        if coverage < 0.20 {
            Some(Tip {
                severity: Severity::Info,
                category: RuleCategory::Quality,
                rule_name: self.name().into(),
                message: format!(
                    "Only {:.0}% of turns have been graded ({}/{})",
                    coverage * 100.0,
                    session.grades.len(),
                    session.turns.len()
                ),
                suggestion: "Grade coverage is low. Quality metrics may not be representative. Consider grading more turns to get reliable quality signals.".into(),
            })
        } else {
            None
        }
    }
}

// ── Context Analyzer ────────────────────────────────────────────

pub struct ContextAnalyzer {
    turn_rules: Vec<Box<dyn AnalysisRule>>,
    session_rules: Vec<Box<dyn SessionAnalysisRule>>,
}

impl Default for ContextAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextAnalyzer {
    pub fn new() -> Self {
        Self {
            turn_rules: vec![
                Box::new(BudgetPressure),
                Box::new(SystemPromptHeavy),
                Box::new(MemoryStarvation),
                Box::new(ShallowHistory),
                Box::new(HighToolDensity),
                Box::new(ToolFailures),
                Box::new(ExpensiveTurn),
                Box::new(EmptyReasoning),
                Box::new(SystemPromptTax),
                Box::new(HistoryCostDominant),
                Box::new(LargeOutputRatio),
                Box::new(CachedTurnSavings),
            ],
            session_rules: vec![
                Box::new(ContextDrift),
                Box::new(FrequentEscalation),
                Box::new(CostAcceleration),
                Box::new(UnderutilizedMemory),
                Box::new(ToolSuccessRate),
                Box::new(ModelChurn),
                Box::new(QualityDeclining),
                Box::new(CostQualityMismatch),
                Box::new(MemoryHelps),
                Box::new(LowCoverageWarning),
            ],
        }
    }

    pub fn analyze_turn(&self, turn: &TurnData, session_avg_cost: Option<f64>) -> Vec<Tip> {
        self.turn_rules
            .iter()
            .filter_map(|r| r.evaluate_turn(turn, session_avg_cost))
            .collect()
    }

    pub fn analyze_session(&self, session: &SessionData) -> Vec<Tip> {
        self.session_rules
            .iter()
            .filter_map(|r| r.evaluate_session(session))
            .collect()
    }
}

// ── LLM-powered deep analysis ──────────────────────────────────

pub struct LlmAnalyzer;

impl LlmAnalyzer {
    pub fn build_analysis_prompt(turn: &TurnData, heuristic_tips: &[Tip]) -> String {
        let budget_util = if turn.token_budget > 0 {
            let used = turn.system_prompt_tokens + turn.memory_tokens + turn.history_tokens;
            (used as f64 / turn.token_budget as f64) * 100.0
        } else {
            0.0
        };

        let sys_pct = if turn.token_budget > 0 {
            (turn.system_prompt_tokens as f64 / turn.token_budget as f64) * 100.0
        } else {
            0.0
        };

        let mem_pct = if turn.token_budget > 0 {
            (turn.memory_tokens as f64 / turn.token_budget as f64) * 100.0
        } else {
            0.0
        };

        let tips_text = if heuristic_tips.is_empty() {
            "  (none)\n".to_string()
        } else {
            heuristic_tips
                .iter()
                .map(|t| format!("  - [{:?}] {}: {}", t.severity, t.rule_name, t.message))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "Analyze this LLM context turn:\n\
             - Model: {}\n\
             - Tokens in: {}, out: {}\n\
             - Budget utilization: {:.0}%\n\
             - System prompt: {:.0}% of budget\n\
             - Memory: {:.0}% of budget\n\
             - History depth: {} messages\n\
             - Tool calls: {} ({} failed)\n\
             - Complexity level: {}\n\
             - Cached: {}\n\
             \nExisting analysis tips:\n{}\n\
             \nProvide additional insights about:\n\
             1. System prompt clarity and specificity\n\
             2. Whether retrieved memories seem relevant\n\
             3. Suggestions for improving the configuration\n\
             4. Whether the model selection was appropriate",
            turn.model,
            turn.tokens_in,
            turn.tokens_out,
            budget_util,
            sys_pct,
            mem_pct,
            turn.history_depth,
            turn.tool_call_count,
            turn.tool_failure_count,
            turn.complexity_level,
            turn.cached,
            tips_text,
        )
    }

    pub fn build_session_prompt(session: &SessionData, heuristic_tips: &[Tip]) -> String {
        let total_cost: f64 = session.turns.iter().map(|t| t.cost).sum();
        let total_tokens: i64 = session
            .turns
            .iter()
            .map(|t| t.tokens_in + t.tokens_out)
            .sum();
        let models: HashSet<&str> = session.turns.iter().map(|t| t.model.as_str()).collect();

        let tips_text = if heuristic_tips.is_empty() {
            "  (none)\n".to_string()
        } else {
            heuristic_tips
                .iter()
                .map(|t| format!("  - [{:?}] {}: {}", t.severity, t.rule_name, t.message))
                .collect::<Vec<_>>()
                .join("\n")
        };

        format!(
            "Analyze this LLM session:\n\
             - Session ID: {}\n\
             - Total turns: {}\n\
             - Total tokens: {}\n\
             - Total cost: ${:.4}\n\
             - Models used: {}\n\
             \nExisting analysis tips:\n{}\n\
             \nProvide insights about:\n\
             1. Session-level cost efficiency\n\
             2. Context management patterns\n\
             3. Opportunities for optimization\n\
             4. Overall session health assessment",
            session.session_id,
            session.turns.len(),
            total_tokens,
            total_cost,
            models.into_iter().collect::<Vec<_>>().join(", "),
            tips_text,
        )
    }
}

// ── Tests ───────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_turn(overrides: impl FnOnce(&mut TurnData)) -> TurnData {
        let mut t = TurnData {
            turn_id: "turn-1".into(),
            token_budget: 128_000,
            system_prompt_tokens: 10_000,
            memory_tokens: 5_000,
            history_tokens: 20_000,
            history_depth: 10,
            complexity_level: "L1".into(),
            model: "gpt-4".into(),
            cost: 0.05,
            tokens_in: 35_000,
            tokens_out: 2_000,
            tool_call_count: 1,
            tool_failure_count: 0,
            thinking_length: 500,
            has_reasoning: true,
            cached: false,
        };
        overrides(&mut t);
        t
    }

    #[test]
    fn budget_pressure_fires_above_90_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 50;
            t.memory_tokens = 20;
            t.history_tokens = 25;
        });
        let tip = BudgetPressure.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn budget_pressure_critical_above_95_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 50;
            t.memory_tokens = 20;
            t.history_tokens = 27;
        });
        let tip = BudgetPressure.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn budget_pressure_silent_below_90_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 20;
            t.memory_tokens = 10;
            t.history_tokens = 30;
        });
        assert!(BudgetPressure.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn system_prompt_heavy_fires_above_40_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 45;
        });
        let tip = SystemPromptHeavy.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().category, RuleCategory::Prompt);
    }

    #[test]
    fn memory_starvation_fires_below_10_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 1000;
            t.memory_tokens = 50;
        });
        let tip = MemoryStarvation.evaluate_turn(&turn, None);
        assert!(tip.is_some());
    }

    #[test]
    fn shallow_history_fires_below_3() {
        let turn = make_turn(|t| {
            t.history_depth = 2;
        });
        let tip = ShallowHistory.evaluate_turn(&turn, None);
        assert!(tip.is_some());
    }

    #[test]
    fn high_tool_density_fires_above_3() {
        let turn = make_turn(|t| {
            t.tool_call_count = 5;
        });
        let tip = HighToolDensity.evaluate_turn(&turn, None);
        assert!(tip.is_some());
    }

    #[test]
    fn tool_failures_fires_on_failure() {
        let turn = make_turn(|t| {
            t.tool_call_count = 4;
            t.tool_failure_count = 3;
        });
        let tip = ToolFailures.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn expensive_turn_fires_above_2x_avg() {
        let turn = make_turn(|t| {
            t.cost = 0.10;
        });
        let tip = ExpensiveTurn.evaluate_turn(&turn, Some(0.03));
        assert!(tip.is_some());
    }

    #[test]
    fn expensive_turn_silent_without_avg() {
        let turn = make_turn(|_| {});
        assert!(ExpensiveTurn.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn empty_reasoning_fires_when_has_reasoning_but_empty() {
        let turn = make_turn(|t| {
            t.has_reasoning = true;
            t.thinking_length = 0;
        });
        let tip = EmptyReasoning.evaluate_turn(&turn, None);
        assert!(tip.is_some());
    }

    #[test]
    fn cached_turn_savings_fires_on_cache_hit() {
        let turn = make_turn(|t| {
            t.cached = true;
        });
        let tip = CachedTurnSavings.evaluate_turn(&turn, None);
        assert!(tip.is_some());
    }

    #[test]
    fn context_analyzer_produces_mixed_tips() {
        let analyzer = ContextAnalyzer::new();
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 50;
            t.memory_tokens = 5;
            t.history_tokens = 40;
            t.history_depth = 2;
            t.tool_call_count = 6;
            t.tool_failure_count = 2;
        });
        let tips = analyzer.analyze_turn(&turn, Some(0.01));
        assert!(
            tips.len() >= 3,
            "expected multiple tips, got {}",
            tips.len()
        );
    }

    fn make_session_turns(count: usize, modifier: impl Fn(usize, &mut TurnData)) -> SessionData {
        let turns: Vec<TurnData> = (0..count)
            .map(|i| {
                make_turn(|t| {
                    t.turn_id = format!("turn-{i}");
                    modifier(i, t);
                })
            })
            .collect();
        SessionData {
            turns,
            session_id: "session-1".into(),
            grades: vec![],
        }
    }

    #[test]
    fn context_drift_fires_when_utilization_increases() {
        let session = make_session_turns(8, |i, t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 10;
            t.memory_tokens = 5;
            t.history_tokens = if i < 4 { 20 } else { 70 };
        });
        let tip = ContextDrift.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn cost_acceleration_fires_when_costs_increase() {
        let session = make_session_turns(6, |i, t| {
            t.cost = if i < 3 { 0.01 } else { 0.10 };
        });
        let tip = CostAcceleration.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn underutilized_memory_fires_when_all_zero() {
        let session = make_session_turns(4, |_, t| {
            t.memory_tokens = 0;
        });
        let tip = UnderutilizedMemory.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn tool_success_rate_fires_below_80_pct() {
        let session = make_session_turns(3, |_, t| {
            t.tool_call_count = 5;
            t.tool_failure_count = 3;
        });
        let tip = ToolSuccessRate.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn model_churn_fires_above_3_models() {
        let models = ["gpt-4", "claude-3", "gemini-1.5", "llama-3"];
        let session = make_session_turns(4, |i, t| {
            t.model = models[i].into();
        });
        let tip = ModelChurn.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn full_session_analysis_returns_tips() {
        let analyzer = ContextAnalyzer::new();
        let session = make_session_turns(6, |i, t| {
            t.cost = if i < 3 { 0.01 } else { 0.10 };
            t.memory_tokens = 0;
        });
        let tips = analyzer.analyze_session(&session);
        assert!(!tips.is_empty());
    }

    #[test]
    fn llm_analyzer_build_prompt_not_empty() {
        let turn = make_turn(|_| {});
        let tips = vec![Tip {
            severity: Severity::Warning,
            category: RuleCategory::Budget,
            rule_name: "test".into(),
            message: "test message".into(),
            suggestion: "test suggestion".into(),
        }];
        let prompt = LlmAnalyzer::build_analysis_prompt(&turn, &tips);
        assert!(prompt.contains("gpt-4"));
        assert!(prompt.contains("test message"));
    }

    #[test]
    fn llm_analyzer_session_prompt_not_empty() {
        let session = make_session_turns(3, |_, _| {});
        let tips = vec![];
        let prompt = LlmAnalyzer::build_session_prompt(&session, &tips);
        assert!(prompt.contains("session-1"));
        assert!(prompt.contains("Total turns: 3"));
    }

    #[test]
    fn tip_serialization_roundtrip() {
        let tip = Tip {
            severity: Severity::Warning,
            category: RuleCategory::Cost,
            rule_name: "test_rule".into(),
            message: "test message".into(),
            suggestion: "test suggestion".into(),
        };
        let json = serde_json::to_string(&tip).unwrap();
        let back: Tip = serde_json::from_str(&json).unwrap();
        assert_eq!(back.severity, Severity::Warning);
        assert_eq!(back.category, RuleCategory::Cost);
        assert_eq!(back.rule_name, "test_rule");
    }

    #[test]
    fn analyzer_default_has_all_rules() {
        let analyzer = ContextAnalyzer::default();
        assert_eq!(analyzer.turn_rules.len(), 12);
        assert_eq!(analyzer.session_rules.len(), 10);
    }

    fn make_graded_session(
        count: usize,
        modifier: impl Fn(usize, &mut TurnData),
        grades: Vec<(String, i32)>,
    ) -> SessionData {
        let turns: Vec<TurnData> = (0..count)
            .map(|i| {
                make_turn(|t| {
                    t.turn_id = format!("turn-{i}");
                    modifier(i, t);
                })
            })
            .collect();
        SessionData {
            turns,
            session_id: "session-1".into(),
            grades,
        }
    }

    #[test]
    fn quality_declining_fires_when_grades_drop() {
        let grades: Vec<(String, i32)> = (0..8)
            .map(|i| {
                let grade = if i < 4 { 5 } else { 3 };
                (format!("turn-{i}"), grade)
            })
            .collect();
        let session = make_graded_session(8, |_, _| {}, grades);
        let tip = QualityDeclining.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn quality_declining_silent_when_stable() {
        let grades: Vec<(String, i32)> = (0..6).map(|i| (format!("turn-{i}"), 4)).collect();
        let session = make_graded_session(6, |_, _| {}, grades);
        assert!(QualityDeclining.evaluate_session(&session).is_none());
    }

    #[test]
    fn cost_quality_mismatch_fires() {
        let grades = vec![
            ("turn-0".into(), 3),
            ("turn-1".into(), 3),
            ("turn-2".into(), 5),
            ("turn-3".into(), 5),
        ];
        let session = make_graded_session(
            4,
            |i, t| {
                if i < 2 {
                    t.model = "expensive".into();
                    t.cost = 0.10;
                } else {
                    t.model = "cheap".into();
                    t.cost = 0.01;
                }
            },
            grades,
        );
        let tip = CostQualityMismatch.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn memory_helps_fires_when_significant() {
        let grades: Vec<(String, i32)> = (0..8)
            .map(|i| {
                let grade = if i < 4 { 2 } else { 5 };
                (format!("turn-{i}"), grade)
            })
            .collect();
        let session = make_graded_session(
            8,
            |i, t| {
                t.memory_tokens = if i < 4 { 0 } else { 500 };
            },
            grades,
        );
        let tip = MemoryHelps.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn low_coverage_warning_fires_below_20_pct() {
        let grades: Vec<(String, i32)> = (0..5).map(|i| (format!("turn-{i}"), 4)).collect();
        let session = make_graded_session(60, |_, _| {}, grades);
        let tip = LowCoverageWarning.evaluate_session(&session);
        assert!(tip.is_some());
    }

    #[test]
    fn low_coverage_warning_silent_for_small_sessions() {
        let session = make_graded_session(10, |_, _| {}, vec![]);
        assert!(LowCoverageWarning.evaluate_session(&session).is_none());
    }

    // ── Coverage for name() and category() on all turn rules ─────

    #[test]
    fn budget_pressure_name_and_category() {
        assert_eq!(BudgetPressure.name(), "budget_pressure");
        assert_eq!(BudgetPressure.category(), RuleCategory::Budget);
    }

    #[test]
    fn system_prompt_heavy_name_and_category() {
        assert_eq!(SystemPromptHeavy.name(), "system_prompt_heavy");
        assert_eq!(SystemPromptHeavy.category(), RuleCategory::Prompt);
    }

    #[test]
    fn memory_starvation_name_and_category() {
        assert_eq!(MemoryStarvation.name(), "memory_starvation");
        assert_eq!(MemoryStarvation.category(), RuleCategory::Memory);
    }

    #[test]
    fn shallow_history_name_and_category() {
        assert_eq!(ShallowHistory.name(), "shallow_history");
        assert_eq!(ShallowHistory.category(), RuleCategory::Quality);
    }

    #[test]
    fn high_tool_density_name_and_category() {
        assert_eq!(HighToolDensity.name(), "high_tool_density");
        assert_eq!(HighToolDensity.category(), RuleCategory::Tools);
    }

    #[test]
    fn tool_failures_name_and_category() {
        assert_eq!(ToolFailures.name(), "tool_failures");
        assert_eq!(ToolFailures.category(), RuleCategory::Tools);
    }

    #[test]
    fn expensive_turn_name_and_category() {
        assert_eq!(ExpensiveTurn.name(), "expensive_turn");
        assert_eq!(ExpensiveTurn.category(), RuleCategory::Cost);
    }

    #[test]
    fn empty_reasoning_name_and_category() {
        assert_eq!(EmptyReasoning.name(), "empty_reasoning");
        assert_eq!(EmptyReasoning.category(), RuleCategory::Quality);
    }

    #[test]
    fn system_prompt_tax_name_and_category() {
        assert_eq!(SystemPromptTax.name(), "system_prompt_tax");
        assert_eq!(SystemPromptTax.category(), RuleCategory::Cost);
    }

    #[test]
    fn history_cost_dominant_name_and_category() {
        assert_eq!(HistoryCostDominant.name(), "history_cost_dominant");
        assert_eq!(HistoryCostDominant.category(), RuleCategory::Cost);
    }

    #[test]
    fn large_output_ratio_name_and_category() {
        assert_eq!(LargeOutputRatio.name(), "large_output_ratio");
        assert_eq!(LargeOutputRatio.category(), RuleCategory::Cost);
    }

    #[test]
    fn cached_turn_savings_name_and_category() {
        assert_eq!(CachedTurnSavings.name(), "cached_turn_savings");
        assert_eq!(CachedTurnSavings.category(), RuleCategory::Cost);
    }

    // ── Coverage for name() and category() on session rules ──────

    #[test]
    fn context_drift_name_and_category() {
        assert_eq!(ContextDrift.name(), "context_drift");
        assert_eq!(ContextDrift.category(), RuleCategory::Budget);
    }

    #[test]
    fn frequent_escalation_name_and_category() {
        assert_eq!(FrequentEscalation.name(), "frequent_escalation");
        assert_eq!(FrequentEscalation.category(), RuleCategory::Quality);
    }

    #[test]
    fn cost_acceleration_name_and_category() {
        assert_eq!(CostAcceleration.name(), "cost_acceleration");
        assert_eq!(CostAcceleration.category(), RuleCategory::Cost);
    }

    #[test]
    fn underutilized_memory_name_and_category() {
        assert_eq!(UnderutilizedMemory.name(), "underutilized_memory");
        assert_eq!(UnderutilizedMemory.category(), RuleCategory::Memory);
    }

    #[test]
    fn tool_success_rate_name_and_category() {
        assert_eq!(ToolSuccessRate.name(), "tool_success_rate");
        assert_eq!(ToolSuccessRate.category(), RuleCategory::Tools);
    }

    #[test]
    fn model_churn_name_and_category() {
        assert_eq!(ModelChurn.name(), "model_churn");
        assert_eq!(ModelChurn.category(), RuleCategory::Quality);
    }

    #[test]
    fn quality_declining_name_and_category() {
        assert_eq!(QualityDeclining.name(), "quality_declining");
        assert_eq!(QualityDeclining.category(), RuleCategory::Quality);
    }

    #[test]
    fn cost_quality_mismatch_name_and_category() {
        assert_eq!(CostQualityMismatch.name(), "cost_quality_mismatch");
        assert_eq!(CostQualityMismatch.category(), RuleCategory::Cost);
    }

    #[test]
    fn memory_helps_name_and_category() {
        assert_eq!(MemoryHelps.name(), "memory_helps");
        assert_eq!(MemoryHelps.category(), RuleCategory::Memory);
    }

    #[test]
    fn low_coverage_warning_name_and_category() {
        assert_eq!(LowCoverageWarning.name(), "low_coverage_warning");
        assert_eq!(LowCoverageWarning.category(), RuleCategory::Quality);
    }

    // ── Coverage for SystemPromptTax evaluate_turn ────────────────

    #[test]
    fn system_prompt_tax_fires_when_expensive() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.system_prompt_tokens = 5_000;
            t.cost = 0.10;
        });
        let tip = SystemPromptTax.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        let tip = tip.unwrap();
        assert_eq!(tip.rule_name, "system_prompt_tax");
        assert!(tip.message.contains("System prompt"));
    }

    #[test]
    fn system_prompt_tax_warning_for_very_expensive() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.system_prompt_tokens = 5_000;
            t.cost = 0.20;
        });
        let tip = SystemPromptTax.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn system_prompt_tax_silent_for_zero_input() {
        let turn = make_turn(|t| {
            t.tokens_in = 0;
        });
        assert!(SystemPromptTax.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn system_prompt_tax_silent_for_low_cost() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.system_prompt_tokens = 100;
            t.cost = 0.005;
        });
        assert!(SystemPromptTax.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for HistoryCostDominant evaluate_turn ────────────

    #[test]
    fn history_cost_dominant_fires_above_60_pct() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.history_tokens = 7_000;
        });
        let tip = HistoryCostDominant.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.as_ref().unwrap().rule_name, "history_cost_dominant");
    }

    #[test]
    fn history_cost_dominant_warning_above_80_pct() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.history_tokens = 8_500;
        });
        let tip = HistoryCostDominant.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn history_cost_dominant_silent_for_zero_input() {
        let turn = make_turn(|t| {
            t.tokens_in = 0;
        });
        assert!(HistoryCostDominant.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn history_cost_dominant_silent_for_normal() {
        let turn = make_turn(|t| {
            t.tokens_in = 10_000;
            t.history_tokens = 3_000;
        });
        assert!(HistoryCostDominant.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for LargeOutputRatio evaluate_turn ───────────────

    #[test]
    fn large_output_ratio_fires_for_verbose() {
        let turn = make_turn(|t| {
            t.tokens_in = 1_000;
            t.tokens_out = 3_000;
        });
        let tip = LargeOutputRatio.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().rule_name, "large_output_ratio");
    }

    #[test]
    fn large_output_ratio_silent_for_small_output() {
        let turn = make_turn(|t| {
            t.tokens_in = 5_000;
            t.tokens_out = 1_000;
        });
        assert!(LargeOutputRatio.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn large_output_ratio_silent_for_zero_in() {
        let turn = make_turn(|t| {
            t.tokens_in = 0;
            t.tokens_out = 5_000;
        });
        assert!(LargeOutputRatio.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn large_output_ratio_silent_for_zero_out() {
        let turn = make_turn(|t| {
            t.tokens_in = 5_000;
            t.tokens_out = 0;
        });
        assert!(LargeOutputRatio.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for FrequentEscalation evaluate_session ──────────

    #[test]
    fn frequent_escalation_fires_above_40_pct() {
        let session = make_session_turns(10, |i, t| {
            t.complexity_level = if i < 5 { "L2".into() } else { "L3".into() };
        });
        let tip = FrequentEscalation.evaluate_session(&session);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn frequent_escalation_silent_below_40_pct() {
        let session = make_session_turns(10, |i, t| {
            t.complexity_level = if i < 2 { "L2".into() } else { "L0".into() };
        });
        assert!(FrequentEscalation.evaluate_session(&session).is_none());
    }

    #[test]
    fn frequent_escalation_silent_for_empty_session() {
        let session = SessionData {
            turns: vec![],
            session_id: "s".into(),
            grades: vec![],
        };
        assert!(FrequentEscalation.evaluate_session(&session).is_none());
    }

    // ── Coverage for ModelChurn edge cases ────────────────────────

    #[test]
    fn model_churn_silent_for_3_or_fewer_models() {
        let models = ["gpt-4", "claude-3", "gemini"];
        let session = make_session_turns(3, |i, t| {
            t.model = models[i].into();
        });
        assert!(ModelChurn.evaluate_session(&session).is_none());
    }

    #[test]
    fn model_churn_ignores_empty_model_names() {
        let session = make_session_turns(5, |_, t| {
            t.model = String::new();
        });
        assert!(ModelChurn.evaluate_session(&session).is_none());
    }

    // ── Coverage for ExpensiveTurn edge cases ─────────────────────

    #[test]
    fn expensive_turn_critical_above_5x() {
        let turn = make_turn(|t| {
            t.cost = 0.60;
        });
        let tip = ExpensiveTurn.evaluate_turn(&turn, Some(0.10));
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn expensive_turn_silent_for_zero_avg() {
        let turn = make_turn(|t| {
            t.cost = 0.10;
        });
        assert!(ExpensiveTurn.evaluate_turn(&turn, Some(0.0)).is_none());
    }

    #[test]
    fn expensive_turn_silent_below_2x() {
        let turn = make_turn(|t| {
            t.cost = 0.05;
        });
        assert!(ExpensiveTurn.evaluate_turn(&turn, Some(0.04)).is_none());
    }

    // ── Coverage for BudgetPressure zero budget ──────────────────

    #[test]
    fn budget_pressure_silent_for_zero_budget() {
        let turn = make_turn(|t| {
            t.token_budget = 0;
        });
        assert!(BudgetPressure.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for SystemPromptHeavy edge cases ────────────────

    #[test]
    fn system_prompt_heavy_critical_above_60_pct() {
        let turn = make_turn(|t| {
            t.token_budget = 100;
            t.system_prompt_tokens = 65;
        });
        let tip = SystemPromptHeavy.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn system_prompt_heavy_silent_for_zero_budget() {
        let turn = make_turn(|t| {
            t.token_budget = 0;
        });
        assert!(SystemPromptHeavy.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for MemoryStarvation edge cases ─────────────────

    #[test]
    fn memory_starvation_silent_for_zero_budget() {
        let turn = make_turn(|t| {
            t.token_budget = 0;
        });
        assert!(MemoryStarvation.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn memory_starvation_silent_for_high_memory() {
        let turn = make_turn(|t| {
            t.token_budget = 1000;
            t.memory_tokens = 200;
        });
        assert!(MemoryStarvation.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for ShallowHistory edge cases ───────────────────

    #[test]
    fn shallow_history_silent_for_depth_3_or_more() {
        let turn = make_turn(|t| {
            t.history_depth = 3;
        });
        assert!(ShallowHistory.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for HighToolDensity edge cases ──────────────────

    #[test]
    fn high_tool_density_warning_above_8() {
        let turn = make_turn(|t| {
            t.tool_call_count = 10;
        });
        let tip = HighToolDensity.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn high_tool_density_silent_for_3_or_fewer() {
        let turn = make_turn(|t| {
            t.tool_call_count = 3;
        });
        assert!(HighToolDensity.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for ToolFailures edge cases ─────────────────────

    #[test]
    fn tool_failures_warning_below_50_pct() {
        let turn = make_turn(|t| {
            t.tool_call_count = 4;
            t.tool_failure_count = 1;
        });
        let tip = ToolFailures.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Warning);
    }

    #[test]
    fn tool_failures_silent_for_no_failures() {
        let turn = make_turn(|t| {
            t.tool_call_count = 5;
            t.tool_failure_count = 0;
        });
        assert!(ToolFailures.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn tool_failures_handles_zero_total_calls() {
        let turn = make_turn(|t| {
            t.tool_call_count = 0;
            t.tool_failure_count = 1;
        });
        let tip = ToolFailures.evaluate_turn(&turn, None);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    // ── Coverage for EmptyReasoning edge cases ───────────────────

    #[test]
    fn empty_reasoning_silent_when_no_reasoning() {
        let turn = make_turn(|t| {
            t.has_reasoning = false;
            t.thinking_length = 0;
        });
        assert!(EmptyReasoning.evaluate_turn(&turn, None).is_none());
    }

    #[test]
    fn empty_reasoning_silent_when_thinking_present() {
        let turn = make_turn(|t| {
            t.has_reasoning = true;
            t.thinking_length = 500;
        });
        assert!(EmptyReasoning.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for CachedTurnSavings edge cases ────────────────

    #[test]
    fn cached_turn_savings_silent_when_not_cached() {
        let turn = make_turn(|t| {
            t.cached = false;
        });
        assert!(CachedTurnSavings.evaluate_turn(&turn, None).is_none());
    }

    // ── Coverage for ContextDrift edge cases ─────────────────────

    #[test]
    fn context_drift_silent_for_short_sessions() {
        let session = make_session_turns(3, |_, _| {});
        assert!(ContextDrift.evaluate_session(&session).is_none());
    }

    #[test]
    fn context_drift_silent_with_zero_budgets() {
        let session = make_session_turns(6, |_, t| {
            t.token_budget = 0;
        });
        assert!(ContextDrift.evaluate_session(&session).is_none());
    }

    // ── Coverage for CostAcceleration edge cases ─────────────────

    #[test]
    fn cost_acceleration_silent_for_short_sessions() {
        let session = make_session_turns(3, |_, _| {});
        assert!(CostAcceleration.evaluate_session(&session).is_none());
    }

    #[test]
    fn cost_acceleration_silent_when_stable() {
        let session = make_session_turns(6, |_, t| {
            t.cost = 0.05;
        });
        assert!(CostAcceleration.evaluate_session(&session).is_none());
    }

    // ── Coverage for ToolSuccessRate edge cases ──────────────────

    #[test]
    fn tool_success_rate_critical_below_50_pct() {
        let session = make_session_turns(3, |_, t| {
            t.tool_call_count = 5;
            t.tool_failure_count = 4;
        });
        let tip = ToolSuccessRate.evaluate_session(&session);
        assert!(tip.is_some());
        assert_eq!(tip.unwrap().severity, Severity::Critical);
    }

    #[test]
    fn tool_success_rate_silent_for_few_calls() {
        let session = make_session_turns(2, |_, t| {
            t.tool_call_count = 1;
            t.tool_failure_count = 1;
        });
        assert!(ToolSuccessRate.evaluate_session(&session).is_none());
    }

    // ── Coverage for UnderutilizedMemory edge cases ──────────────

    #[test]
    fn underutilized_memory_silent_for_empty_session() {
        let session = SessionData {
            turns: vec![],
            session_id: "s".into(),
            grades: vec![],
        };
        assert!(UnderutilizedMemory.evaluate_session(&session).is_none());
    }

    #[test]
    fn underutilized_memory_silent_when_some_memory() {
        let session = make_session_turns(4, |i, t| {
            t.memory_tokens = if i == 0 { 100 } else { 0 };
        });
        assert!(UnderutilizedMemory.evaluate_session(&session).is_none());
    }

    // ── Coverage for QualityDeclining edge cases ─────────────────

    #[test]
    fn quality_declining_silent_for_few_grades() {
        let grades = vec![("turn-0".into(), 5), ("turn-1".into(), 1)];
        let session = make_graded_session(4, |_, _| {}, grades);
        assert!(QualityDeclining.evaluate_session(&session).is_none());
    }

    // ── Coverage for CostQualityMismatch edge cases ──────────────

    #[test]
    fn cost_quality_mismatch_silent_for_empty_grades() {
        let session = make_graded_session(4, |_, _| {}, vec![]);
        assert!(CostQualityMismatch.evaluate_session(&session).is_none());
    }

    #[test]
    fn cost_quality_mismatch_silent_for_single_turn() {
        let grades = vec![("turn-0".into(), 5)];
        let session = make_graded_session(1, |_, _| {}, grades);
        assert!(CostQualityMismatch.evaluate_session(&session).is_none());
    }

    #[test]
    fn cost_quality_mismatch_silent_for_single_model() {
        let grades = vec![("turn-0".into(), 5), ("turn-1".into(), 4)];
        let session = make_graded_session(2, |_, _| {}, grades);
        assert!(CostQualityMismatch.evaluate_session(&session).is_none());
    }

    // ── Coverage for MemoryHelps edge cases ───────────────────────

    #[test]
    fn memory_helps_silent_for_empty_grades() {
        let session = make_graded_session(4, |_, _| {}, vec![]);
        assert!(MemoryHelps.evaluate_session(&session).is_none());
    }

    #[test]
    fn memory_helps_silent_when_insufficient_samples() {
        let grades = vec![("turn-0".into(), 5)];
        let session = make_graded_session(
            2,
            |i, t| {
                t.memory_tokens = if i == 0 { 100 } else { 0 };
            },
            grades,
        );
        assert!(MemoryHelps.evaluate_session(&session).is_none());
    }

    // ── Coverage for LlmAnalyzer prompts ─────────────────────────

    #[test]
    fn llm_analyzer_build_prompt_zero_budget() {
        let turn = make_turn(|t| {
            t.token_budget = 0;
        });
        let prompt = LlmAnalyzer::build_analysis_prompt(&turn, &[]);
        assert!(prompt.contains("Budget utilization: 0%"));
    }

    #[test]
    fn llm_analyzer_build_prompt_with_no_tips() {
        let turn = make_turn(|_| {});
        let prompt = LlmAnalyzer::build_analysis_prompt(&turn, &[]);
        assert!(prompt.contains("(none)"));
    }

    #[test]
    fn llm_analyzer_session_prompt_with_tips() {
        let session = make_session_turns(3, |_, _| {});
        let tips = vec![Tip {
            severity: Severity::Warning,
            category: RuleCategory::Cost,
            rule_name: "test_rule".into(),
            message: "test msg".into(),
            suggestion: "test sugg".into(),
        }];
        let prompt = LlmAnalyzer::build_session_prompt(&session, &tips);
        assert!(prompt.contains("test_rule"));
        assert!(prompt.contains("test msg"));
    }

    #[test]
    fn llm_analyzer_session_prompt_no_tips() {
        let session = make_session_turns(3, |_, _| {});
        let prompt = LlmAnalyzer::build_session_prompt(&session, &[]);
        assert!(prompt.contains("(none)"));
    }

    // ── Coverage for LowCoverageWarning fires ────────────────────

    #[test]
    fn low_coverage_warning_silent_when_good_coverage() {
        let grades: Vec<(String, i32)> = (0..50).map(|i| (format!("turn-{i}"), 4)).collect();
        let session = make_graded_session(60, |_, _| {}, grades);
        assert!(LowCoverageWarning.evaluate_session(&session).is_none());
    }
}
