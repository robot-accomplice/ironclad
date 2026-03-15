use super::*;
use crate::api::routes::agent::guards::{is_low_value_response, is_parroting_user_prompt};

// ── low-value response tests ─────────────────────────────────

#[test]
fn low_value_response_detector_flags_ready_and_status_loops() {
    assert!(is_low_value_response(
        "brainstorm revenue streams for agent self-funding",
        "ready"
    ));
    assert!(is_low_value_response(
        "brainstorm revenue streams for agent self-funding",
        "I await your insights"
    ));
    assert!(is_low_value_response(
        "brainstorm revenue streams for agent self-funding",
        "⚔️ Duncan is on it…\n\n🤖🧠…\n\nready"
    ));
}

#[test]
fn low_value_response_detector_allows_ack_only_prompts() {
    assert!(!is_low_value_response(
        "Acknowledge this request in one sentence and then wait.",
        "By your command, I acknowledge this request and will hold for your next instruction."
    ));
}

// ── parroting tests ──────────────────────────────────────────

#[test]
fn parroting_detector_flags_exact_echo() {
    let prompt = "I want a brainstorm on low-risk self-funding mechanisms.";
    let response = "I want a brainstorm on low-risk self-funding mechanisms.";
    assert!(is_parroting_user_prompt(prompt, response));
}

#[test]
fn parroting_detector_allows_explicit_repeat_requests() {
    let prompt = "Repeat exactly what I said.";
    let response = "Repeat exactly what I said.";
    assert!(!is_parroting_user_prompt(prompt, response));
}

#[test]
fn parroting_detector_allows_substantive_extension() {
    let prompt = "Brainstorm low-risk self-funding mechanisms.";
    let response = "Start with A2A micropaid services, narrow paid monitoring, and recurring reporting. Prioritize stable demand and strict cost caps.";
    assert!(!is_parroting_user_prompt(prompt, response));
}

// ── current-events truth guard tests ─────────────────────────

#[test]
fn current_events_guard_blocks_stale_knowledge_disclaimer() {
    let prompt = "What's the geopolitical situation?";
    let response =
        "As of my last update in early 2023, I cannot provide real-time updates.".to_string();
    let guarded = enforce_current_events_truth_guard(prompt, response);
    assert!(guarded.contains("cannot provide a current-events sitrep from stale memory"));
}

#[test]
fn current_events_guard_blocks_live_news_feed_capability_refusal() {
    let prompt = "What does the geopolitical sub agent say about goings on in the US?";
    let response = "I cannot provide real-time geopolitical analysis, as my capabilities do not include live news feeds or specialized geopolitical subagents.".to_string();
    let guarded = enforce_current_events_truth_guard(prompt, response);
    assert!(guarded.contains("cannot provide a current-events sitrep from stale memory"));
}

#[test]
fn current_events_guard_keeps_valid_current_events_response() {
    let prompt = "Give me a geopolitical sitrep";
    let response = "Acknowledged. I am retrieving a live sitrep now.".to_string();
    let guarded = enforce_current_events_truth_guard(prompt, response.clone());
    assert_eq!(guarded, response);
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
