use super::*;

// ── estimate_cost_from_provider tests ────────────────────────

#[test]
fn estimate_cost_zero_tokens() {
    assert_eq!(estimate_cost_from_provider(0.001, 0.002, 0, 0), 0.0);
}

#[test]
fn estimate_cost_input_only() {
    let cost = estimate_cost_from_provider(0.001, 0.002, 100, 0);
    assert!((cost - 0.1).abs() < f64::EPSILON);
}

#[test]
fn estimate_cost_output_only() {
    let cost = estimate_cost_from_provider(0.001, 0.002, 0, 100);
    assert!((cost - 0.2).abs() < f64::EPSILON);
}

#[test]
fn estimate_cost_both_directions() {
    let cost = estimate_cost_from_provider(0.001, 0.002, 500, 200);
    let expected = 500.0 * 0.001 + 200.0 * 0.002;
    assert!((cost - expected).abs() < f64::EPSILON);
}

#[test]
fn estimate_cost_negative_tokens_handled() {
    let cost = estimate_cost_from_provider(0.001, 0.002, -100, -50);
    assert!(cost < 0.0);
}

#[test]
fn estimate_cost_large_values() {
    let cost = estimate_cost_from_provider(0.00001, 0.00003, 1_000_000, 500_000);
    let expected = 1_000_000.0 * 0.00001 + 500_000.0 * 0.00003;
    assert!((cost - expected).abs() < 1e-6);
}

#[test]
fn estimate_cost_zero_rates() {
    let cost = estimate_cost_from_provider(0.0, 0.0, 1000, 2000);
    assert_eq!(cost, 0.0);
}

// ── summarize_user_excerpt tests ─────────────────────────────

#[test]
fn summarize_user_excerpt_limits_token_count_and_length() {
    let long = (0..100)
        .map(|i| format!("tok{i}"))
        .collect::<Vec<_>>()
        .join(" ");
    let summary = summarize_user_excerpt(&long);
    assert!(summary.split_whitespace().count() <= 20);
    assert!(summary.len() <= 240);
}

// ── fallback_candidates tests ────────────────────────────────

#[test]
fn fallback_candidates_preserve_primary_and_dedup_primary_from_fallbacks() {
    let cfg = ironclad_core::IroncladConfig::from_str(
        r#"
[agent]
name = "TestBot"
id = "test-agent"

[server]
port = 0

[database]
path = ":memory:"

[models]
primary = "openai/gpt-4o"
fallbacks = ["openai/gpt-4o", "anthropic/claude-sonnet-4-20250514", "google/gemini-3.1-pro-preview"]
"#,
    )
    .unwrap();
    let cands = fallback_candidates(&cfg, "openai/gpt-4o");
    assert_eq!(cands[0], "openai/gpt-4o");
    assert_eq!(cands.len(), 3);
    assert!(cands.contains(&"anthropic/claude-sonnet-4-20250514".to_string()));
}
