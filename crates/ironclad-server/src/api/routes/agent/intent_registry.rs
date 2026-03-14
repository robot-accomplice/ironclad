//! Unified intent classification registry.
//!
//! Replaces the 22 scattered `requests_*()` functions in `intents.rs` and
//! 3 local intent detectors in `channel_message.rs` with a single
//! [`IntentRegistry::classify()`] entry point that lowercases the prompt
//! exactly once and evaluates all registered descriptors.

use std::collections::HashSet;

// ── Intent enum ──────────────────────────────────────────────────────────

/// All classifiable user intents. Each variant maps 1:1 to a former
/// `requests_*()` function or channel-local intent detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum Intent {
    /// General tool-use or execution request (broad catch-all).
    Execution,
    /// Explicit delegation to a subagent.
    Delegation,
    /// Cron job scheduling request.
    Cron,
    /// File distribution listing.
    FileDistribution,
    /// Folder scan / directory listing.
    FolderScan,
    /// "Pick a random tool and use it" requests.
    RandomToolUse,
    /// Model identity query ("/status", "what model").
    ModelIdentity,
    /// Current events / geopolitical sitrep.
    CurrentEvents,
    /// Introspection of available tools and subagents.
    Introspection,
    /// Terse acknowledgement + wait pattern.
    Acknowledgement,
    /// LLM provider inventory query.
    ProviderInventory,
    /// Personality / identity profile query.
    PersonalityProfile,
    /// Capability summary ("what can you do?").
    CapabilitySummary,
    /// Wallet address / credential scan in filesystem.
    WalletAddressScan,
    /// Image file count scan.
    ImageCountScan,
    /// Markdown file count scan.
    MarkdownCountScan,
    /// Obsidian vault insights.
    ObsidianInsights,
    /// Email triage / inbox check.
    EmailTriage,
    /// Literary quote contextualisation.
    LiteraryQuoteContext,
    /// Short contradiction follow-up ("that's not true", "incorrect").
    Contradiction,
    /// Short follow-up referencing a previous reply ("what's that from?").
    ShortFollowup,
    /// Short reactive sarcasm ("wow", "great", "sure").
    ReactiveSarcasm,
}

// ── IntentMatcher ────────────────────────────────────────────────────────

/// Matching strategy for a single intent descriptor.
pub(super) enum IntentMatcher {
    /// Matches if the lowered prompt contains ANY of the given substrings.
    AnyKeyword(&'static [&'static str]),
    /// Matches if the prompt contains at least one keyword from EACH group.
    /// All groups must have ≥1 match (logical AND of groups, OR within each).
    AllGroups(&'static [&'static [&'static str]]),
    /// Arbitrary matching logic. Receives the already-lowercased prompt.
    Custom(fn(&str) -> bool),
}

impl IntentMatcher {
    fn matches(&self, lower: &str) -> bool {
        match self {
            Self::AnyKeyword(keywords) => keywords.iter().any(|k| lower.contains(k)),
            Self::AllGroups(groups) => groups
                .iter()
                .all(|group| group.iter().any(|k| lower.contains(k))),
            Self::Custom(f) => f(lower),
        }
    }
}

// ── IntentDescriptor ─────────────────────────────────────────────────────

/// Descriptor binding an intent to its classification metadata.
pub(super) struct IntentDescriptor {
    pub intent: Intent,
    /// Shortcut dispatch priority (higher = checked first, wins conflicts).
    pub priority: u8,
    /// If true, cached responses are skipped when this intent is detected.
    pub bypasses_cache: bool,
    /// The matching strategy for this intent.
    pub matcher: IntentMatcher,
}

// ── IntentRegistry ───────────────────────────────────────────────────────

/// Central registry for all intent classification.
///
/// Call [`classify()`](IntentRegistry::classify) once with the raw user
/// prompt to get all matching intents, sorted by priority (highest first).
pub(super) struct IntentRegistry {
    descriptors: Vec<IntentDescriptor>,
}

impl IntentRegistry {
    /// Build the default registry with all known intents.
    pub fn default_registry() -> Self {
        Self {
            descriptors: vec![
                // ── AnyKeyword intents ────────────────────────────
                IntentDescriptor {
                    intent: Intent::Execution,
                    priority: 10,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(EXECUTION_KEYWORDS),
                },
                IntentDescriptor {
                    intent: Intent::FileDistribution,
                    priority: 37,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(&["file distribution"]),
                },
                IntentDescriptor {
                    intent: Intent::ModelIdentity,
                    priority: 80,
                    bypasses_cache: false,
                    matcher: IntentMatcher::AnyKeyword(MODEL_IDENTITY_KEYWORDS),
                },
                IntentDescriptor {
                    intent: Intent::CurrentEvents,
                    priority: 65,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(CURRENT_EVENTS_KEYWORDS),
                },
                IntentDescriptor {
                    intent: Intent::Introspection,
                    priority: 60,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(INTROSPECTION_KEYWORDS),
                },
                IntentDescriptor {
                    intent: Intent::ProviderInventory,
                    priority: 75,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(PROVIDER_INVENTORY_KEYWORDS),
                },
                IntentDescriptor {
                    intent: Intent::CapabilitySummary,
                    priority: 71,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AnyKeyword(CAPABILITY_SUMMARY_KEYWORDS),
                },
                // ── AllGroups intents ─────────────────────────────
                IntentDescriptor {
                    intent: Intent::FolderScan,
                    priority: 39,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(FOLDER_SCAN_GROUPS),
                },
                IntentDescriptor {
                    intent: Intent::Acknowledgement,
                    priority: 85,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(ACKNOWLEDGEMENT_GROUPS),
                },
                IntentDescriptor {
                    intent: Intent::WalletAddressScan,
                    priority: 45,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(WALLET_SCAN_GROUPS),
                },
                IntentDescriptor {
                    intent: Intent::ImageCountScan,
                    priority: 43,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(IMAGE_COUNT_GROUPS),
                },
                IntentDescriptor {
                    intent: Intent::MarkdownCountScan,
                    priority: 41,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(MARKDOWN_COUNT_GROUPS),
                },
                IntentDescriptor {
                    intent: Intent::ObsidianInsights,
                    priority: 35,
                    bypasses_cache: true,
                    matcher: IntentMatcher::AllGroups(OBSIDIAN_INSIGHTS_GROUPS),
                },
                // ── Custom intents ────────────────────────────────
                IntentDescriptor {
                    intent: Intent::Delegation,
                    priority: 55,
                    bypasses_cache: false,
                    matcher: IntentMatcher::Custom(match_delegation),
                },
                IntentDescriptor {
                    intent: Intent::Cron,
                    priority: 53,
                    bypasses_cache: false,
                    matcher: IntentMatcher::Custom(match_cron),
                },
                IntentDescriptor {
                    intent: Intent::RandomToolUse,
                    priority: 50,
                    bypasses_cache: true,
                    matcher: IntentMatcher::Custom(match_random_tool_use),
                },
                IntentDescriptor {
                    intent: Intent::PersonalityProfile,
                    priority: 73,
                    bypasses_cache: true,
                    matcher: IntentMatcher::Custom(match_personality_profile),
                },
                IntentDescriptor {
                    intent: Intent::EmailTriage,
                    priority: 63,
                    bypasses_cache: true,
                    matcher: IntentMatcher::Custom(match_email_triage),
                },
                IntentDescriptor {
                    intent: Intent::LiteraryQuoteContext,
                    priority: 30,
                    bypasses_cache: true,
                    matcher: IntentMatcher::Custom(match_literary_quote_context),
                },
                // ── Channel intents (from channel_message.rs) ────
                IntentDescriptor {
                    intent: Intent::Contradiction,
                    priority: 95,
                    bypasses_cache: false,
                    matcher: IntentMatcher::Custom(match_contradiction),
                },
                IntentDescriptor {
                    intent: Intent::ShortFollowup,
                    priority: 93,
                    bypasses_cache: false,
                    matcher: IntentMatcher::Custom(match_short_followup),
                },
                IntentDescriptor {
                    intent: Intent::ReactiveSarcasm,
                    priority: 91,
                    bypasses_cache: false,
                    matcher: IntentMatcher::Custom(match_reactive_sarcasm),
                },
            ],
        }
    }

    /// Classify a user prompt into matching intents, sorted by priority
    /// (highest first). Lowercases exactly once.
    pub fn classify(&self, prompt: &str) -> Vec<Intent> {
        let lower = prompt.to_ascii_lowercase();
        let mut matches: Vec<(Intent, u8)> = self
            .descriptors
            .iter()
            .filter(|d| d.matcher.matches(&lower))
            .map(|d| (d.intent, d.priority))
            .collect();
        matches.sort_by(|a, b| b.1.cmp(&a.1));
        matches.into_iter().map(|(intent, _)| intent).collect()
    }

    /// Returns true if any of the matched intents require cache bypass.
    pub fn should_bypass_cache(&self, intents: &[Intent]) -> bool {
        let set: HashSet<Intent> = intents.iter().copied().collect();
        self.descriptors
            .iter()
            .any(|d| d.bypasses_cache && set.contains(&d.intent))
    }
}

// ── Keyword constants ────────────────────────────────────────────────────

const EXECUTION_KEYWORDS: &[&str] = &[
    " run ",
    " execute ",
    " use a tool",
    "use the tool",
    "tools you can use",
    "pick one at random",
    "introspection tool",
    "introspection skill",
    "introspect",
    "list entries",
    "list files",
    "file distribution",
    "schedule a cron",
    "schedule cron",
    "create cron",
    "order a subagent",
    "delegate",
    "orchestrate",
    "ls ",
    "/status",
];

const MODEL_IDENTITY_KEYWORDS: &[&str] = &[
    "current model",
    "what model",
    "which model",
    "still on",
    "still using",
    "using moonshot",
    "confirm for me",
    "/status",
];

const CURRENT_EVENTS_KEYWORDS: &[&str] = &[
    "geopolitical situation",
    "geopolitical",
    "geo political",
    "geopolitical sitrep",
    "sitrep",
    "current events",
    "latest news",
    "what's happening",
    "what is happening",
    "goings on",
    "going on in the",
    "what does the",
    "today's",
    "as of today",
];

const INTROSPECTION_KEYWORDS: &[&str] = &[
    "introspection tool",
    "introspection skill",
    "introspect",
    "what tools can you use",
    "what tools do you have",
    "available tools",
    "subagent functionality",
    "current subagent functionality",
    "summarize the results",
    "summarize introspection",
];

const PROVIDER_INVENTORY_KEYWORDS: &[&str] = &[
    "which llm providers",
    "what llm providers",
    "which providers",
    "what providers",
];

const CAPABILITY_SUMMARY_KEYWORDS: &[&str] = &[
    "what are you able to do",
    "what can you do",
    "what are you able",
    "what can you help",
];

const FOLDER_SCAN_GROUPS: &[&[&str]] = &[
    &["look in", "check my", "search", "scan", "inspect"],
    &[
        "folder",
        "directory",
        "~/downloads",
        "~/documents",
        "~/pictures",
        "~/photos",
        "~/desktop",
        "~/",
        "/users/",
    ],
];

const ACKNOWLEDGEMENT_GROUPS: &[&[&str]] = &[
    &["acknowledge", "acknowledg"],
    &["one sentence", "then wait"],
];

const WALLET_SCAN_GROUPS: &[&[&str]] = &[
    &[
        "wallet address",
        "wallet addresses",
        "wallet credential",
        "wallet credentials",
        "private key",
        "seed phrase",
        "mnemonic",
        "keystore",
        "xprv",
        "xpub",
    ],
    &[
        "search",
        "find",
        "scan",
        "recursively",
        "look in",
        "check",
        "see if there are",
    ],
];

const IMAGE_COUNT_GROUPS: &[&[&str]] = &[
    &["how many", "count", "number of", "total"],
    &["image files", "images", "photos", "pictures"],
];

const MARKDOWN_COUNT_GROUPS: &[&[&str]] = &[
    &["how many", "count", "number of", "total"],
    &[
        "markdown file",
        "markdown files",
        ".md files",
        "md files",
        "files ending in .md",
    ],
];

const OBSIDIAN_INSIGHTS_GROUPS: &[&[&str]] = &[
    &["obsidian", "vault"],
    &[
        "insight",
        "summary",
        "summarize",
        "what",
        "say about",
        "status",
    ],
];

// ── Custom matcher functions ─────────────────────────────────────────────

/// Delegation: compound OR + AND logic from `requests_delegation()`.
fn match_delegation(lower: &str) -> bool {
    if lower.contains("delegate") || lower.contains("orchestrate") {
        return true;
    }
    if lower.contains("assign") && (lower.contains("subagent") || lower.contains("sub agent")) {
        return true;
    }
    (lower.contains("subagent") || lower.contains("sub agent"))
        && (lower.contains("order")
            || lower.contains("ask")
            || lower.contains("task")
            || lower.contains("run ")
            || lower.contains("to a subagent")
            || lower.contains("to a sub agent")
            || lower.contains("to the subagent")
            || lower.contains("to the sub agent"))
}

/// Cron: simple OR + compound AND from `requests_cron()`.
fn match_cron(lower: &str) -> bool {
    lower.contains("cron") || (lower.contains("schedule") && lower.contains("minute"))
}

/// Random tool use: OR + compound from `requests_random_tool_use()`.
fn match_random_tool_use(lower: &str) -> bool {
    lower.contains("tools you can use")
        || (lower.contains("pick one at random") && lower.contains("tool"))
}

/// Personality profile: compound from `requests_personality_profile()`.
fn match_personality_profile(lower: &str) -> bool {
    (lower.contains("personality") && (lower.contains("your") || lower.contains("you")))
        || lower.contains("who are you")
}

/// Email triage: AnyKeyword OR compound from `requests_email_triage()`.
fn match_email_triage(lower: &str) -> bool {
    const EMAIL_MARKERS: &[&str] = &[
        "check my email",
        "check email",
        "inbox",
        "mailbox",
        "important email",
        "important emails",
        "scan my email",
        "email triage",
        "email digest",
    ];
    const BRIDGE_MARKERS: &[&str] = &["proton bridge", "protonbridge", "himalaya"];
    EMAIL_MARKERS.iter().any(|m| lower.contains(m))
        || (lower.contains("email") && BRIDGE_MARKERS.iter().any(|m| lower.contains(m)))
}

/// Literary quote context: complex compound from `requests_literary_quote_context()`.
fn match_literary_quote_context(lower: &str) -> bool {
    if lower.contains("what's that from") || lower.contains("what is that from") {
        return true;
    }
    let asks_for_quote = lower.contains("quote")
        || lower.contains("line from")
        || lower.contains("appropriate line")
        || lower.contains("what quote");
    let literary_source = lower.contains("dune")
        || lower.contains("frank herbert")
        || lower.contains("litany against fear");
    let contextual_target = lower.contains("conflict")
        || lower.contains("iran")
        || lower.contains("geopolitical")
        || lower.contains("situation");
    asks_for_quote && (literary_source || contextual_target)
}

/// Short contradiction follow-up from `is_short_contradiction_followup()`.
/// Length-gated to ≤48 chars (trimmed).
fn match_contradiction(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.len() > 48 {
        return false;
    }
    const MARKERS: &[&str] = &[
        "that's not true",
        "that is not true",
        "not true",
        "that's wrong",
        "that is wrong",
        "incorrect",
    ];
    MARKERS.iter().any(|m| trimmed.contains(m))
}

/// Short follow-up referencing a previous reply from
/// `is_short_followup_for_previous_reply()`.
/// Length-gated to ≤80 chars (trimmed).
fn match_short_followup(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.len() > 80 {
        return false;
    }
    const MARKERS: &[&str] = &[
        "what's that from",
        "what is that from",
        "where is that from",
        "no, your quote",
        "your quote",
        "what quote",
        "source?",
    ];
    MARKERS.iter().any(|m| trimmed.contains(m))
}

/// Short reactive sarcasm from `is_short_reactive_sarcasm()`.
/// Length-gated to ≤32 chars (trimmed). Uses exact/suffix match
/// (marker, marker., marker...) — not substring.
fn match_reactive_sarcasm(lower: &str) -> bool {
    let trimmed = lower.trim();
    if trimmed.len() > 32 {
        return false;
    }
    const MARKERS: &[&str] = &[
        "wow",
        "great",
        "fantastic",
        "amazing",
        "incredible",
        "brilliant",
        "sure",
        "right",
    ];
    MARKERS.iter().any(|m| {
        trimmed == *m || {
            // Allocation-free suffix strip: check for "marker." or "marker..."
            let stripped = trimmed
                .strip_suffix("...")
                .or_else(|| trimmed.strip_suffix('.'));
            stripped.is_some_and(|s| s == *m)
        }
    })
}

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn classify(prompt: &str) -> Vec<Intent> {
        IntentRegistry::default_registry().classify(prompt)
    }

    fn has(prompt: &str, intent: Intent) -> bool {
        classify(prompt).contains(&intent)
    }

    fn bypass(prompt: &str) -> bool {
        let reg = IntentRegistry::default_registry();
        let intents = reg.classify(prompt);
        reg.should_bypass_cache(&intents)
    }

    // ── Parity tests (ported from intents.rs) ────────────────────────

    #[test]
    fn execution_markers_cover_shortcut_and_guard_triggers() {
        assert!(has(
            "tell me about the tools you can use, pick one at random, and use it",
            Intent::Execution,
        ));
        assert!(has("/status", Intent::Execution));
        assert!(has("please execute ls /tmp", Intent::Execution));
    }

    #[test]
    fn delegation_and_cron_markers_match_expected_prompts() {
        assert!(has("order a subagent to do this", Intent::Delegation));
        assert!(has("ask the sub agent to do this", Intent::Delegation));
        assert!(!has(
            "use your introspection tool to discover current subagent functionality",
            Intent::Delegation,
        ));
        assert!(has("schedule a cron job every 5 minute", Intent::Cron,));
    }

    #[test]
    fn model_identity_markers_match_expected_prompts() {
        assert!(has(
            "can you confirm for me that you are still using moonshot?",
            Intent::ModelIdentity,
        ));
        assert!(has("/status", Intent::ModelIdentity));
    }

    #[test]
    fn current_events_markers_match_expected_prompts() {
        assert!(has(
            "What's the geopolitical situation?",
            Intent::CurrentEvents,
        ));
        assert!(has("Give me a geopolitical sitrep", Intent::CurrentEvents,));
        assert!(has(
            "What does the geo political sub agent say about goings on in the US?",
            Intent::CurrentEvents,
        ));
        assert!(has(
            "What are today's current events?",
            Intent::CurrentEvents,
        ));
    }

    #[test]
    fn introspection_markers_match_expected_prompts() {
        assert!(has(
            "I want you to use your introspection skill",
            Intent::Introspection,
        ));
        assert!(has(
            "use your introspection tool to discover current subagent functionality",
            Intent::Introspection,
        ));
        assert!(has(
            "what tools do you have available?",
            Intent::Introspection,
        ));
    }

    #[test]
    fn email_triage_markers_match_expected_prompts() {
        assert!(has(
            "Can you have a subagent check my email for anything important?",
            Intent::EmailTriage,
        ));
        assert!(has(
            "Use proton bridge and triage inbox for urgent items",
            Intent::EmailTriage,
        ));
        assert!(!has("Check my calendar for tomorrow", Intent::EmailTriage,));
    }

    #[test]
    fn literary_quote_markers_match_expected_prompts() {
        assert!(has(
            "Give me an appropriate dune quote for the conflict in Iran",
            Intent::LiteraryQuoteContext,
        ));
        assert!(has("What's that from?", Intent::LiteraryQuoteContext,));
        assert!(!has(
            "Give me a geopolitical situation update",
            Intent::LiteraryQuoteContext,
        ));
    }

    #[test]
    fn acknowledgement_markers_match_expected_prompts() {
        assert!(has(
            "Good evening Duncan. Acknowledge this request in one sentence, then wait.",
            Intent::Acknowledgement,
        ));
        assert!(has(
            "acknowledge this in one sentence and then wait for my next command",
            Intent::Acknowledgement,
        ));
        assert!(!has("please acknowledge receipt", Intent::Acknowledgement,));
    }

    #[test]
    fn provider_inventory_markers_match_expected_prompts() {
        assert!(has("which llm providers?", Intent::ProviderInventory));
        assert!(has(
            "what providers are configured",
            Intent::ProviderInventory,
        ));
        assert!(!has("what model are you using", Intent::ProviderInventory));
    }

    #[test]
    fn personality_and_capability_markers_match_expected_prompts() {
        assert!(has(
            "Tell me about your personality",
            Intent::PersonalityProfile,
        ));
        assert!(has("who are you", Intent::PersonalityProfile));
        assert!(has(
            "Duncan, what are you able to do for me right now?",
            Intent::CapabilitySummary,
        ));
    }

    #[test]
    fn wallet_scan_markers_match_expected_prompts() {
        assert!(has(
            "search the ~/code folder recursively and tell me files containing wallet address",
            Intent::WalletAddressScan,
        ));
        assert!(has(
            "find wallet addresses in /tmp recursively",
            Intent::WalletAddressScan,
        ));
        assert!(has(
            "I want you to check my ~/Downloads folder to see if there are any wallet credentials there",
            Intent::WalletAddressScan,
        ));
        assert!(has(
            "Now look in my Downloads folder for wallet credentials",
            Intent::WalletAddressScan,
        ));
        assert!(has(
            "Please check my Desktop folder for files containing private key and list full paths.",
            Intent::WalletAddressScan,
        ));
        assert!(!has(
            "show me your wallet balance",
            Intent::WalletAddressScan,
        ));
    }

    #[test]
    fn image_count_markers_match_expected_prompts() {
        assert!(has(
            "How many image files are in my photos?",
            Intent::ImageCountScan,
        ));
        assert!(has(
            "count images in ~/Downloads recursively",
            Intent::ImageCountScan,
        ));
        assert!(!has(
            "show me photos from yesterday",
            Intent::ImageCountScan,
        ));
    }

    #[test]
    fn markdown_count_markers_match_expected_prompts() {
        assert!(has(
            "Count markdown files recursively in /Users/jmachen/code and return only the number.",
            Intent::MarkdownCountScan,
        ));
        assert!(has(
            "how many .md files are in ~/code?",
            Intent::MarkdownCountScan,
        ));
        assert!(!has(
            "count image files in ~/Pictures",
            Intent::MarkdownCountScan,
        ));
    }

    #[test]
    fn folder_scan_markers_match_expected_prompts() {
        assert!(has(
            "Now look in my Downloads folder and summarize what is there",
            Intent::FolderScan,
        ));
        assert!(has(
            "please check my ~/Documents folder for wallet credentials",
            Intent::FolderScan,
        ));
        assert!(!has("what's your personality?", Intent::FolderScan));
    }

    #[test]
    fn obsidian_insight_markers_match_expected_prompts() {
        assert!(has(
            "Any insights you care to draw from the obsidian vault?",
            Intent::ObsidianInsights,
        ));
        assert!(has("summarize my vault", Intent::ObsidianInsights));
        assert!(!has("vault token price", Intent::ObsidianInsights));
    }

    // ── Cache bypass parity ──────────────────────────────────────────

    #[test]
    fn cache_bypass_markers_cover_shortcut_handled_prompts() {
        assert!(bypass(
            "tell me about the tools you can use, pick one at random, and use it",
        ));
        assert!(bypass(
            "Good evening Duncan. Acknowledge this request in one sentence, then wait.",
        ));
        assert!(bypass(
            "What does the geopolitical monitor have to say about today's news?",
        ));
        assert!(!bypass("Summarize this paragraph in one sentence."));
    }

    // ── Priority ordering ────────────────────────────────────────────

    #[test]
    fn model_identity_wins_over_execution_for_status() {
        let intents = classify("/status");
        assert!(intents.contains(&Intent::ModelIdentity));
        assert!(intents.contains(&Intent::Execution));
        let mi_pos = intents
            .iter()
            .position(|i| *i == Intent::ModelIdentity)
            .unwrap();
        let ex_pos = intents
            .iter()
            .position(|i| *i == Intent::Execution)
            .unwrap();
        assert!(
            mi_pos < ex_pos,
            "ModelIdentity (priority 80) should precede Execution (priority 10)"
        );
    }

    #[test]
    fn acknowledgement_has_highest_shortcut_priority_among_standard_intents() {
        // Acknowledgement (85) should be higher than any other standard intent
        let intents = classify(
            "acknowledge this in one sentence, then wait. Also tell me the current events.",
        );
        assert!(intents.contains(&Intent::Acknowledgement));
        assert!(intents.contains(&Intent::CurrentEvents));
        let ack_pos = intents
            .iter()
            .position(|i| *i == Intent::Acknowledgement)
            .unwrap();
        let ce_pos = intents
            .iter()
            .position(|i| *i == Intent::CurrentEvents)
            .unwrap();
        assert!(ack_pos < ce_pos);
    }

    // ── Channel intent parity ────────────────────────────────────────

    #[test]
    fn contradiction_matches_short_prompts() {
        assert!(has("that's not true", Intent::Contradiction));
        assert!(has("That is wrong.", Intent::Contradiction));
        assert!(has("incorrect", Intent::Contradiction));
        // Over 48 chars → rejected
        assert!(!has(
            "I think that is not true based on extensive research and evidence",
            Intent::Contradiction,
        ));
    }

    #[test]
    fn short_followup_matches_quote_references() {
        assert!(has("what's that from?", Intent::ShortFollowup));
        assert!(has("Where is that from?", Intent::ShortFollowup));
        assert!(has("source?", Intent::ShortFollowup));
        // Over 80 chars → rejected
        let long = format!(
            "What's that from? I need to know because {}",
            "a".repeat(80)
        );
        assert!(!has(&long, Intent::ShortFollowup));
    }

    #[test]
    fn reactive_sarcasm_matches_exact_and_suffixed() {
        assert!(has("wow", Intent::ReactiveSarcasm));
        assert!(has("Great.", Intent::ReactiveSarcasm));
        assert!(has("fantastic...", Intent::ReactiveSarcasm));
        assert!(has("  sure  ", Intent::ReactiveSarcasm));
        // Not a substring match — "wow that was great" should NOT match
        assert!(!has("wow that was great", Intent::ReactiveSarcasm));
        // Over 32 chars → rejected
        assert!(!has(
            "wow this is incredibly amazing work",
            Intent::ReactiveSarcasm,
        ));
    }

    #[test]
    fn channel_intents_have_highest_priorities() {
        // Contradiction (95) > ShortFollowup (93) > ReactiveSarcasm (91) > all others
        let reg = IntentRegistry::default_registry();
        let channel_priorities: Vec<u8> = reg
            .descriptors
            .iter()
            .filter(|d| {
                matches!(
                    d.intent,
                    Intent::Contradiction | Intent::ShortFollowup | Intent::ReactiveSarcasm
                )
            })
            .map(|d| d.priority)
            .collect();
        let max_standard: u8 = reg
            .descriptors
            .iter()
            .filter(|d| {
                !matches!(
                    d.intent,
                    Intent::Contradiction | Intent::ShortFollowup | Intent::ReactiveSarcasm
                )
            })
            .map(|d| d.priority)
            .max()
            .unwrap_or(0);
        assert!(
            channel_priorities.iter().all(|&p| p > max_standard),
            "Channel intents must have higher priority than all standard intents"
        );
    }

    // ── Negative tests ───────────────────────────────────────────────

    #[test]
    fn ordinary_prompt_matches_no_intents() {
        let intents = classify("Tell me a joke about Rust programming.");
        assert!(intents.is_empty());
    }

    #[test]
    fn classify_lowercases_once_and_matches_case_insensitively() {
        assert!(has("WHICH LLM PROVIDERS?", Intent::ProviderInventory));
        assert!(has("What Model Are You Using?", Intent::ModelIdentity));
        assert!(has("/STATUS", Intent::Execution));
    }
}
