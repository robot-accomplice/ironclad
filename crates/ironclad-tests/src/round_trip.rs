use ironclad_core::{ApiFormat, IroncladConfig};
use ironclad_db::Database;
use ironclad_llm::format::{
    UnifiedMessage, UnifiedRequest, translate_request, translate_response,
};

fn test_config() -> IroncladConfig {
    IroncladConfig::from_str(
        r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
"#,
    )
    .unwrap()
}

#[test]
fn session_message_and_llm_format_roundtrip() {
    let db = Database::new(":memory:").unwrap();
    let _config = test_config();

    let session_id = ironclad_db::sessions::find_or_create(&db, "test-agent").unwrap();

    ironclad_db::sessions::append_message(&db, &session_id, "user", "What is Rust?").unwrap();

    let messages = ironclad_db::sessions::list_messages(&db, &session_id, None).unwrap();
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].role, "user");
    assert_eq!(messages[0].content, "What is Rust?");

    let unified = UnifiedRequest {
        model: "claude-sonnet-4-20250514".into(),
        messages: messages
            .iter()
            .map(|m| UnifiedMessage {
                role: m.role.clone(),
                content: m.content.clone(),
            })
            .collect(),
        max_tokens: Some(1024),
        temperature: Some(0.7),
        system: Some("You are a helpful programming assistant.".into()),
    };

    let anthropic_body = translate_request(&unified, ApiFormat::AnthropicMessages).unwrap();
    assert_eq!(anthropic_body["model"], "claude-sonnet-4-20250514");
    assert_eq!(
        anthropic_body["system"],
        "You are a helpful programming assistant."
    );
    let api_msgs = anthropic_body["messages"].as_array().unwrap();
    assert_eq!(api_msgs.len(), 1);
    assert_eq!(api_msgs[0]["role"], "user");
    assert_eq!(api_msgs[0]["content"], "What is Rust?");

    let mock_response = serde_json::json!({
        "content": [{"type": "text", "text": "Rust is a systems programming language."}],
        "model": "claude-sonnet-4-20250514",
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 42, "output_tokens": 18}
    });

    let unified_resp =
        translate_response(&mock_response, ApiFormat::AnthropicMessages).unwrap();
    assert_eq!(
        unified_resp.content,
        "Rust is a systems programming language."
    );
    assert_eq!(unified_resp.tokens_in, 42);
    assert_eq!(unified_resp.tokens_out, 18);

    ironclad_db::sessions::append_message(&db, &session_id, "assistant", &unified_resp.content)
        .unwrap();

    let cost_per_token_in = 0.000003;
    let cost_per_token_out = 0.000015;
    let cost = (unified_resp.tokens_in as f64 * cost_per_token_in)
        + (unified_resp.tokens_out as f64 * cost_per_token_out);

    ironclad_db::metrics::record_inference_cost(
        &db,
        &unified_resp.model,
        "anthropic",
        unified_resp.tokens_in as i64,
        unified_resp.tokens_out as i64,
        cost,
        Some("T3"),
        false,
    )
    .unwrap();

    let final_messages = ironclad_db::sessions::list_messages(&db, &session_id, None).unwrap();
    assert_eq!(final_messages.len(), 2);
    assert_eq!(final_messages[0].role, "user");
    assert_eq!(final_messages[1].role, "assistant");
    assert_eq!(
        final_messages[1].content,
        "Rust is a systems programming language."
    );
}

#[test]
fn multi_format_translation_consistency() {
    let req = UnifiedRequest {
        model: "test-model".into(),
        messages: vec![
            UnifiedMessage {
                role: "user".into(),
                content: "Hello".into(),
            },
            UnifiedMessage {
                role: "assistant".into(),
                content: "Hi".into(),
            },
        ],
        max_tokens: Some(512),
        temperature: None,
        system: Some("Be brief.".into()),
    };

    for format in [
        ApiFormat::AnthropicMessages,
        ApiFormat::OpenAiCompletions,
        ApiFormat::OpenAiResponses,
        ApiFormat::GoogleGenerativeAi,
    ] {
        let body = translate_request(&req, format).unwrap();
        assert!(
            body.is_object(),
            "format {format:?} should produce a JSON object"
        );
    }
}
