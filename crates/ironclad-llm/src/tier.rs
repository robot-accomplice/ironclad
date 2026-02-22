use ironclad_core::ModelTier;
use ironclad_core::config::TierAdaptConfig;

use crate::format::UnifiedMessage;

/// Classify a model by name using heuristic substring matching.
/// Prefer using the tier from provider config when available.
pub fn classify(model_name: &str) -> ModelTier {
    let lower = model_name.to_lowercase();

    if lower.contains("ollama")
        || lower.contains("local")
        || lower.contains("phi")
        || lower.contains("qwen")
    {
        ModelTier::T1
    } else if lower.contains("flash")
        || lower.contains("haiku")
        || (lower.contains("mini") && !lower.contains("gemini"))
        || lower.contains("moonshot")
    {
        ModelTier::T2
    } else if lower.contains("opus")
        || lower.contains("gpt-5")
        || lower.contains("o1")
        || lower.contains("o3")
    {
        ModelTier::T4
    } else if lower.contains("sonnet")
        || lower.contains("gpt-4")
        || lower.contains("codex")
        || lower.contains("gemini-2")
    {
        ModelTier::T3
    } else {
        ModelTier::T2
    }
}

/// Apply tier-appropriate adaptations to messages using the provided config.
pub fn adapt_for_tier(
    tier: ModelTier,
    messages: &mut Vec<UnifiedMessage>,
    config: &TierAdaptConfig,
) {
    match tier {
        ModelTier::T1 => adapt_t1(messages, config),
        ModelTier::T2 => adapt_t2(messages, config),
        ModelTier::T3 | ModelTier::T4 => {}
    }
}

fn adapt_t1(messages: &mut Vec<UnifiedMessage>, config: &TierAdaptConfig) {
    if config.t1_strip_system {
        messages.retain(|m| m.role != "system");
    }

    if config.t1_condense_turns && messages.len() > 2 {
        let combined = messages
            .iter()
            .map(|m| format!("[{}] {}", m.role, m.content))
            .collect::<Vec<_>>()
            .join("\n");
        messages.clear();
        messages.push(UnifiedMessage {
            role: "user".into(),
            content: combined,
        });
    }
}

fn adapt_t2(messages: &mut Vec<UnifiedMessage>, config: &TierAdaptConfig) {
    let has_system = messages.iter().any(|m| m.role == "system");
    if !has_system && let Some(ref preamble) = config.t2_default_preamble {
        messages.insert(
            0,
            UnifiedMessage {
                role: "system".into(),
                content: preamble.clone(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_adapt() -> TierAdaptConfig {
        TierAdaptConfig::default()
    }

    #[test]
    fn classify_known_models() {
        assert_eq!(classify("ollama/qwen3:8b"), ModelTier::T1);
        assert_eq!(classify("local-llama-3"), ModelTier::T1);
        assert_eq!(classify("phi-3-mini"), ModelTier::T1);

        assert_eq!(classify("gemini-2.5-flash"), ModelTier::T2);
        assert_eq!(classify("claude-3-haiku"), ModelTier::T2);
        assert_eq!(classify("gpt-4o-mini"), ModelTier::T2);

        assert_eq!(classify("claude-sonnet-4-20250514"), ModelTier::T3);
        assert_eq!(classify("gpt-4o"), ModelTier::T3);
        assert_eq!(classify("openai/gpt-5.3-codex"), ModelTier::T4);
        assert_eq!(classify("openai/codex-mini"), ModelTier::T2);
        assert_eq!(classify("gemini-2.5-pro"), ModelTier::T3);

        assert_eq!(classify("claude-opus-5"), ModelTier::T4);
        assert_eq!(classify("gpt-5-turbo"), ModelTier::T4);
        assert_eq!(classify("o1-preview"), ModelTier::T4);
        assert_eq!(classify("o3-mini"), ModelTier::T2);
        assert_eq!(classify("o3-pro"), ModelTier::T4);

        assert_eq!(classify("some-unknown-model"), ModelTier::T2);
    }

    #[test]
    fn t1_strips_system_and_condenses() {
        let cfg = default_adapt();
        let mut msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "You are helpful.".into(),
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Hello".into(),
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Hi!".into(),
            },
            UnifiedMessage {
                role: "user".into(),
                content: "How are you?".into(),
            },
        ];

        adapt_for_tier(ModelTier::T1, &mut msgs, &cfg);

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "user");
        assert!(!msgs[0].content.contains("system"));
        assert!(msgs[0].content.contains("Hello"));
        assert!(msgs[0].content.contains("How are you?"));
    }

    #[test]
    fn t1_config_disables_strip_and_condense() {
        let cfg = TierAdaptConfig {
            t1_strip_system: false,
            t1_condense_turns: false,
            ..Default::default()
        };
        let mut msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "Be helpful.".into(),
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Hello".into(),
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Hi!".into(),
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Bye".into(),
            },
        ];
        adapt_for_tier(ModelTier::T1, &mut msgs, &cfg);
        assert_eq!(msgs.len(), 4, "no condensing or stripping when disabled");
        assert_eq!(msgs[0].role, "system");
    }

    #[test]
    fn t2_adds_preamble() {
        let cfg = default_adapt();
        let mut msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "Hello".into(),
        }];

        adapt_for_tier(ModelTier::T2, &mut msgs, &cfg);

        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].role, "system");
        assert!(msgs[0].content.contains("concise"));
    }

    #[test]
    fn t2_custom_preamble() {
        let cfg = TierAdaptConfig {
            t2_default_preamble: Some("You are a pirate.".into()),
            ..Default::default()
        };
        let mut msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "Hello".into(),
        }];
        adapt_for_tier(ModelTier::T2, &mut msgs, &cfg);
        assert_eq!(msgs[0].content, "You are a pirate.");
    }

    #[test]
    fn t2_no_preamble_when_none() {
        let cfg = TierAdaptConfig {
            t2_default_preamble: None,
            ..Default::default()
        };
        let mut msgs = vec![UnifiedMessage {
            role: "user".into(),
            content: "Hello".into(),
        }];
        adapt_for_tier(ModelTier::T2, &mut msgs, &cfg);
        assert_eq!(
            msgs.len(),
            1,
            "no system message added when preamble is None"
        );
    }

    #[test]
    fn t3_t4_passthrough() {
        let cfg = default_adapt();
        let mut msgs = vec![
            UnifiedMessage {
                role: "system".into(),
                content: "You are an expert.".into(),
            },
            UnifiedMessage {
                role: "user".into(),
                content: "Explain quantum computing.".into(),
            },
        ];
        let original = msgs.clone();

        adapt_for_tier(ModelTier::T3, &mut msgs, &cfg);
        assert_eq!(msgs, original);

        adapt_for_tier(ModelTier::T4, &mut msgs, &cfg);
        assert_eq!(msgs, original);
    }
}
