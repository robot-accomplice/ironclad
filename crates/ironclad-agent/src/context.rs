use ironclad_llm::format::UnifiedMessage;

// ── Progressive compaction stages (OPENDEV pattern) ────────────────────────

/// Progressive compaction stages, ordered from least to most aggressive.
///
/// The OPENDEV paper demonstrates that staged compression outperforms
/// single-shot summarization because each stage preserves strictly more
/// information than the next, allowing the system to use the *least
/// aggressive* stage that fits the token budget.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CompactionStage {
    /// Stage 0: Full messages, no compression.
    Verbatim,
    /// Stage 1: Drop social filler (greetings, acks) but keep substantive content.
    SelectiveTrim,
    /// Stage 2: Apply entropy-based compression via `PromptCompressor` (~60% ratio).
    SemanticCompress,
    /// Stage 3: Reduce each message to its topic sentence.
    TopicExtract,
    /// Stage 4: Collapse entire conversation to a structural outline.
    Skeleton,
}

impl CompactionStage {
    /// Choose compaction stage based on how far over budget the content is.
    ///
    /// `excess_ratio` = current_tokens / target_tokens.
    /// A ratio of 1.0 means exactly at budget; >1.0 means over.
    pub fn from_excess(excess_ratio: f64) -> Self {
        if excess_ratio <= 1.0 {
            Self::Verbatim
        } else if excess_ratio <= 1.5 {
            Self::SelectiveTrim
        } else if excess_ratio <= 2.5 {
            Self::SemanticCompress
        } else if excess_ratio <= 4.0 {
            Self::TopicExtract
        } else {
            Self::Skeleton
        }
    }
}

/// Apply progressive compaction to a slice of messages at the requested stage.
///
/// System messages are always preserved. Higher stages produce shorter output.
pub fn compact_to_stage(
    messages: &[UnifiedMessage],
    stage: CompactionStage,
) -> Vec<UnifiedMessage> {
    match stage {
        CompactionStage::Verbatim => messages.to_vec(),
        CompactionStage::SelectiveTrim => selective_trim(messages),
        CompactionStage::SemanticCompress => semantic_compress(messages),
        CompactionStage::TopicExtract => topic_extract(messages),
        CompactionStage::Skeleton => skeleton_compress(messages),
    }
}

/// Stage 1: Drop messages that are pure social filler.
fn selective_trim(messages: &[UnifiedMessage]) -> Vec<UnifiedMessage> {
    const FILLER: &[&str] = &[
        "hello",
        "hi",
        "hey",
        "thanks",
        "thank you",
        "ok",
        "okay",
        "sure",
        "got it",
        "sounds good",
        "no problem",
        "np",
        "ack",
        "roger",
    ];
    messages
        .iter()
        .filter(|m| {
            if m.role == "system" {
                return true;
            }
            // Keep any message with substantive length
            if m.content.len() >= 40 {
                return true;
            }
            let lower = m.content.trim().to_lowercase();
            // Exact match (not substring) so "ok, I updated the schema" isn't
            // falsely classified as filler.
            !FILLER.contains(&lower.as_str())
        })
        .cloned()
        .collect()
}

/// Stage 2: Entropy-based compression on non-system messages ≥100 chars.
fn semantic_compress(messages: &[UnifiedMessage]) -> Vec<UnifiedMessage> {
    use ironclad_llm::compression::PromptCompressor;
    let compressor = PromptCompressor::new(0.6);
    messages
        .iter()
        .map(|m| {
            if m.role == "system" || m.content.len() < 100 {
                m.clone()
            } else {
                UnifiedMessage {
                    role: m.role.clone(),
                    content: compressor.compress(&m.content),
                    parts: None,
                }
            }
        })
        .collect()
}

/// Stage 3: Reduce each non-system message to its topic sentence.
fn topic_extract(messages: &[UnifiedMessage]) -> Vec<UnifiedMessage> {
    messages
        .iter()
        .map(|m| {
            if m.role == "system" {
                m.clone()
            } else {
                UnifiedMessage {
                    role: m.role.clone(),
                    content: extract_topic_sentence(&m.content),
                    parts: None,
                }
            }
        })
        .collect()
}

/// Stage 4: Collapse all non-system messages into a single skeleton outline.
fn skeleton_compress(messages: &[UnifiedMessage]) -> Vec<UnifiedMessage> {
    let topics: Vec<String> = messages
        .iter()
        .filter(|m| m.role != "system")
        .map(|m| {
            let topic = extract_topic_sentence(&m.content);
            format!("[{}] {}", m.role, topic)
        })
        .filter(|line| line.len() > 10)
        .collect();

    if topics.is_empty() {
        return messages
            .iter()
            .filter(|m| m.role == "system")
            .cloned()
            .collect();
    }

    let mut result: Vec<UnifiedMessage> = messages
        .iter()
        .filter(|m| m.role == "system")
        .cloned()
        .collect();
    result.push(UnifiedMessage {
        role: "assistant".into(),
        content: format!("[Conversation Skeleton]\n{}", topics.join("\n")),
        parts: None,
    });
    result
}

/// Extract the first sentence (up to 120 chars) from text.
fn extract_topic_sentence(text: &str) -> String {
    let end = text
        .find(". ")
        .or_else(|| text.find(".\n"))
        .or_else(|| text.find('?'))
        .or_else(|| text.find('!'))
        .map(|i| i + 1)
        .unwrap_or_else(|| text.len().min(120));
    text[..end.min(text.len())].trim().to_string()
}

// ── Complexity levels & context assembly ─────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComplexityLevel {
    L0,
    L1,
    L2,
    L3,
}

pub fn determine_level(complexity_score: f64) -> ComplexityLevel {
    if complexity_score < 0.3 {
        ComplexityLevel::L0
    } else if complexity_score < 0.6 {
        ComplexityLevel::L1
    } else if complexity_score < 0.9 {
        ComplexityLevel::L2
    } else {
        ComplexityLevel::L3
    }
}

pub fn token_budget(level: ComplexityLevel) -> usize {
    match level {
        ComplexityLevel::L0 => 4_000,
        ComplexityLevel::L1 => 8_000,
        ComplexityLevel::L2 => 16_000,
        ComplexityLevel::L3 => 32_000,
    }
}

/// Rough estimate: ~4 characters per token.
fn estimate_tokens(text: &str) -> usize {
    text.len().div_ceil(4)
}

/// Assembles context messages within the token budget for the given complexity level.
pub fn build_context(
    level: ComplexityLevel,
    system_prompt: &str,
    memories: &str,
    history: &[UnifiedMessage],
) -> Vec<UnifiedMessage> {
    let budget = token_budget(level);
    let mut used = 0usize;
    let mut messages = Vec::new();

    // System prompt is always included — it defines the agent's identity.
    // If the prompt exceeds the entire budget, truncate it to fit but never
    // drop it entirely (an agent without identity is worse than one with a
    // truncated identity).
    let sys_tokens = estimate_tokens(system_prompt);
    if sys_tokens <= budget {
        messages.push(UnifiedMessage {
            role: "system".into(),
            content: system_prompt.to_string(),
            parts: None,
        });
        used += sys_tokens;
    } else {
        // Truncate the system prompt to roughly fit the budget.  Each token
        // averages ~4 chars; we leave a small margin for the token estimator's
        // over/under-count.
        let max_chars = budget.saturating_mul(4);
        let truncated: String = system_prompt.chars().take(max_chars).collect();
        let truncated_tokens = estimate_tokens(&truncated);
        messages.push(UnifiedMessage {
            role: "system".into(),
            content: truncated,
            parts: None,
        });
        used += truncated_tokens;
        tracing::warn!(
            sys_tokens,
            budget,
            "system prompt exceeds budget — truncated to fit"
        );
    }

    if !memories.is_empty() {
        let mem_tokens = estimate_tokens(memories);
        if used + mem_tokens <= budget {
            messages.push(UnifiedMessage {
                role: "system".into(),
                content: memories.to_string(),
                parts: None,
            });
            used += mem_tokens;
        }
    }

    let mut history_buf: Vec<&UnifiedMessage> = Vec::new();
    let mut history_tokens = 0usize;

    for msg in history.iter().rev() {
        let msg_tokens = estimate_tokens(&msg.content);
        if used + history_tokens + msg_tokens > budget {
            break;
        }
        history_tokens += msg_tokens;
        history_buf.push(msg);
    }

    history_buf.reverse();
    for msg in history_buf {
        messages.push(msg.clone());
    }

    // Wire pruning path: if assembled context exceeds budget, soft-trim oldest
    // non-system messages while preserving recency.
    let prune_cfg = PruningConfig {
        max_tokens: budget,
        soft_trim_ratio: 1.0,
        ..PruningConfig::default()
    };
    if needs_pruning(&messages, &prune_cfg) {
        return soft_trim(&messages, &prune_cfg).messages;
    }

    messages
}

/// Inject an instruction anti-fade micro-reminder into the message list.
///
/// The OPENDEV paper shows that LLM instruction-following degrades as
/// conversation length grows — the system prompt fades from the model's
/// effective attention. This function injects a compact distillation of
/// key directives just before the final user message when the conversation
/// exceeds `ANTI_FADE_TURN_THRESHOLD` non-system turns.
///
/// Returns `true` if a reminder was injected, `false` otherwise.
pub fn inject_instruction_reminder(messages: &mut Vec<UnifiedMessage>, reminder: &str) -> bool {
    let non_system_turns = messages.iter().filter(|m| m.role != "system").count();
    if non_system_turns < crate::prompt::ANTI_FADE_TURN_THRESHOLD {
        return false;
    }

    // Find the last user message and inject the reminder just before it.
    // This puts the reminder in the "recency hotspot" where it most influences
    // the model's next generation.
    let insert_pos = messages
        .iter()
        .rposition(|m| m.role == "user")
        .unwrap_or(messages.len());

    messages.insert(
        insert_pos,
        UnifiedMessage {
            role: "system".into(),
            content: reminder.to_string(),
            parts: None,
        },
    );
    true
}

#[derive(Debug, Clone)]
pub struct PruningConfig {
    pub max_tokens: usize,
    pub soft_trim_ratio: f64,
    pub hard_clear_ratio: f64,
    pub preserve_recent: usize,
}

impl Default for PruningConfig {
    fn default() -> Self {
        Self {
            max_tokens: 128_000,
            soft_trim_ratio: 0.8,
            hard_clear_ratio: 0.95,
            preserve_recent: 10,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PruningResult {
    pub messages: Vec<UnifiedMessage>,
    pub trimmed_count: usize,
    pub compaction_summary: Option<String>,
    pub total_tokens: usize,
}

pub fn count_tokens(messages: &[UnifiedMessage]) -> usize {
    messages.iter().map(|m| estimate_tokens(&m.content)).sum()
}

pub fn needs_pruning(messages: &[UnifiedMessage], config: &PruningConfig) -> bool {
    let tokens = count_tokens(messages);
    tokens > ((config.max_tokens as f64 * config.soft_trim_ratio) as usize)
}

pub fn needs_hard_clear(messages: &[UnifiedMessage], config: &PruningConfig) -> bool {
    let tokens = count_tokens(messages);
    tokens > ((config.max_tokens as f64 * config.hard_clear_ratio) as usize)
}

/// Soft trim: remove oldest non-system messages while preserving the most recent N.
pub fn soft_trim(messages: &[UnifiedMessage], config: &PruningConfig) -> PruningResult {
    let target_tokens = (config.max_tokens as f64 * config.soft_trim_ratio) as usize;

    let system_msgs: Vec<_> = messages
        .iter()
        .filter(|m| m.role == "system")
        .cloned()
        .collect();

    let non_system: Vec<_> = messages
        .iter()
        .filter(|m| m.role != "system")
        .cloned()
        .collect();

    let preserve_count = config.preserve_recent.min(non_system.len());
    let preserved = &non_system[non_system.len().saturating_sub(preserve_count)..];

    let mut result: Vec<UnifiedMessage> = system_msgs;
    let system_tokens = count_tokens(&result);

    let mut available = target_tokens.saturating_sub(system_tokens);
    let mut kept = Vec::new();

    for msg in preserved.iter().rev() {
        let msg_tokens = estimate_tokens(&msg.content);
        if msg_tokens <= available {
            kept.push(msg.clone());
            available = available.saturating_sub(msg_tokens);
        }
        // Skip individual messages that exceed remaining budget rather
        // than breaking — older, smaller messages may still fit.
    }
    kept.reverse();

    let trimmed_count = non_system.len() - kept.len();
    result.extend(kept);

    let total_tokens = count_tokens(&result);

    PruningResult {
        messages: result,
        trimmed_count,
        compaction_summary: None,
        total_tokens,
    }
}

/// Extract messages that would be trimmed (for summarization).
pub fn extract_trimmable(
    messages: &[UnifiedMessage],
    config: &PruningConfig,
) -> Vec<UnifiedMessage> {
    let non_system: Vec<_> = messages
        .iter()
        .filter(|m| m.role != "system")
        .cloned()
        .collect();

    let preserve_count = config.preserve_recent.min(non_system.len());
    let trim_end = non_system.len().saturating_sub(preserve_count);

    non_system[..trim_end].to_vec()
}

/// Build a summarization prompt from trimmed messages.
pub fn build_compaction_prompt(trimmed: &[UnifiedMessage]) -> String {
    let mut prompt = String::from(
        "Summarize the following conversation history into a concise paragraph. \
         Capture key facts, decisions, and context. Do not include greetings or filler.\n\n",
    );

    for msg in trimmed {
        prompt.push_str(&format!("{}: {}\n", msg.role, msg.content));
    }

    prompt
}

/// Compress assembled context messages using the `PromptCompressor`.
///
/// System messages (prompt, memories) and older history get compressed.
/// The most recent user message is preserved intact so the LLM understands
/// the current query.  Messages under 50 tokens are skipped (not worth it).
pub fn compress_context(messages: &mut [UnifiedMessage], target_ratio: f64) {
    use ironclad_llm::compression::PromptCompressor;

    let compressor = PromptCompressor::new(target_ratio);

    // Find the last user message index — preserve it intact
    let last_user_idx = messages.iter().rposition(|m| m.role == "user");

    for (i, msg) in messages.iter_mut().enumerate() {
        if Some(i) == last_user_idx {
            continue; // preserve current query
        }
        // Only compress messages with enough content to be worth it (~50 tokens ≈ 200 chars)
        if msg.content.len() < 200 {
            continue;
        }
        msg.content = compressor.compress(&msg.content);
    }
}

/// Insert a compaction summary as a system message after the original system messages.
pub fn insert_compaction_summary(messages: &mut Vec<UnifiedMessage>, summary: String) {
    let insert_pos = messages
        .iter()
        .position(|m| m.role != "system")
        .unwrap_or(messages.len());

    messages.insert(
        insert_pos,
        UnifiedMessage {
            role: "system".into(),
            content: format!("[Conversation Summary] {summary}"),
            parts: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_determination() {
        assert_eq!(determine_level(0.0), ComplexityLevel::L0);
        assert_eq!(determine_level(0.29), ComplexityLevel::L0);
        assert_eq!(determine_level(0.3), ComplexityLevel::L1);
        assert_eq!(determine_level(0.59), ComplexityLevel::L1);
        assert_eq!(determine_level(0.6), ComplexityLevel::L2);
        assert_eq!(determine_level(0.89), ComplexityLevel::L2);
        assert_eq!(determine_level(0.9), ComplexityLevel::L3);
        assert_eq!(determine_level(1.0), ComplexityLevel::L3);
    }

    #[test]
    fn budget_values() {
        assert_eq!(token_budget(ComplexityLevel::L0), 4_000);
        assert_eq!(token_budget(ComplexityLevel::L1), 8_000);
        assert_eq!(token_budget(ComplexityLevel::L2), 16_000);
        assert_eq!(token_budget(ComplexityLevel::L3), 32_000);
    }

    #[test]
    fn context_assembly_respects_budget() {
        let sys = "You are a helpful agent.";
        let mem = "User prefers concise answers.";
        let history = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "Hello".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Hi there!".into(),
                parts: None,
            },
        ];

        let ctx = build_context(ComplexityLevel::L0, sys, mem, &history);

        assert!(!ctx.is_empty());
        assert_eq!(ctx[0].role, "system");
        assert_eq!(ctx[0].content, sys);

        let total_chars: usize = ctx.iter().map(|m| m.content.len()).sum();
        let total_tokens = total_chars.div_ceil(4);
        assert!(total_tokens <= token_budget(ComplexityLevel::L0));
    }

    #[test]
    fn context_truncates_old_history() {
        let sys = "System prompt";
        let mem = "";
        let big_msg = "x".repeat(8000);
        let history = vec![
            UnifiedMessage {
                role: "user".into(),
                content: big_msg,
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "recent message".into(),
                parts: None,
            },
        ];

        let ctx = build_context(ComplexityLevel::L0, sys, mem, &history);
        assert!(ctx.len() >= 2);
        assert_eq!(ctx.last().unwrap().content, "recent message");
    }

    #[test]
    fn pruning_config_defaults() {
        let cfg = PruningConfig::default();
        assert_eq!(cfg.max_tokens, 128_000);
        assert_eq!(cfg.soft_trim_ratio, 0.8);
        assert_eq!(cfg.hard_clear_ratio, 0.95);
        assert_eq!(cfg.preserve_recent, 10);
    }

    #[test]
    fn count_tokens_basic() {
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "hello world".into(),
            parts: None,
        }];
        let tokens = count_tokens(&msgs);
        assert!(tokens > 0);
        assert_eq!(tokens, estimate_tokens("hello world"));
    }

    #[test]
    fn needs_pruning_under_threshold() {
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "short".into(),
            parts: None,
        }];
        let cfg = PruningConfig::default();
        assert!(!needs_pruning(&msgs, &cfg));
    }

    #[test]
    fn needs_pruning_over_threshold() {
        let big = "x".repeat(500_000);
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: big,
            parts: None,
        }];
        let cfg = PruningConfig::default();
        assert!(needs_pruning(&msgs, &cfg));
    }

    #[test]
    fn soft_trim_preserves_recent() {
        let mut msgs = Vec::new();
        msgs.push(UnifiedMessage {
            role: "system".into(),
            content: "sys".into(),
            parts: None,
        });
        for i in 0..20 {
            msgs.push(UnifiedMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                content: format!("message {i}"),
                parts: None,
            });
        }

        let cfg = PruningConfig {
            max_tokens: 200,
            soft_trim_ratio: 0.8,
            preserve_recent: 5,
            ..Default::default()
        };

        let result = soft_trim(&msgs, &cfg);
        assert!(result.messages[0].role == "system");
        assert!(result.trimmed_count > 0);
        let last = result.messages.last().unwrap();
        assert_eq!(last.content, "message 19");
    }

    #[test]
    fn extract_trimmable_gets_old_messages() {
        let mut msgs = Vec::new();
        msgs.push(UnifiedMessage {
            role: "system".into(),
            content: "sys".into(),
            parts: None,
        });
        for i in 0..10 {
            msgs.push(UnifiedMessage {
                role: "user".into(),
                content: format!("msg {i}"),
                parts: None,
            });
        }

        let cfg = PruningConfig {
            preserve_recent: 3,
            ..Default::default()
        };
        let trimmed = extract_trimmable(&msgs, &cfg);
        assert_eq!(trimmed.len(), 7);
        assert_eq!(trimmed[0].content, "msg 0");
    }

    #[test]
    fn build_compaction_prompt_format() {
        let msgs = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "hi".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "hello".into(),
                parts: None,
            },
        ];
        let prompt = build_compaction_prompt(&msgs);
        assert!(prompt.contains("Summarize"));
        assert!(prompt.contains("user: hi"));
        assert!(prompt.contains("assistant: hello"));
    }

    #[test]
    fn insert_compaction_summary_placement() {
        let mut msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "sys".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "hi".into(),
                parts: None,
            },
        ];
        insert_compaction_summary(&mut msgs, "summary here".into());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "system");
        assert!(msgs[1].content.contains("summary here"));
        assert_eq!(msgs[2].role, "user");
    }

    #[test]
    fn needs_hard_clear_under_threshold() {
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "short".into(),
            parts: None,
        }];
        let cfg = PruningConfig::default();
        assert!(!needs_hard_clear(&msgs, &cfg));
    }

    #[test]
    fn needs_hard_clear_over_threshold() {
        // 128_000 * 0.95 = 121_600 tokens; each char ~0.25 tokens => 486_400 chars
        let big = "y".repeat(500_000);
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: big,
            parts: None,
        }];
        let cfg = PruningConfig::default();
        assert!(needs_hard_clear(&msgs, &cfg));
    }

    #[test]
    fn insert_compaction_summary_no_system_messages() {
        // When there are no system messages, summary should be inserted at position 0
        let mut msgs = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "hello".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "hi".into(),
                parts: None,
            },
        ];
        insert_compaction_summary(&mut msgs, "compacted info".into());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("compacted info"));
        assert_eq!(msgs[1].role, "user");
    }

    #[test]
    fn insert_compaction_summary_all_system_messages() {
        // When all messages are system messages, summary is appended at the end
        let mut msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "sys1".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "system".into(),
                content: "sys2".into(),
                parts: None,
            },
        ];
        insert_compaction_summary(&mut msgs, "final summary".into());
        assert_eq!(msgs.len(), 3);
        // Insert at position 2 (len since no non-system found)
        assert_eq!(msgs[2].role, "system");
        assert!(msgs[2].content.contains("final summary"));
    }

    #[test]
    fn build_context_sys_prompt_exceeds_budget() {
        // System prompt is enormous relative to L0 budget (4000 tokens ~ 16000 chars)
        let big_sys = "z".repeat(20_000);
        let mem = "";
        let history = vec![UnifiedMessage {
            role: "user".into(),
            content: "hi".into(),
            parts: None,
        }];

        let ctx = build_context(ComplexityLevel::L0, &big_sys, mem, &history);
        // System prompt is truncated to fit, never dropped entirely.
        assert!(!ctx.is_empty());
        assert_eq!(ctx[0].role, "system");
        // The truncated content must be shorter than the original.
        assert!(ctx[0].content.len() < big_sys.len());
        // But still non-empty — agent always gets some identity.
        assert!(!ctx[0].content.is_empty());
    }

    #[test]
    fn build_context_empty_history() {
        let sys = "Agent prompt";
        let mem = "Memory info";
        let history: Vec<UnifiedMessage> = vec![];

        let ctx = build_context(ComplexityLevel::L1, sys, mem, &history);
        assert_eq!(ctx.len(), 2); // system + memories
        assert_eq!(ctx[0].content, sys);
        assert_eq!(ctx[1].content, mem);
    }

    #[test]
    fn soft_trim_no_non_system_messages() {
        let msgs = vec![UnifiedMessage {
            role: "system".into(),
            content: "sys".into(),
            parts: None,
        }];
        let cfg = PruningConfig {
            max_tokens: 200,
            preserve_recent: 5,
            ..Default::default()
        };
        let result = soft_trim(&msgs, &cfg);
        assert_eq!(result.messages.len(), 1);
        assert_eq!(result.trimmed_count, 0);
    }

    #[test]
    fn extract_trimmable_fewer_than_preserve() {
        let msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "only one".into(),
            parts: None,
        }];
        let cfg = PruningConfig {
            preserve_recent: 5,
            ..Default::default()
        };
        let trimmed = extract_trimmable(&msgs, &cfg);
        assert!(
            trimmed.is_empty(),
            "nothing to trim if fewer than preserve_recent"
        );
    }

    #[test]
    fn count_tokens_empty() {
        assert_eq!(count_tokens(&[]), 0);
    }

    // ── CompactionStage tests ──────────────────────────────────────────

    #[test]
    fn compaction_stage_from_excess_boundaries() {
        assert_eq!(CompactionStage::from_excess(0.5), CompactionStage::Verbatim);
        assert_eq!(CompactionStage::from_excess(1.0), CompactionStage::Verbatim);
        assert_eq!(
            CompactionStage::from_excess(1.01),
            CompactionStage::SelectiveTrim
        );
        assert_eq!(
            CompactionStage::from_excess(1.5),
            CompactionStage::SelectiveTrim
        );
        assert_eq!(
            CompactionStage::from_excess(1.51),
            CompactionStage::SemanticCompress
        );
        assert_eq!(
            CompactionStage::from_excess(2.5),
            CompactionStage::SemanticCompress
        );
        assert_eq!(
            CompactionStage::from_excess(2.51),
            CompactionStage::TopicExtract
        );
        assert_eq!(
            CompactionStage::from_excess(4.0),
            CompactionStage::TopicExtract
        );
        assert_eq!(
            CompactionStage::from_excess(4.01),
            CompactionStage::Skeleton
        );
        assert_eq!(
            CompactionStage::from_excess(100.0),
            CompactionStage::Skeleton
        );
    }

    #[test]
    fn compaction_stage_ordering() {
        assert!(CompactionStage::Verbatim < CompactionStage::SelectiveTrim);
        assert!(CompactionStage::SelectiveTrim < CompactionStage::SemanticCompress);
        assert!(CompactionStage::SemanticCompress < CompactionStage::TopicExtract);
        assert!(CompactionStage::TopicExtract < CompactionStage::Skeleton);
    }

    #[test]
    fn selective_trim_removes_filler() {
        let msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "sys prompt".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "hello".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "ok".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Please analyze the data and find anomalies in the revenue stream".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "thanks".into(),
                parts: None,
            },
        ];
        let result = selective_trim(&msgs);
        // System message always kept, substantive user message kept (>=40 chars),
        // filler "hello", "ok", "thanks" dropped
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].role, "system");
        assert!(result[1].content.contains("analyze the data"));
    }

    #[test]
    fn selective_trim_keeps_all_long_messages() {
        let msgs = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "This is a long enough message that should never be trimmed away".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "I agree, this response is also long enough to stay around".into(),
                parts: None,
            },
        ];
        let result = selective_trim(&msgs);
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn topic_extract_takes_first_sentence() {
        let msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content:
                    "Deploy the model to production. Then run the test suite. Finally update docs."
                        .into(),
                parts: None,
            },
        ];
        let result = topic_extract(&msgs);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "You are helpful."); // system preserved
        assert_eq!(result[1].content, "Deploy the model to production."); // first sentence
    }

    #[test]
    fn skeleton_compress_creates_outline() {
        let msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "System prompt".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "How does authentication work in this app?".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Authentication uses JWT tokens with a 24-hour expiry. The flow starts at the login endpoint.".into(),
                parts: None,
            },
        ];
        let result = skeleton_compress(&msgs);
        // System message preserved + one skeleton assistant message
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].content, "System prompt");
        assert_eq!(result[1].role, "assistant");
        assert!(result[1].content.contains("[Conversation Skeleton]"));
        assert!(result[1].content.contains("[user]"));
        assert!(result[1].content.contains("[assistant]"));
    }

    #[test]
    fn skeleton_compress_empty_non_system() {
        let msgs = vec![UnifiedMessage {
            role: "system".into(),
            content: "sys".into(),
            parts: None,
        }];
        let result = skeleton_compress(&msgs);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].role, "system");
    }

    #[test]
    fn compact_to_stage_verbatim_is_identity() {
        let msgs = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "test".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "resp".into(),
                parts: None,
            },
        ];
        let result = compact_to_stage(&msgs, CompactionStage::Verbatim);
        assert_eq!(result.len(), msgs.len());
        assert_eq!(result[0].content, "test");
        assert_eq!(result[1].content, "resp");
    }

    #[test]
    fn compact_to_stage_dispatches_correctly() {
        let msgs = vec![
            UnifiedMessage {
                role: "user".into(),
                content: "hi".into(),
                parts: None,
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Analyze the market data and identify trends in revenue growth over Q3"
                    .into(),
                parts: None,
            },
        ];
        // SelectiveTrim should remove the "hi" filler
        let trimmed = compact_to_stage(&msgs, CompactionStage::SelectiveTrim);
        assert_eq!(trimmed.len(), 1);
        assert!(trimmed[0].content.contains("Analyze"));
    }

    #[test]
    fn extract_topic_sentence_with_period() {
        assert_eq!(
            extract_topic_sentence("First sentence. Second sentence. Third."),
            "First sentence."
        );
    }

    #[test]
    fn extract_topic_sentence_with_question() {
        assert_eq!(
            extract_topic_sentence("What is this? More details here."),
            "What is this?"
        );
    }

    #[test]
    fn extract_topic_sentence_no_punctuation() {
        let short = "Just some text without ending";
        assert_eq!(extract_topic_sentence(short), short);
    }

    #[test]
    fn extract_topic_sentence_very_long() {
        let long = "x".repeat(200);
        let result = extract_topic_sentence(&long);
        assert!(result.len() <= 120);
    }

    // ── Anti-fade injection tests ───────────────────────────────────────

    fn make_msg(role: &str, content: &str) -> UnifiedMessage {
        UnifiedMessage {
            role: role.into(),
            content: content.into(),
            parts: None,
        }
    }

    #[test]
    fn inject_reminder_skips_short_conversations() {
        let mut msgs = vec![
            make_msg("system", "You are helpful."),
            make_msg("user", "Hello"),
            make_msg("assistant", "Hi!"),
            make_msg("user", "How are you?"),
            make_msg("assistant", "Good, thanks!"),
        ];
        // Only 4 non-system turns, below threshold of 8
        let injected = inject_instruction_reminder(&mut msgs, "[Reminder] Be helpful.");
        assert!(!injected);
        assert_eq!(msgs.len(), 5);
    }

    #[test]
    fn inject_reminder_fires_for_long_conversations() {
        let mut msgs = vec![make_msg("system", "You are helpful.")];
        // Add 10 user/assistant pairs (20 non-system turns)
        for i in 0..10 {
            msgs.push(make_msg("user", &format!("question {i}")));
            msgs.push(make_msg("assistant", &format!("answer {i}")));
        }
        let len_before = msgs.len();
        let injected = inject_instruction_reminder(&mut msgs, "[Reminder] Always be thorough.");
        assert!(injected);
        assert_eq!(msgs.len(), len_before + 1);

        // The reminder should be inserted just before the last user message
        let last_user_idx = msgs.iter().rposition(|m| m.role == "user").unwrap();
        assert_eq!(msgs[last_user_idx - 1].role, "system");
        assert!(
            msgs[last_user_idx - 1]
                .content
                .contains("[Reminder] Always be thorough.")
        );
    }

    #[test]
    fn inject_reminder_places_before_last_user_message() {
        let mut msgs = vec![make_msg("system", "System prompt.")];
        for i in 0..5 {
            msgs.push(make_msg("user", &format!("q{i}")));
            msgs.push(make_msg("assistant", &format!("a{i}")));
        }
        // Final user message
        msgs.push(make_msg("user", "final question"));

        let injected = inject_instruction_reminder(&mut msgs, "[Reminder] Key directive.");
        assert!(injected);

        // Last message should still be the user's final question
        assert_eq!(msgs.last().unwrap().content, "final question");
        assert_eq!(msgs.last().unwrap().role, "user");

        // Second-to-last should be the reminder
        let second_last = &msgs[msgs.len() - 2];
        assert_eq!(second_last.role, "system");
        assert!(second_last.content.contains("[Reminder]"));
    }

    #[test]
    fn inject_reminder_no_user_messages_appends_at_end() {
        let mut msgs = vec![make_msg("system", "System prompt.")];
        // Add only assistant messages (unusual but tests edge case)
        for i in 0..10 {
            msgs.push(make_msg("assistant", &format!("response {i}")));
        }
        let len_before = msgs.len();
        let injected = inject_instruction_reminder(&mut msgs, "[Reminder] Test.");
        assert!(injected);
        // When no user message found, inserts at the end
        assert_eq!(msgs.len(), len_before + 1);
        assert_eq!(msgs.last().unwrap().content, "[Reminder] Test.");
    }
}
