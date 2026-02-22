use ironclad_llm::format::UnifiedMessage;

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
    // History and memories get trimmed if the budget is tight.
    let sys_tokens = estimate_tokens(system_prompt);
    messages.push(UnifiedMessage {
        role: "system".into(),
        content: system_prompt.to_string(),
    });
    used += sys_tokens;

    if !memories.is_empty() {
        let mem_tokens = estimate_tokens(memories);
        if used + mem_tokens <= budget {
            messages.push(UnifiedMessage {
                role: "system".into(),
                content: memories.to_string(),
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

    messages
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
        } else {
            break;
        }
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
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Hi there!".into(),
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
            },
            UnifiedMessage {
                role: "user".into(),
                content: "recent message".into(),
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
        });
        for i in 0..20 {
            msgs.push(UnifiedMessage {
                role: if i % 2 == 0 { "user" } else { "assistant" }.into(),
                content: format!("message {i}"),
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
        });
        for i in 0..10 {
            msgs.push(UnifiedMessage {
                role: "user".into(),
                content: format!("msg {i}"),
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
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "hello".into(),
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
            },
            UnifiedMessage {
                role: "user".into(),
                content: "hi".into(),
            },
        ];
        insert_compaction_summary(&mut msgs, "summary here".into());
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0].role, "system");
        assert_eq!(msgs[1].role, "system");
        assert!(msgs[1].content.contains("summary here"));
        assert_eq!(msgs[2].role, "user");
    }
}
