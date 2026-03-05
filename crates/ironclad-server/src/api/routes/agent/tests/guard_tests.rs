use super::*;

// ── subagent claim guard tests ───────────────────────────────

#[test]
fn subagent_claim_guard_blocks_unverified_live_delegation() {
    let fabricated =
        "[delegating to subagent: geopolitical specialist]\n\nGEOPOLITICAL FLASH UPDATE ...";
    let guarded =
        enforce_subagent_claim_guard(fabricated.to_string(), &DelegationProvenance::default());
    assert!(guarded.contains("I can't claim live subagent-produced output"));
}

#[test]
fn subagent_claim_guard_allows_when_delegated_this_turn() {
    let content = "[delegating to subagent: geopolitical specialist]";
    let guarded = enforce_subagent_claim_guard(
        content.to_string(),
        &DelegationProvenance {
            subagent_task_started: true,
            subagent_task_completed: true,
            subagent_result_attached: true,
        },
    );
    assert_eq!(guarded, content);
}

#[test]
fn subagent_claim_guard_blocks_standing_by_claim_without_provenance() {
    let fabricated = "Good. The subagents are actually running now - all 10 taskable subagents operational.\n\nGeopolitical Specialist: Standing by for tasking.";
    let guarded =
        enforce_subagent_claim_guard(fabricated.to_string(), &DelegationProvenance::default());
    assert!(guarded.contains("I can't claim live subagent-produced output"));
}

#[test]
fn subagent_claim_guard_blocks_subagent_generated_claim_without_provenance() {
    let fabricated =
        "Subagent-generated sitrep: geopolitical flash update with live delegated output.";
    let guarded =
        enforce_subagent_claim_guard(fabricated.to_string(), &DelegationProvenance::default());
    assert!(guarded.contains("I can't claim live subagent-produced output"));
}

#[test]
fn claim_detection_catches_live_delegation_markers() {
    assert!(claims_unverified_subagent_output(
        "[delegating to subagent: geo specialist]"
    ));
    assert!(claims_unverified_subagent_output(
        "Subagent Status - LIVE: running now"
    ));
    assert!(!claims_unverified_subagent_output(
        "Normal response without delegation claims."
    ));
}

// ── non-repetition guard tests ───────────────────────────────

#[test]
fn non_repetition_guard_rewrites_near_duplicate_output() {
    let prev = "The system appears stable. Monitoring remains active across all channels with no critical errors. I can continue watching and report any changes immediately.";
    let current = "The system appears stable. Monitoring remains active across all channels with no critical errors. I can continue watching and report any changes immediately.";
    let guarded = enforce_non_repetition(current.to_string(), Some(prev));
    assert!(guarded.contains("fresh check now"));
    assert_ne!(guarded, current);
}

#[test]
fn non_repetition_guard_keeps_distinct_output() {
    let prev =
        "Provider health is degraded and retries are being attempted through fallback models.";
    let current =
        "Two subagents are now running, one is still booting, and delegation is available.";
    let guarded = enforce_non_repetition(current.to_string(), Some(prev));
    assert_eq!(guarded, current);
}

#[test]
fn enforce_non_repetition_with_none_previous() {
    let response = "Some unique response";
    let result = enforce_non_repetition(response.to_string(), None);
    assert_eq!(result, response);
}

// ── execution truth guard tests ──────────────────────────────

#[test]
fn execution_truth_guard_blocks_unexecuted_command_suggestion() {
    let prompt = "Use a tool to list files in /Users/jmachen";
    let response =
        "You can use the following command: `ls /Users/jmachen | head -n 10`".to_string();
    let guarded = enforce_execution_truth_guard(prompt, response, &[]);
    assert!(guarded.contains("did not execute a tool"));
}

#[test]
fn execution_truth_guard_allows_verified_tool_output() {
    let prompt = "Run ls /Users/jmachen";
    let response = "/Users/jmachen\nApplications\nDesktop".to_string();
    let guarded = enforce_execution_truth_guard(
        prompt,
        response.clone(),
        &[("bash".to_string(), "Applications".to_string())],
    );
    assert_eq!(guarded, response);
}

#[test]
fn execution_truth_guard_blocks_unverified_delegation_claim() {
    let prompt = "Order a subagent to produce a sitrep.";
    let response = "Here is the sitrep from the geopolitical subagent: ...".to_string();
    let guarded = enforce_execution_truth_guard(prompt, response, &[]);
    assert!(guarded.contains("did not execute a delegated subagent task"));
}

#[test]
fn execution_truth_guard_blocks_failed_delegation_attempt() {
    let prompt = "Delegate this to a subagent.";
    let response = "Delegation complete.".to_string();
    let guarded = enforce_execution_truth_guard(
        prompt,
        response,
        &[(
            "assign-tasks".to_string(),
            "error: unknown tool".to_string(),
        )],
    );
    assert!(guarded.contains("did not execute a delegated subagent task"));
}

#[test]
fn execution_truth_guard_blocks_unverified_cron_claim() {
    let prompt = "Schedule a cron job every 5 minutes.";
    let response = "Use this crontab entry: */5 * * * *".to_string();
    let guarded = enforce_execution_truth_guard(prompt, response, &[]);
    assert!(guarded.contains("did not execute a cron scheduling tool"));
}

// ── model identity guard tests ───────────────────────────────

#[test]
fn model_identity_guard_corrects_mismatched_self_report() {
    let prompt = "Are you still on your current model?";
    let response = "I am currently on openai/gpt-5.3-codex.".to_string();
    let guarded = enforce_model_identity_truth_guard(prompt, response, "ollama/phi4-mini:latest");
    assert!(guarded.contains("ollama/phi4-mini:latest"));
}

#[test]
fn model_identity_guard_always_emits_canonical_model_for_identity_prompts() {
    let prompt = "What model are you running?";
    let response = "I am currently running on ollama/phi4-mini:latest.".to_string();
    let guarded =
        enforce_model_identity_truth_guard(prompt, response.clone(), "ollama/phi4-mini:latest");
    assert_eq!(
        guarded,
        "I am currently running on ollama/phi4-mini:latest."
    );
}

#[test]
fn model_identity_guard_handles_still_using_phrase() {
    let prompt = "Can you confirm for me that you are still using moonshot?";
    let response = "Yes, still moonshot.".to_string();
    let guarded = enforce_model_identity_truth_guard(prompt, response, "ollama/phi4-mini:latest");
    assert_eq!(
        guarded,
        "I am currently running on ollama/phi4-mini:latest."
    );
}

// ── repeat_tokens tests ──────────────────────────────────────

#[test]
fn repeat_tokens_extracts_lowercase_alpha_tokens() {
    let tokens = repeat_tokens("Hello World! Foo-Bar 42");
    assert!(tokens.contains("hello"));
    assert!(tokens.contains("world"));
    assert!(tokens.contains("foo"));
    assert!(tokens.contains("bar"));
    // "42" is only 2 chars, below the 3-char minimum
    assert!(!tokens.contains("42"));
}

#[test]
fn repeat_tokens_empty_input() {
    let tokens = repeat_tokens("");
    assert!(tokens.is_empty());
}

#[test]
fn repeat_tokens_deduplicates() {
    let tokens = repeat_tokens("hello hello hello");
    assert_eq!(tokens.len(), 1);
    assert!(tokens.contains("hello"));
}

// ── common_prefix_ratio tests ────────────────────────────────

#[test]
fn common_prefix_ratio_identical() {
    assert!((common_prefix_ratio("hello", "hello") - 1.0).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_no_common() {
    assert!((common_prefix_ratio("abc", "xyz") - 0.0).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_partial() {
    let ratio = common_prefix_ratio("abcdef", "abcxyz");
    // common prefix = "abc" (3 bytes), max_len = 6
    assert!((ratio - 0.5).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_empty_strings() {
    assert!((common_prefix_ratio("", "") - 0.0).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_one_empty() {
    assert!((common_prefix_ratio("abc", "") - 0.0).abs() < f64::EPSILON);
    assert!((common_prefix_ratio("", "abc") - 0.0).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_ascii() {
    assert!((common_prefix_ratio("hello", "hello") - 1.0).abs() < f64::EPSILON);
    assert!((common_prefix_ratio("hello", "world") - 0.0).abs() < f64::EPSILON);
    assert!((common_prefix_ratio("hello", "help") - 0.6).abs() < f64::EPSILON); // 3/5
    assert!((common_prefix_ratio("", "") - 0.0).abs() < f64::EPSILON);
}

#[test]
fn common_prefix_ratio_unicode() {
    // L-HIGH-2: must compare characters, not bytes
    let a = "\u{4F60}\u{597D}\u{4E16}\u{754C}"; // 你好世界
    let b = "\u{4F60}\u{597D}\u{5929}\u{6C14}"; // 你好天气
    // 2 shared chars out of 4 = 0.5
    let ratio = common_prefix_ratio(a, b);
    assert!(
        (ratio - 0.5).abs() < f64::EPSILON,
        "expected 0.5 for 2/4 shared CJK chars, got {ratio}"
    );
}

// ── looks_repetitive tests ───────────────────────────────────

#[test]
fn looks_repetitive_exact_match_case_insensitive() {
    assert!(looks_repetitive("Hello World", "hello world"));
}

#[test]
fn looks_repetitive_empty_inputs() {
    assert!(!looks_repetitive("", "some text"));
    assert!(!looks_repetitive("some text", ""));
    assert!(!looks_repetitive("", ""));
}

#[test]
fn looks_repetitive_short_but_different() {
    // Texts under 80 chars and not identical should not be flagged
    assert!(!looks_repetitive("Short text A", "Short text B"));
}

#[test]
fn looks_repetitive_high_overlap_long_texts() {
    let base = "The system monitoring is active and operational. All channels report normal status. There are no critical errors detected. Continuing to watch for changes and will report immediately.";
    let similar = "The system monitoring is active and operational. All channels report normal status. There are no critical errors detected. Continuing to watch for changes and will report at once.";
    assert!(looks_repetitive(base, similar));
}

#[test]
fn looks_repetitive_different_long_texts() {
    let a = "The provider health is degraded due to circuit breaker activation. Multiple fallback attempts are being tried through the configured fallback model list, but latency has increased significantly across all routes.";
    let b = "Two new subagent processes have started and are now fully operational. The geopolitical specialist is running with updated skills. The risk analysis agent has completed its initial calibration phase successfully.";
    assert!(!looks_repetitive(a, b));
}

// ── scope validation tests ───────────────────────────────────

#[test]
fn resolve_web_scope_respects_group_peer_and_agent_modes() {
    let mut req = AgentMessageRequest {
        content: "hello".into(),
        session_id: None,
        channel: Some("web".into()),
        sender_id: Some("user-1".into()),
        peer_id: None,
        group_id: Some("room-9".into()),
        is_group: Some(true),
    };

    let cfg_group = test_config_with_scope("group");
    let scope = resolve_web_scope(&cfg_group, &req).expect("group scope");
    assert_eq!(
        scope,
        ironclad_db::sessions::SessionScope::Group {
            group_id: "room-9".into(),
            channel: "web".into()
        }
    );

    let cfg_peer = test_config_with_scope("peer");
    req.group_id = None;
    let scope = resolve_web_scope(&cfg_peer, &req).expect("peer scope");
    assert_eq!(
        scope,
        ironclad_db::sessions::SessionScope::Peer {
            peer_id: "user-1".into(),
            channel: "web".into()
        }
    );

    let cfg_agent = test_config_with_scope("agent");
    let scope = resolve_web_scope(&cfg_agent, &req).expect("agent scope");
    assert_eq!(scope, ironclad_db::sessions::SessionScope::Agent);
}

#[test]
fn resolve_web_scope_rejects_missing_principal_in_peer_or_group_modes() {
    let req = AgentMessageRequest {
        content: "hello".into(),
        session_id: None,
        channel: Some("web".into()),
        sender_id: None,
        peer_id: None,
        group_id: None,
        is_group: Some(false),
    };

    let cfg_peer = test_config_with_scope("peer");
    assert!(resolve_web_scope(&cfg_peer, &req).is_err());

    let cfg_group = test_config_with_scope("group");
    assert!(resolve_web_scope(&cfg_group, &req).is_err());
}

#[test]
fn resolve_web_scope_rejects_oversized_scope_ids() {
    // S-MED-2: peer_id, group_id, channel capped at MAX_SCOPE_ID
    let long = "a".repeat(MAX_SCOPE_ID + 1);
    let cfg = test_config_with_scope("peer");
    let req = AgentMessageRequest {
        content: "hi".into(),
        session_id: None,
        channel: Some("web".into()),
        sender_id: None,
        peer_id: Some(long.clone()),
        group_id: None,
        is_group: None,
    };
    assert!(resolve_web_scope(&cfg, &req).is_err());

    let req2 = AgentMessageRequest {
        content: "hi".into(),
        session_id: None,
        channel: Some(long),
        sender_id: None,
        peer_id: Some("ok".into()),
        group_id: None,
        is_group: None,
    };
    assert!(resolve_web_scope(&cfg, &req2).is_err());
}

#[test]
fn resolve_web_scope_rejects_control_chars() {
    let cfg = test_config_with_scope("peer");
    let req = AgentMessageRequest {
        content: "hi".into(),
        session_id: None,
        channel: Some("web".into()),
        sender_id: None,
        peer_id: Some("user\x00evil".into()),
        group_id: None,
        is_group: None,
    };
    assert!(resolve_web_scope(&cfg, &req).is_err());
}
