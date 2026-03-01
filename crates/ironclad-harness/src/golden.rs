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
    }
}
