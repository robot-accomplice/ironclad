use super::*;

// ── split_subtasks tests ─────────────────────────────────────

#[test]
fn split_subtasks_detects_multi_step_inputs() {
    let parts = split_subtasks("research impact and draft summary then propose next steps");
    assert!(parts.len() >= 3);
}

#[test]
fn split_subtasks_returns_single_item_for_simple_prompt() {
    let parts = split_subtasks("summarize this report");
    assert_eq!(parts.len(), 1);
    assert_eq!(parts[0], "summarize this report");
}

#[test]
fn split_subtasks_semicolons() {
    let parts = split_subtasks("task A; task B; task C");
    assert_eq!(parts.len(), 3);
    assert_eq!(parts[0], "task A");
    assert_eq!(parts[1], "task B");
    assert_eq!(parts[2], "task C");
}

#[test]
fn split_subtasks_empty() {
    let parts = split_subtasks("");
    assert!(parts.is_empty());
}

#[test]
fn split_subtasks_newlines() {
    let parts = split_subtasks("line 1\nline 2\nline 3");
    assert_eq!(parts.len(), 3);
}

#[test]
fn split_subtasks_deduplicates_adjacent() {
    let parts = split_subtasks("task A\ntask A");
    assert_eq!(parts.len(), 1);
}

// ── utility_margin_for_delegation tests ──────────────────────

#[test]
fn utility_margin_penalizes_low_fit() {
    let low_fit = utility_margin_for_delegation(0.6, 3, 0.1);
    let high_fit = utility_margin_for_delegation(0.6, 3, 0.9);
    assert!(high_fit > low_fit);
}

#[test]
fn utility_margin_increases_with_complexity_and_falls_with_low_fit() {
    let base = utility_margin_for_delegation(0.3, 2, 0.9);
    let more_complex = utility_margin_for_delegation(0.3, 4, 0.9);
    let lower_fit = utility_margin_for_delegation(0.3, 4, 0.2);
    assert!(more_complex > base);
    assert!(lower_fit < more_complex);
}

#[test]
fn utility_margin_negative_for_single_task_low_complexity() {
    let margin = utility_margin_for_delegation(0.1, 1, 0.1);
    assert!(
        margin < 0.0,
        "single trivial task should not justify delegation"
    );
}

#[test]
fn utility_margin_high_for_complex_multi_task() {
    let margin = utility_margin_for_delegation(1.0, 5, 1.0);
    assert!(
        margin > 0.0,
        "complex multi-task with perfect fit should justify delegation"
    );
}

// ── proposal_to_json tests ───────────────────────────────────

#[test]
fn proposal_json_contains_reviewable_config() {
    let proposal = SpecialistProposal {
        name: "geo-specialist".into(),
        display_name: "Geo Specialist".into(),
        description: "Monitors geopolitical risk".into(),
        skills: vec!["geopolitical".into(), "risk-analysis".into()],
        model: "auto".into(),
    };
    let payload = proposal_to_json(&proposal, "coverage gap");
    assert_eq!(payload["name"], "geo-specialist");
    assert!(payload["skills"].is_array());
    assert_eq!(payload["model"], "auto");
}

// ── capability_tokens tests ──────────────────────────────────

#[test]
fn capability_tokens_extracts_lowercase_wordlike_tokens() {
    let tokens = capability_tokens("Geo-Political analysis, RISK_123 and alerts!");
    assert!(tokens.contains(&"political".to_string()));
    assert!(tokens.contains(&"analysis".to_string()));
    assert!(tokens.contains(&"risk".to_string()));
    assert!(tokens.contains(&"alerts".to_string()));
}

#[test]
fn capability_tokens_filters_short_tokens() {
    let tokens = capability_tokens("a bb ccc dddd");
    assert!(!tokens.contains(&"a".to_string()));
    assert!(!tokens.contains(&"bb".to_string()));
    assert!(!tokens.contains(&"ccc".to_string()));
    assert!(tokens.contains(&"dddd".to_string()));
}

#[test]
fn capability_tokens_empty() {
    assert!(capability_tokens("").is_empty());
}
