use super::super::channel_message::format_channel_reply_for_delivery;
use super::super::strip_internal_delegation_metadata;
use super::*;

// ── metadata_str tests ───────────────────────────────────────

#[test]
fn metadata_str_reads_strings_and_numbers() {
    let meta = serde_json::json!({
        "chat_id": "chat-1",
        "channel_id": 123,
        "thread_id": 456u64
    });
    assert_eq!(
        metadata_str(Some(&meta), "/chat_id").as_deref(),
        Some("chat-1")
    );
    assert_eq!(
        metadata_str(Some(&meta), "/channel_id").as_deref(),
        Some("123")
    );
    assert_eq!(
        metadata_str(Some(&meta), "/thread_id").as_deref(),
        Some("456")
    );
    assert!(metadata_str(Some(&meta), "/missing").is_none());
}

#[test]
fn metadata_str_returns_none_for_none_meta() {
    assert!(metadata_str(None, "/chat_id").is_none());
}

#[test]
fn metadata_str_returns_none_for_non_matching_pointer() {
    let meta = serde_json::json!({"a": 1});
    assert!(metadata_str(Some(&meta), "/b").is_none());
}

#[test]
fn metadata_str_returns_none_for_bool_or_array() {
    let meta = serde_json::json!({"flag": true, "list": [1, 2]});
    assert!(metadata_str(Some(&meta), "/flag").is_none());
    assert!(metadata_str(Some(&meta), "/list").is_none());
}

// ── resolve_channel_chat_id tests ────────────────────────────

#[test]
fn resolve_channel_chat_id_uses_priority_and_fallback() {
    let inbound = inbound_with_meta(serde_json::json!({"chat_id": "chat-xyz"}));
    assert_eq!(resolve_channel_chat_id(&inbound), "chat-xyz");

    let inbound = inbound_with_meta(serde_json::json!({"message": {"chat": {"id": 777}}}));
    assert_eq!(resolve_channel_chat_id(&inbound), "777");

    let inbound = ironclad_channels::InboundMessage {
        id: "msg-2".into(),
        platform: "telegram".into(),
        sender_id: "sender-fallback".into(),
        content: "hi".into(),
        timestamp: Utc::now(),
        metadata: None,
    };
    assert_eq!(resolve_channel_chat_id(&inbound), "sender-fallback");
}

// ── resolve_channel_is_group tests ───────────────────────────

#[test]
fn resolve_channel_is_group_detects_flags_and_chat_type() {
    let inbound = inbound_with_meta(serde_json::json!({"is_group": true}));
    assert!(resolve_channel_is_group(&inbound));

    let inbound =
        inbound_with_meta(serde_json::json!({"message": {"chat": {"type": "supergroup"}}}));
    assert!(resolve_channel_is_group(&inbound));

    let inbound = inbound_with_meta(serde_json::json!({"message": {"chat": {"type": "private"}}}));
    assert!(!resolve_channel_is_group(&inbound));
}

// ── resolve_channel_scope tests ──────────────────────────────

#[test]
fn resolve_channel_scope_respects_config_mode() {
    let cfg_group = test_config_with_scope("group");
    let inbound_group = inbound_with_meta(serde_json::json!({"is_group": true}));
    let scope = resolve_channel_scope(&cfg_group, &inbound_group, "group-chat");
    assert_eq!(
        scope,
        ironclad_db::sessions::SessionScope::Group {
            group_id: "group-chat".into(),
            channel: "telegram".into()
        }
    );

    let cfg_peer = test_config_with_scope("peer");
    let inbound_peer = inbound_with_meta(serde_json::json!({}));
    let scope = resolve_channel_scope(&cfg_peer, &inbound_peer, "ignored");
    assert_eq!(
        scope,
        ironclad_db::sessions::SessionScope::Peer {
            peer_id: "sender-1".into(),
            channel: "telegram".into()
        }
    );

    let cfg_agent = test_config_with_scope("agent");
    let inbound_agent = inbound_with_meta(serde_json::json!({"is_group": true}));
    let scope = resolve_channel_scope(&cfg_agent, &inbound_agent, "group-chat");
    assert_eq!(scope, ironclad_db::sessions::SessionScope::Agent);
}

#[test]
fn resolve_channel_scope_non_group_in_group_mode_falls_to_peer() {
    let cfg = test_config_with_scope("group");
    // Non-group message in group mode falls back to peer
    let inbound = inbound_with_meta(serde_json::json!({}));
    let scope = resolve_channel_scope(&cfg, &inbound, "some-chat");
    assert_eq!(
        scope,
        ironclad_db::sessions::SessionScope::Peer {
            peer_id: "sender-1".into(),
            channel: "telegram".into()
        }
    );
}

// ── parse_skills_json tests ──────────────────────────────────

#[test]
fn parse_skills_json_handles_none_invalid_and_valid_payloads() {
    assert!(parse_skills_json(None).is_empty());
    assert!(parse_skills_json(Some("not-json")).is_empty());
    let parsed = parse_skills_json(Some(r#"["geo","risk-analysis"]"#));
    assert_eq!(parsed, vec!["geo".to_string(), "risk-analysis".to_string()]);
}

// ── channel output sanitation tests ──────────────────────────

#[test]
fn strip_internal_delegation_metadata_removes_internal_lines() {
    let raw = "\
delegated_subagent=geopolitical-sitrep model=openrouter/auto fallback_models=[\"a\",\"b\"]\n\
subtask 1 -> geopolitical-sitrep\n\
**Global update**\n\
notes=timeout guardrail rerouted\n\
Key point one\n\
Key point two";
    let cleaned = strip_internal_delegation_metadata(raw);
    assert!(!cleaned.contains("delegated_subagent="));
    assert!(!cleaned.contains("subtask 1 ->"));
    assert!(!cleaned.contains("notes="));
    assert!(cleaned.contains("Global update"));
    assert!(cleaned.contains("Key point one"));
}

#[test]
fn strip_internal_delegation_metadata_removes_orchestration_narrative_lines() {
    let raw = "\
Centralized delegation is sensible for a simple, single-step task.\n\
Expected utility margin: -0.10\n\
Rationale: task is single-step\n\
Subtasks:\n\
- collect data\n\
Actual user-facing answer.";
    let cleaned = strip_internal_delegation_metadata(raw);
    assert!(
        !cleaned
            .to_ascii_lowercase()
            .contains("centralized delegation")
    );
    assert!(
        !cleaned
            .to_ascii_lowercase()
            .contains("expected utility margin")
    );
    assert!(!cleaned.to_ascii_lowercase().contains("rationale:"));
    assert!(!cleaned.to_ascii_lowercase().contains("subtasks:"));
    assert!(cleaned.contains("Actual user-facing answer."));
}

#[test]
fn strip_internal_delegation_metadata_removes_tool_protocol_leakage() {
    let raw = "\
{\"tool_call\":{\"name\":\"bash\",\"params\":{\"command\":\"ls\"}}}\n\
unexecuted_streaming_tool_call: {\"command\":\"ls\"}\n\
Visible answer line.";
    let cleaned = strip_internal_delegation_metadata(raw);
    assert!(!cleaned.contains("\"tool_call\""));
    assert!(!cleaned.contains("unexecuted_streaming_tool_call"));
    assert!(cleaned.contains("Visible answer line."));
}

#[test]
fn strip_internal_delegation_metadata_returns_empty_when_all_internal() {
    let raw = "delegated_subagent=geo model=x fallback_models=[]\nsubtask 1 -> geo";
    let cleaned = strip_internal_delegation_metadata(raw);
    assert!(cleaned.is_empty());
}

#[test]
fn format_channel_reply_for_delivery_normalizes_telegram_markdown() {
    let raw = "\
# Heading\n\
**bold** and `code`\n\
[Delegated to geo]\n\
line";
    let out = format_channel_reply_for_delivery("telegram", raw);
    // ** markers converted to Telegram MarkdownV2 *bold*
    assert!(!out.contains("**"));
    // Delegation metadata stripped
    assert!(!out.contains("Delegated to"));
    // Content preserved (headers → bold, inline formatting kept)
    assert!(out.contains("Heading"));
    assert!(out.contains("bold"));
    assert!(out.contains("line"));
}

#[test]
fn format_channel_reply_for_delivery_strips_numeric_citations_for_telegram() {
    let raw = "Situation remains tense across regions.[1][2]\nKey trend: supply chain splits.[14]";
    let out = format_channel_reply_for_delivery("telegram", raw);
    // Numeric bracket citations stripped
    assert!(!out.contains("[1]"));
    assert!(!out.contains("[2]"));
    assert!(!out.contains("[14]"));
    // Content preserved (periods escaped for Telegram MarkdownV2)
    assert!(out.contains("Situation remains tense across regions"));
    assert!(out.contains("Key trend: supply chain splits"));
}
