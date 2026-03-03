use super::*;

// ── sanitize_diag_token tests ────────────────────────────────

#[test]
fn sanitize_diag_token_removes_unsafe_chars_and_limits_len() {
    let token = sanitize_diag_token("  !!openai/gpt-4o:mini??\n\t", 10);
    assert_eq!(token, "openai/gpt");
}

#[test]
fn sanitize_diag_token_empty_input() {
    assert_eq!(sanitize_diag_token("", 50), "");
}

#[test]
fn sanitize_diag_token_all_special_chars() {
    assert_eq!(sanitize_diag_token("!!!@@@###$$$", 50), "");
}

#[test]
fn sanitize_diag_token_preserves_allowed_chars() {
    assert_eq!(
        sanitize_diag_token("openai/gpt-4o:mini_v2", 50),
        "openai/gpt-4o:mini_v2"
    );
}

#[test]
fn sanitize_diag_token_strips_leading_trailing_separators() {
    assert_eq!(sanitize_diag_token("---model---", 50), "model");
    assert_eq!(sanitize_diag_token("___test___", 50), "test");
    assert_eq!(sanitize_diag_token("///path///", 50), "path");
}

// ── is_model_proxy_role tests ────────────────────────────────

#[test]
fn model_proxy_roles_are_detected_case_insensitively() {
    assert!(is_model_proxy_role("model-proxy"));
    assert!(is_model_proxy_role("Model-Proxy"));
    assert!(!is_model_proxy_role("subagent"));
}

// ── diagnostics_system_note tests ────────────────────────────

fn sample_diagnostics() -> RuntimeDiagnostics {
    RuntimeDiagnostics {
        uptime_seconds: 42,
        primary_model: "ollama/qwen3:8b".into(),
        active_model: "ollama/qwen3:8b".into(),
        primary_provider: "ollama".into(),
        primary_provider_state: "closed".into(),
        breaker_open_count: 0,
        breaker_half_open_count: 0,
        cache_entries: 3,
        cache_hit_rate_pct: 50.0,
        pending_approvals: 1,
        taskable_subagents_total: 2,
        taskable_subagents_enabled: 1,
        taskable_subagents_booting: 0,
        taskable_subagents_running: 1,
        taskable_subagents_error: 0,
        delegation_tools_available: true,
        channels_total: 2,
        channels_with_errors: 0,
    }
}

#[test]
fn diagnostics_system_note_contains_expected_sections() {
    let note = diagnostics_system_note(&sample_diagnostics());
    assert!(note.contains("Runtime diagnostics"));
    assert!(note.contains("models:"));
    assert!(note.contains("provider:"));
    assert!(note.contains("cache:"));
    assert!(note.contains("approvals_pending"));
    assert!(note.contains("delegation_tools_available"));
}

#[test]
fn diagnostics_system_note_warns_when_delegation_tools_unavailable() {
    let mut diag = sample_diagnostics();
    diag.taskable_subagents_total = 10;
    diag.taskable_subagents_enabled = 10;
    diag.taskable_subagents_running = 10;
    diag.delegation_tools_available = false;
    let note = diagnostics_system_note(&diag);
    assert!(note.contains("delegated subagent tools are unavailable"));
}

#[test]
fn diagnostics_system_note_reports_booting_not_taskable() {
    let mut diag = sample_diagnostics();
    diag.taskable_subagents_total = 4;
    diag.taskable_subagents_enabled = 4;
    diag.taskable_subagents_booting = 4;
    diag.taskable_subagents_running = 0;
    let note = diagnostics_system_note(&diag);
    assert!(note.contains("subagents are booting and are not taskable yet"));
    assert!(note.contains("booting=4"));
}
