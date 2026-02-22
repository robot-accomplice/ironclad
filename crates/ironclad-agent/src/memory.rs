use ironclad_core::config::MemoryConfig;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemoryBudgets {
    pub working: usize,
    pub episodic: usize,
    pub semantic: usize,
    pub procedural: usize,
    pub relationship: usize,
}

pub struct MemoryBudgetManager {
    config: MemoryConfig,
}

impl MemoryBudgetManager {
    pub fn new(config: MemoryConfig) -> Self {
        Self { config }
    }

    /// Distributes `total_tokens` across the five memory tiers based on config percentages.
    /// Any remainder from rounding is added to the working memory tier.
    pub fn allocate_budgets(&self, total_tokens: usize) -> MemoryBudgets {
        let working = pct(total_tokens, self.config.working_budget_pct);
        let episodic = pct(total_tokens, self.config.episodic_budget_pct);
        let semantic = pct(total_tokens, self.config.semantic_budget_pct);
        let procedural = pct(total_tokens, self.config.procedural_budget_pct);
        let relationship = pct(total_tokens, self.config.relationship_budget_pct);

        let allocated = working + episodic + semantic + procedural + relationship;
        let rollover = total_tokens.saturating_sub(allocated);

        MemoryBudgets {
            working: working + rollover,
            episodic,
            semantic,
            procedural,
            relationship,
        }
    }
}

fn pct(total: usize, percent: f64) -> usize {
    ((total as f64) * percent / 100.0).floor() as usize
}

// ── Post-turn memory ingestion ──────────────────────────────────

/// Classifies the type of a conversational turn for memory routing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnType {
    Reasoning,
    ToolUse,
    Creative,
    Financial,
    Social,
}

/// Classifies a turn based on user + assistant content and tool results.
pub fn classify_turn(
    user_msg: &str,
    assistant_msg: &str,
    tool_results: &[(String, String)],
) -> TurnType {
    if !tool_results.is_empty() {
        return TurnType::ToolUse;
    }
    let combined = format!("{user_msg} {assistant_msg}").to_lowercase();
    if combined.contains("transfer") || combined.contains("balance") || combined.contains("wallet")
        || combined.contains("payment") || combined.contains("usdc")
    {
        return TurnType::Financial;
    }
    if combined.contains("hello") || combined.contains("thanks") || combined.contains("please")
        || combined.contains("how are you")
    {
        return TurnType::Social;
    }
    if combined.contains("write a") || combined.contains("create a") || combined.contains("design a")
        || combined.contains("compose a") || combined.contains("draw") || combined.contains("generate a")
    {
        return TurnType::Creative;
    }
    TurnType::Reasoning
}

/// Ingests a completed turn into the appropriate memory tiers.
pub fn ingest_turn(
    db: &ironclad_db::Database,
    session_id: &str,
    user_msg: &str,
    assistant_msg: &str,
    tool_results: &[(String, String)],
) {
    let turn_type = classify_turn(user_msg, assistant_msg, tool_results);

    // Working memory: update active goals/context
    let summary = if assistant_msg.len() > 200 {
        &assistant_msg[..200]
    } else {
        assistant_msg
    };
    ironclad_db::memory::store_working(db, session_id, "turn_summary", summary, 3).ok();

    // Episodic: record significant events (tool use, financial operations)
    match turn_type {
        TurnType::ToolUse => {
            for (tool_name, result) in tool_results {
                let event = format!("Used tool '{tool_name}': {result}");
                ironclad_db::memory::store_episodic(db, "tool_use", &event, 7).ok();
            }
        }
        TurnType::Financial => {
            let event = format!("Financial interaction: {summary}");
            ironclad_db::memory::store_episodic(db, "financial", &event, 8).ok();
        }
        _ => {}
    }

    // Semantic: extract factual information from responses longer than a threshold
    if assistant_msg.len() > 100 && (turn_type == TurnType::Reasoning || turn_type == TurnType::Creative) {
        let key = format!("turn_{}", session_id.get(..8).unwrap_or("unknown"));
        ironclad_db::memory::store_semantic(db, "learned", &key, summary, 0.6).ok();
    }

    // Procedural: track tool success/failure
    if turn_type == TurnType::ToolUse {
        for (tool_name, _) in tool_results {
            ironclad_db::memory::record_procedural_success(db, tool_name).ok();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_config() -> MemoryConfig {
        MemoryConfig {
            working_budget_pct: 30.0,
            episodic_budget_pct: 25.0,
            semantic_budget_pct: 20.0,
            procedural_budget_pct: 15.0,
            relationship_budget_pct: 10.0,
            embedding_provider: None,
            embedding_model: None,
            hybrid_weight: 0.5,
        }
    }

    #[test]
    fn budget_allocation_matches_percentages() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(10_000);

        assert_eq!(budgets.working, 3_000);
        assert_eq!(budgets.episodic, 2_500);
        assert_eq!(budgets.semantic, 2_000);
        assert_eq!(budgets.procedural, 1_500);
        assert_eq!(budgets.relationship, 1_000);

        let sum = budgets.working
            + budgets.episodic
            + budgets.semantic
            + budgets.procedural
            + budgets.relationship;
        assert_eq!(sum, 10_000);
    }

    #[test]
    fn rollover_goes_to_working() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(99);

        let sum = budgets.working
            + budgets.episodic
            + budgets.semantic
            + budgets.procedural
            + budgets.relationship;
        assert_eq!(sum, 99, "all tokens must be distributed");
        assert!(budgets.working >= pct(99, 30.0));
    }

    #[test]
    fn zero_total_tokens() {
        let mgr = MemoryBudgetManager::new(default_config());
        let budgets = mgr.allocate_budgets(0);

        assert_eq!(
            budgets,
            MemoryBudgets {
                working: 0,
                episodic: 0,
                semantic: 0,
                procedural: 0,
                relationship: 0,
            }
        );
    }

    #[test]
    fn classify_turn_tool_use() {
        let results = vec![("echo".into(), "hello".into())];
        assert_eq!(classify_turn("test", "response", &results), TurnType::ToolUse);
    }

    #[test]
    fn classify_turn_financial() {
        assert_eq!(classify_turn("check my wallet balance", "Your balance is 42 USDC", &[]), TurnType::Financial);
    }

    #[test]
    fn classify_turn_social() {
        assert_eq!(classify_turn("hello how are you", "I'm great!", &[]), TurnType::Social);
    }

    #[test]
    fn classify_turn_creative() {
        assert_eq!(classify_turn("write a poem about rust", "Here's a poem...", &[]), TurnType::Creative);
    }

    #[test]
    fn classify_turn_reasoning() {
        assert_eq!(classify_turn("explain monads", "A monad is a design pattern...", &[]), TurnType::Reasoning);
    }

    #[test]
    fn ingest_turn_stores_memories() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent").unwrap();
        ingest_turn(
            &db,
            &session_id,
            "What is Rust?",
            "Rust is a systems programming language focused on safety and performance.",
            &[],
        );
        let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
        assert!(!working.is_empty(), "should store turn summary in working memory");
    }

    #[test]
    fn ingest_turn_with_tools_stores_episodic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent").unwrap();
        ironclad_db::memory::store_procedural(&db, "echo", "echo tool").ok();
        ingest_turn(
            &db,
            &session_id,
            "echo hello",
            "Tool says: hello",
            &[("echo".into(), "hello".into())],
        );
        let episodic = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        assert!(!episodic.is_empty(), "should store tool use in episodic memory");
    }
}
