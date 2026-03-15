mod channel_tests;
mod decomposition_tests;
mod delegation_tests;
mod diagnostics_tests;
mod guard_tests;
mod routing_tests;
mod tool_tests;

use super::*;
use chrono::Utc;

pub(super) fn test_config_with_scope(scope_mode: &str) -> ironclad_core::IroncladConfig {
    ironclad_core::IroncladConfig::from_str(&format!(
        r#"
[agent]
name = "TestBot"
id = "test-agent"

[server]
port = 0

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "{scope_mode}"
"#
    ))
    .unwrap()
}

pub(super) fn inbound_with_meta(meta: serde_json::Value) -> ironclad_channels::InboundMessage {
    ironclad_channels::InboundMessage {
        id: "msg-1".into(),
        platform: "telegram".into(),
        sender_id: "sender-1".into(),
        content: "hello".into(),
        timestamp: Utc::now(),
        metadata: Some(meta),
    }
}
