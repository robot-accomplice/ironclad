use ironclad_core::config::MemoryConfig;
use tracing::warn;

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
    // BUG-08: Only check user_msg for financial keywords, and require >= 2 matches
    // to avoid false-positives (e.g. "balance" in generic error messages).
    let user_lower = user_msg.to_lowercase();
    let financial_keywords = [
        "transfer",
        "balance",
        "wallet",
        "payment",
        "usdc",
        "send funds",
    ];
    let financial_hits = financial_keywords
        .iter()
        .filter(|kw| user_lower.contains(*kw))
        .count();
    if financial_hits >= 2 {
        return TurnType::Financial;
    }
    let combined = format!("{user_msg} {assistant_msg}").to_lowercase();
    if combined.contains("hello")
        || combined.contains("thanks")
        || combined.contains("please")
        || combined.contains("how are you")
    {
        return TurnType::Social;
    }
    if combined.contains("write a")
        || combined.contains("create a")
        || combined.contains("design a")
        || combined.contains("compose a")
        || combined.contains("draw")
        || combined.contains("generate a")
    {
        return TurnType::Creative;
    }
    TurnType::Reasoning
}

/// Ingests a completed turn into the appropriate memory tiers.
///
/// # Silent Degradation
///
/// This function returns `()` by design: each `db.store_*()` call is
/// independently wrapped in `if let Err(e) = ... { warn!(...) }`, so any
/// combination of memory-tier writes can fail without aborting the turn.
/// This is intentional -- memory ingestion runs in a background
/// `tokio::spawn` and must not block the response path.  A future
/// improvement could return a count of failed operations for
/// observability (see BUG-060 in the bug ledger).
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
        &assistant_msg[..assistant_msg.floor_char_boundary(200)]
    } else {
        assistant_msg
    };
    if let Err(e) = ironclad_db::memory::store_working(db, session_id, "turn_summary", summary, 3) {
        warn!(error = %e, "failed to store working memory");
    }

    // Episodic: record significant events (tool use, financial operations)
    match turn_type {
        TurnType::ToolUse => {
            for (tool_name, result) in tool_results {
                let event = format!("Used tool '{tool_name}': {result}");
                if let Err(e) = ironclad_db::memory::store_episodic(db, "tool_use", &event, 7) {
                    warn!(error = %e, "failed to store episodic tool_use memory");
                }
            }
        }
        TurnType::Financial => {
            let event = format!("Financial interaction: {summary}");
            if let Err(e) = ironclad_db::memory::store_episodic(db, "financial", &event, 8) {
                warn!(error = %e, "failed to store episodic financial memory");
            }
        }
        _ => {}
    }

    // Semantic: extract factual information from responses longer than a threshold
    if assistant_msg.len() > 100
        && (turn_type == TurnType::Reasoning || turn_type == TurnType::Creative)
    {
        let key = format!("turn_{}", session_id.get(..8).unwrap_or("unknown"));
        if let Err(e) = ironclad_db::memory::store_semantic(db, "learned", &key, summary, 0.6) {
            warn!(error = %e, "failed to store semantic memory");
        }
    }

    // Procedural: track tool success/failure
    if turn_type == TurnType::ToolUse {
        for (tool_name, _) in tool_results {
            if let Err(e) = ironclad_db::memory::record_procedural_success(db, tool_name) {
                warn!(error = %e, tool = %tool_name, "failed to record procedural success");
            }
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
            ann_index: false,
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
        assert_eq!(
            classify_turn("test", "response", &results),
            TurnType::ToolUse
        );
    }

    #[test]
    fn classify_turn_financial() {
        assert_eq!(
            classify_turn("check my wallet balance", "Your balance is 42 USDC", &[]),
            TurnType::Financial
        );
    }

    #[test]
    fn classify_turn_social() {
        assert_eq!(
            classify_turn("hello how are you", "I'm great!", &[]),
            TurnType::Social
        );
    }

    #[test]
    fn classify_turn_creative() {
        assert_eq!(
            classify_turn("write a poem about rust", "Here's a poem...", &[]),
            TurnType::Creative
        );
    }

    #[test]
    fn classify_turn_reasoning() {
        assert_eq!(
            classify_turn("explain monads", "A monad is a design pattern...", &[]),
            TurnType::Reasoning
        );
    }

    #[test]
    fn ingest_turn_stores_memories() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        ingest_turn(
            &db,
            &session_id,
            "What is Rust?",
            "Rust is a systems programming language focused on safety and performance.",
            &[],
        );
        let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
        assert!(
            !working.is_empty(),
            "should store turn summary in working memory"
        );
    }

    #[test]
    fn ingest_turn_with_tools_stores_episodic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        ironclad_db::memory::store_procedural(&db, "echo", "echo tool").ok();
        ingest_turn(
            &db,
            &session_id,
            "echo hello",
            "Tool says: hello",
            &[("echo".into(), "hello".into())],
        );
        let episodic = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        assert!(
            !episodic.is_empty(),
            "should store tool use in episodic memory"
        );
    }

    #[test]
    fn ingest_turn_financial_stores_episodic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        ingest_turn(
            &db,
            &session_id,
            "check my wallet balance",
            "Your balance is 42 USDC",
            &[],
        );
        let episodic = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        assert!(
            !episodic.is_empty(),
            "financial turn should store episodic memory"
        );
        assert!(
            episodic
                .iter()
                .any(|e| e.content.contains("Financial interaction")),
            "should prefix with 'Financial interaction'"
        );
    }

    #[test]
    fn ingest_turn_long_reasoning_stores_semantic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        // assistant_msg > 100 chars + Reasoning turn type -> stores semantic
        let long_response = "A ".repeat(60); // 120 chars
        ingest_turn(&db, &session_id, "explain monads", &long_response, &[]);
        let semantic = ironclad_db::memory::retrieve_semantic(&db, "learned").unwrap();
        assert!(
            !semantic.is_empty(),
            "long reasoning turn should store semantic memory"
        );
    }

    #[test]
    fn ingest_turn_long_creative_stores_semantic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        let long_response = "B ".repeat(60); // 120 chars
        ingest_turn(
            &db,
            &session_id,
            "write a poem about Rust",
            &long_response,
            &[],
        );
        let semantic = ironclad_db::memory::retrieve_semantic(&db, "learned").unwrap();
        assert!(
            !semantic.is_empty(),
            "long creative turn should store semantic memory"
        );
    }

    #[test]
    fn ingest_turn_short_reasoning_skips_semantic() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        // assistant_msg <= 100 chars => no semantic storage
        ingest_turn(&db, &session_id, "explain monads", "short answer", &[]);
        let semantic = ironclad_db::memory::retrieve_semantic(&db, "learned").unwrap();
        assert!(
            semantic.is_empty(),
            "short reasoning turn should not store semantic memory"
        );
    }

    #[test]
    fn ingest_turn_truncates_long_summary() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        // assistant_msg > 200 chars -> summary truncated to first 200
        let long_response = "X".repeat(300);
        ingest_turn(&db, &session_id, "explain something", &long_response, &[]);
        let working = ironclad_db::memory::retrieve_working(&db, &session_id).unwrap();
        assert!(!working.is_empty());
        // The stored summary should be at most 200 chars
        for entry in &working {
            assert!(
                entry.content.len() <= 200,
                "working memory summary should be truncated to 200 chars, got {}",
                entry.content.len()
            );
        }
    }

    #[test]
    fn ingest_turn_records_procedural_success() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent", None).unwrap();
        ironclad_db::memory::store_procedural(&db, "custom_tool", "a tool").ok();
        ingest_turn(
            &db,
            &session_id,
            "use custom_tool",
            "done",
            &[("custom_tool".into(), "success".into())],
        );
        // This exercises the procedural success recording path
        // The test passes if no panic occurs
    }

    #[test]
    fn truncation_emoji_at_boundary() {
        // 🦀 is 4 bytes; 198 ASCII + 🦀 = 202 bytes, slice at 200 would split the emoji
        let msg = format!("{}{}", "A".repeat(198), "🦀");
        assert!(msg.len() == 202);
        let summary = if msg.len() > 200 {
            &msg[..msg.floor_char_boundary(200)]
        } else {
            &msg
        };
        assert!(summary.len() <= 200);
        assert!(summary.is_char_boundary(summary.len()));
    }

    #[test]
    fn truncation_cjk_near_boundary() {
        // CJK characters are 3 bytes each; 199 ASCII + 中 = 202 bytes
        let msg = format!("{}{}", "B".repeat(199), "中");
        assert!(msg.len() == 202);
        let summary = if msg.len() > 200 {
            &msg[..msg.floor_char_boundary(200)]
        } else {
            &msg
        };
        assert!(summary.len() <= 200);
        assert!(summary.is_char_boundary(summary.len()));
    }

    #[test]
    fn truncation_ascii_over_200() {
        let msg = "C".repeat(300);
        let summary = if msg.len() > 200 {
            &msg[..msg.floor_char_boundary(200)]
        } else {
            &msg
        };
        assert_eq!(summary.len(), 200);
    }

    #[test]
    fn classify_turn_financial_payment() {
        // BUG-08: need >= 2 financial keywords to classify as Financial
        assert_eq!(
            classify_turn(
                "make a payment of $50 from wallet",
                "Processing payment",
                &[]
            ),
            TurnType::Financial
        );
    }

    #[test]
    fn classify_turn_financial_transfer() {
        assert_eq!(
            classify_turn("transfer 10 USDC", "Transferring...", &[]),
            TurnType::Financial
        );
    }

    #[test]
    fn classify_turn_creative_compose() {
        assert_eq!(
            classify_turn("compose a sonnet", "Here is your sonnet...", &[]),
            TurnType::Creative
        );
    }

    #[test]
    fn classify_turn_creative_design() {
        assert_eq!(
            classify_turn("design a logo concept", "Here's the concept...", &[]),
            TurnType::Creative
        );
    }

    #[test]
    fn classify_turn_creative_generate() {
        assert_eq!(
            classify_turn("generate a story", "Once upon a time...", &[]),
            TurnType::Creative
        );
    }

    #[test]
    fn classify_turn_social_thanks() {
        assert_eq!(
            classify_turn("thanks for your help", "You're welcome!", &[]),
            TurnType::Social
        );
    }

    #[test]
    fn classify_turn_tool_use_takes_precedence() {
        // Even if content matches financial keywords, tool_results non-empty -> ToolUse
        assert_eq!(
            classify_turn(
                "check my wallet balance",
                "Done",
                &[("wallet".into(), "42".into())]
            ),
            TurnType::ToolUse
        );
    }
}
