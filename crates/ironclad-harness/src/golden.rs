//! Golden fixture loader for deterministic LLM response mocking.
//!
//! Golden files live in `fixtures/golden/*.json` and are compiled into the
//! binary via `include_str!` — no filesystem access needed at runtime.

use serde_json::Value;

/// Pre-compiled golden fixtures. Each is a valid OpenAI-format response body.
pub struct Golden;

impl Golden {
    /// Simple chat completion — assistant returns a text answer.
    pub fn chat_simple() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/chat_simple.json"))
            .expect("chat_simple.json is invalid JSON")
    }

    /// Tool-calling response — assistant requests a function call.
    pub fn chat_tool_use() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/chat_tool_use.json"))
            .expect("chat_tool_use.json is invalid JSON")
    }

    /// Rate limit error (HTTP 429).
    pub fn error_429() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/error_429.json"))
            .expect("error_429.json is invalid JSON")
    }

    /// Internal server error (HTTP 500).
    pub fn error_500() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/error_500.json"))
            .expect("error_500.json is invalid JSON")
    }

    /// Delegation tool call — assistant invokes `orchestrate-subagents`.
    pub fn chat_delegation() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/chat_delegation.json"))
            .expect("chat_delegation.json is invalid JSON")
    }

    /// Delegation follow-up — assistant summarizes after tool execution.
    pub fn chat_delegation_followup() -> Value {
        serde_json::from_str(include_str!(
            "../fixtures/golden/chat_delegation_followup.json"
        ))
        .expect("chat_delegation_followup.json is invalid JSON")
    }

    /// Echo tool call — LLM invokes the `echo` tool (always registered, Safe risk).
    pub fn chat_echo_tool_call() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/chat_echo_tool_call.json"))
            .expect("chat_echo_tool_call.json is invalid JSON")
    }

    /// Echo follow-up — assistant text response after echo tool execution.
    pub fn chat_echo_followup() -> Value {
        serde_json::from_str(include_str!("../fixtures/golden/chat_echo_followup.json"))
            .expect("chat_echo_followup.json is invalid JSON")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn golden_fixtures_parse() {
        // Verify all golden files are valid JSON and have expected structure
        let simple = Golden::chat_simple();
        assert_eq!(
            simple["choices"][0]["message"]["role"].as_str(),
            Some("assistant")
        );

        let tool = Golden::chat_tool_use();
        assert_eq!(
            tool["choices"][0]["finish_reason"].as_str(),
            Some("tool_calls")
        );

        let e429 = Golden::error_429();
        assert!(e429["error"]["message"].is_string());

        let e500 = Golden::error_500();
        assert!(e500["error"]["message"].is_string());

        let deleg = Golden::chat_delegation();
        assert_eq!(
            deleg["choices"][0]["message"]["tool_calls"][0]["function"]["name"].as_str(),
            Some("orchestrate-subagents")
        );

        let followup = Golden::chat_delegation_followup();
        assert_eq!(
            followup["choices"][0]["finish_reason"].as_str(),
            Some("stop")
        );
        assert!(followup["choices"][0]["message"]["content"].is_string());

        let echo_tc = Golden::chat_echo_tool_call();
        assert_eq!(
            echo_tc["choices"][0]["message"]["tool_calls"][0]["function"]["name"].as_str(),
            Some("echo")
        );

        let echo_fu = Golden::chat_echo_followup();
        assert_eq!(
            echo_fu["choices"][0]["finish_reason"].as_str(),
            Some("stop")
        );
    }
}
