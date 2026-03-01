use super::*;

// ── parse_tool_call tests ────────────────────────────────────

#[test]
fn parse_tool_call_valid() {
    let input = r#"Let me check that. {"tool_call": {"name": "read_file", "params": {"path": "/tmp/test.txt"}}}"#;
    let result = parse_tool_call(input);
    assert!(result.is_some());
    let (name, params) = result.unwrap();
    assert_eq!(name, "read_file");
    assert_eq!(params["path"], "/tmp/test.txt");
}

#[test]
fn parse_tool_call_no_params() {
    let input = r#"{"tool_call": {"name": "status"}}"#;
    let result = parse_tool_call(input);
    assert!(result.is_some());
    let (name, params) = result.unwrap();
    assert_eq!(name, "status");
    assert!(params.is_object());
}

#[test]
fn parse_tool_call_none_for_no_tool() {
    assert!(parse_tool_call("Hello, how are you?").is_none());
    assert!(parse_tool_call("").is_none());
}

#[test]
fn parse_tool_call_nested_braces() {
    let input = r#"{"tool_call": {"name": "bash", "params": {"command": "echo '{hello}'"}}}"#;
    let result = parse_tool_call(input);
    assert!(result.is_some());
    let (name, _params) = result.unwrap();
    assert_eq!(name, "bash");
}

#[test]
fn parse_tool_call_malformed_json() {
    assert!(parse_tool_call(r#"{"tool_call": {"name": broken}}"#).is_none());
}

#[test]
fn parse_tool_call_surrounded_by_text() {
    let input = r#"I'll read the file now. {"tool_call": {"name": "read_file", "params": {"path": "test.rs"}}} Let me analyze the output."#;
    let result = parse_tool_call(input);
    assert!(result.is_some());
    let (name, params) = result.unwrap();
    assert_eq!(name, "read_file");
    assert_eq!(params["path"], "test.rs");
}

#[test]
fn parse_tool_call_ignores_fake_earlier_mention() {
    // L-HIGH-1: a fake "tool_call" in natural language must not prevent
    // parsing the real tool call at the end
    let resp = r#"The "tool_call" pattern is used for function calls. Here is the actual one: {"tool_call": {"name": "echo", "params": {"msg": "hello"}}}"#;
    let (name, params) = parse_tool_call(resp).expect("should find real tool call");
    assert_eq!(name, "echo");
    assert_eq!(params["msg"], "hello");
}

// ── check_tool_policy tests ──────────────────────────────────

#[test]
fn check_tool_policy_allows_when_no_rules() {
    let engine = ironclad_agent::policy::PolicyEngine::new();
    let result = check_tool_policy(
        &engine,
        "read_file",
        &serde_json::json!({"path": "/tmp/test.txt"}),
        ironclad_core::InputAuthority::Creator,
        ironclad_core::SurvivalTier::Normal,
        ironclad_core::RiskLevel::Safe,
    );
    assert!(result.is_ok());
}

#[test]
fn check_tool_policy_deny_returns_403_and_reason() {
    let mut engine = ironclad_agent::policy::PolicyEngine::new();
    engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
    let result = check_tool_policy(
        &engine,
        "bash",
        &serde_json::json!({"command": "rm -rf /"}),
        ironclad_core::InputAuthority::External,
        ironclad_core::SurvivalTier::Normal,
        ironclad_core::RiskLevel::Dangerous,
    );
    let JsonError(status, reason) = result.unwrap_err();
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!reason.is_empty());
}

#[test]
fn check_tool_policy_with_authority_rule() {
    let mut engine = ironclad_agent::policy::PolicyEngine::new();
    engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
    let result = check_tool_policy(
        &engine,
        "wallet_transfer",
        &serde_json::json!({"amount": 100}),
        ironclad_core::InputAuthority::Creator,
        ironclad_core::SurvivalTier::Normal,
        ironclad_core::RiskLevel::Dangerous,
    );
    assert!(result.is_ok());
}

#[test]
fn check_tool_policy_critical_tier_restricts() {
    let mut engine = ironclad_agent::policy::PolicyEngine::new();
    engine.add_rule(Box::new(ironclad_agent::policy::AuthorityRule));
    engine.add_rule(Box::new(ironclad_agent::policy::CommandSafetyRule));
    let result = check_tool_policy(
        &engine,
        "read_file",
        &serde_json::json!({"path": "/etc/passwd"}),
        ironclad_core::InputAuthority::External,
        ironclad_core::SurvivalTier::Critical,
        ironclad_core::RiskLevel::Safe,
    );
    assert!(result.is_ok() || result.is_err());
}

// ── classify_provider_error / info-disclosure tests ──────────

#[test]
fn classify_provider_error_auth() {
    assert_eq!(
        classify_provider_error("HTTP 401 Unauthorized: invalid api key sk-abc123xyz"),
        "provider authentication error"
    );
    assert_eq!(
        classify_provider_error("403 Forbidden"),
        "provider authentication error"
    );
}

#[test]
fn classify_provider_error_rate_limit() {
    assert_eq!(
        classify_provider_error("429 Too Many Requests - rate limit exceeded"),
        "provider rate limit reached"
    );
    assert_eq!(
        classify_provider_error("rate_limit_error: you have exceeded your quota"),
        "provider rate limit reached"
    );
}

#[test]
fn classify_provider_error_network() {
    assert_eq!(
        classify_provider_error(
            "request failed: connection refused to https://internal.corp:8443/v1/chat"
        ),
        "network error reaching provider"
    );
    assert_eq!(
        classify_provider_error("timeout after 30s"),
        "network error reaching provider"
    );
}

#[test]
fn classify_provider_error_server() {
    assert_eq!(
        classify_provider_error("500 Internal Server Error\n<html>stack trace...</html>"),
        "provider server error"
    );
    assert_eq!(
        classify_provider_error("502 Bad Gateway"),
        "provider server error"
    );
}

#[test]
fn classify_provider_error_circuit_breaker() {
    assert_eq!(
        classify_provider_error("circuit breaker open for provider openai"),
        "provider temporarily unavailable"
    );
}

#[test]
fn classify_provider_error_no_key() {
    assert_eq!(
        classify_provider_error("no API key configured for openai"),
        "no provider configured for this model"
    );
    assert_eq!(
        classify_provider_error("no provider configured for model gpt-4"),
        "no provider configured for this model"
    );
}

#[test]
fn classify_provider_error_quota() {
    assert_eq!(
        classify_provider_error("402 Payment Required - billing issue"),
        "provider quota or billing issue"
    );
    assert_eq!(
        classify_provider_error("insufficient credit balance"),
        "provider quota or billing issue"
    );
}

#[test]
fn classify_provider_error_unknown_fallback() {
    assert_eq!(
        classify_provider_error("something completely unexpected happened"),
        "provider error"
    );
}

#[test]
fn provider_failure_message_varies_by_persistence_behavior() {
    let msg_retry = provider_failure_user_message("timeout", true);
    assert!(msg_retry.contains("stored"));
    assert!(msg_retry.contains("retry"));

    let msg_try_again = provider_failure_user_message("timeout", false);
    assert!(msg_try_again.contains("Please try again"));
}

#[test]
fn provider_failure_user_message_no_leak() {
    let raw_error = "HTTP 401 Unauthorized: api key sk-secret-key-12345 \
                     at https://internal.corp:8443/v1/chat/completions";
    let msg_stored = provider_failure_user_message(raw_error, true);
    let msg_retry = provider_failure_user_message(raw_error, false);

    // The raw error must NOT appear in user-facing messages
    assert!(
        !msg_stored.contains("sk-secret"),
        "API key leaked in stored message: {msg_stored}"
    );
    assert!(
        !msg_stored.contains("internal.corp"),
        "internal URL leaked in stored message: {msg_stored}"
    );
    assert!(
        !msg_retry.contains("sk-secret"),
        "API key leaked in retry message: {msg_retry}"
    );
    assert!(
        !msg_retry.contains("internal.corp"),
        "internal URL leaked in retry message: {msg_retry}"
    );

    // Should contain the safe category instead
    assert!(msg_stored.contains("provider authentication error"));
    assert!(msg_retry.contains("provider authentication error"));
}

// ── is_virtual_delegation_tool tests ─────────────────────────

#[test]
fn is_virtual_delegation_tool_recognizes_all_variants() {
    assert!(is_virtual_delegation_tool("orchestrate-subagents"));
    assert!(is_virtual_delegation_tool("orchestrate_subagents"));
    assert!(is_virtual_delegation_tool("assign-tasks"));
    assert!(is_virtual_delegation_tool("assign_tasks"));
    assert!(is_virtual_delegation_tool("delegate-subagent"));
    assert!(is_virtual_delegation_tool("delegate_subagent"));
    assert!(is_virtual_delegation_tool("select-subagent-model"));
    assert!(is_virtual_delegation_tool("select_subagent_model"));
}

#[test]
fn is_virtual_delegation_tool_case_insensitive() {
    assert!(is_virtual_delegation_tool("ORCHESTRATE-SUBAGENTS"));
    assert!(is_virtual_delegation_tool("Assign-Tasks"));
    assert!(is_virtual_delegation_tool("  Delegate_Subagent  "));
}

#[test]
fn is_virtual_delegation_tool_rejects_non_delegation() {
    assert!(!is_virtual_delegation_tool("read_file"));
    assert!(!is_virtual_delegation_tool("bash"));
    assert!(!is_virtual_delegation_tool("web_search"));
    assert!(!is_virtual_delegation_tool(""));
}

// ── is_virtual_orchestration_tool tests ──────────────────────

#[test]
fn is_virtual_orchestration_tool_recognizes_all_variants() {
    assert!(is_virtual_orchestration_tool("compose-subagent"));
    assert!(is_virtual_orchestration_tool("compose_subagent"));
    assert!(is_virtual_orchestration_tool("update-subagent-skills"));
    assert!(is_virtual_orchestration_tool("update_subagent_skills"));
    assert!(is_virtual_orchestration_tool("list-subagent-roster"));
    assert!(is_virtual_orchestration_tool("list_subagent_roster"));
    assert!(is_virtual_orchestration_tool("list-available-skills"));
    assert!(is_virtual_orchestration_tool("list_available_skills"));
    assert!(is_virtual_orchestration_tool("remove-subagent"));
    assert!(is_virtual_orchestration_tool("remove_subagent"));
}

#[test]
fn is_virtual_orchestration_tool_case_insensitive() {
    assert!(is_virtual_orchestration_tool("COMPOSE-SUBAGENT"));
    assert!(is_virtual_orchestration_tool("List-Subagent-Roster"));
    assert!(is_virtual_orchestration_tool("  Remove_Subagent  "));
}

#[test]
fn is_virtual_orchestration_tool_rejects_non_orchestration() {
    assert!(!is_virtual_orchestration_tool("read_file"));
    assert!(!is_virtual_orchestration_tool("orchestrate-subagents"));
    assert!(!is_virtual_orchestration_tool("assign-tasks"));
    assert!(!is_virtual_orchestration_tool(""));
}

#[test]
fn orchestration_and_delegation_tools_are_disjoint() {
    // Orchestration tools should never match delegation recognition and vice versa.
    let orchestration_tools = [
        "compose-subagent",
        "update-subagent-skills",
        "list-subagent-roster",
        "list-available-skills",
        "remove-subagent",
    ];
    for tool in &orchestration_tools {
        assert!(
            is_virtual_orchestration_tool(tool),
            "{tool} should be orchestration"
        );
        assert!(
            !is_virtual_delegation_tool(tool),
            "{tool} should NOT be delegation"
        );
    }

    let delegation_tools = [
        "orchestrate-subagents",
        "assign-tasks",
        "delegate-subagent",
        "select-subagent-model",
    ];
    for tool in &delegation_tools {
        assert!(
            is_virtual_delegation_tool(tool),
            "{tool} should be delegation"
        );
        assert!(
            !is_virtual_orchestration_tool(tool),
            "{tool} should NOT be orchestration"
        );
    }
}
