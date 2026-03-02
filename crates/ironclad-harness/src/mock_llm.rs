//! WireMock-based LLM response server.
//!
//! Intercepts the HTTP calls that `ironclad-llm`'s client makes to LLM providers
//! and returns canned golden responses. Zero code changes to ironclad-server needed —
//! we simply set the provider's `url` in the config to point at the WireMock server.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use ironclad_harness::mock_llm::MockLlmServer;
//! use ironclad_harness::golden::Golden;
//!
//! #[tokio::test]
//! async fn agent_chat() {
//!     let mock = MockLlmServer::start().await;
//!     mock.enqueue_response(Golden::chat_simple()).await;
//!     // ... spawn sandbox with mock.base_url() as provider URL ...
//! }
//! ```

use serde_json::Value;
use std::sync::atomic::{AtomicUsize, Ordering};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, Respond, ResponseTemplate};

/// A WireMock-backed server that impersonates an LLM provider.
pub struct MockLlmServer {
    server: MockServer,
}

impl MockLlmServer {
    /// Start a new mock LLM server on a random available port.
    pub async fn start() -> Self {
        let server = MockServer::start().await;
        Self { server }
    }

    /// Base URL (e.g., `http://127.0.0.1:PORT`) — use as the provider's `url` in config.
    pub fn base_url(&self) -> String {
        self.server.uri()
    }

    /// Enqueue a successful (200) chat completion response.
    ///
    /// The mock matches `POST /v1/chat/completions` (OpenAI-format path).
    /// Responses are served FIFO — first enqueue is returned to first request.
    pub async fn enqueue_response(&self, body: Value) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json"),
            )
            .expect(1)
            .with_priority(1) // high priority — wins over fallback
            .mount(&self.server)
            .await;
    }

    /// Enqueue an error response with the given status code.
    pub async fn enqueue_error(&self, status: u16, body: Value) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json"),
            )
            .expect(1)
            .mount(&self.server)
            .await;
    }

    /// Enqueue a response that arrives after a delay (for timeout testing).
    pub async fn enqueue_slow_response(&self, body: Value, delay: std::time::Duration) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json")
                    .set_delay(delay),
            )
            .expect(1)
            .mount(&self.server)
            .await;
    }

    /// Enqueue a response that matches ANY POST path (for non-standard endpoints).
    pub async fn enqueue_any_post(&self, body: Value) {
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json"),
            )
            .expect(1)
            .mount(&self.server)
            .await;
    }

    /// Enqueue a mock that expects a flexible number of requests.
    ///
    /// Accepts anything `wiremock::Times` supports: `2` (exact), `2..` (at least 2),
    /// `2..=4` (between 2 and 4), etc. Useful when background tasks (nickname
    /// refinement) may or may not fire additional LLM requests.
    pub async fn enqueue_responses(&self, body: Value, times: impl Into<wiremock::Times>) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json"),
            )
            .expect(times)
            .mount(&self.server)
            .await;
    }

    /// Mount a permissive fallback that absorbs any extra POST requests
    /// (e.g., background nickname refinement, metrics, etc.).
    ///
    /// Uses WireMock priority 10 (low) so specific enqueued mocks take precedence.
    /// Does **not** set an `.expect()` — any number of overflow hits is fine.
    pub async fn mount_fallback(&self, body: Value) {
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json"),
            )
            .with_priority(10) // low priority — specific mocks win
            .mount(&self.server)
            .await;
    }

    /// Mount a sequenced responder that returns different responses for
    /// successive requests. The first request gets `responses[0]`, the second
    /// gets `responses[1]`, etc. After the list is exhausted, the last
    /// response is repeated for any overflow (background tasks, etc.).
    ///
    /// This is essential for ReAct-loop testing where the first LLM call
    /// must return a tool_call and subsequent calls return text.
    ///
    /// NOTE: WireMock's `expect(n)` is verification-only — it does NOT
    /// deactivate a mock after `n` matches. A single mock with a stateful
    /// responder is the correct way to serve different responses in order.
    pub async fn enqueue_sequence(&self, responses: Vec<Value>) {
        assert!(
            !responses.is_empty(),
            "enqueue_sequence requires at least one response"
        );
        let templates: Vec<ResponseTemplate> = responses
            .into_iter()
            .map(|body| {
                ResponseTemplate::new(200)
                    .set_body_json(&body)
                    .insert_header("content-type", "application/json")
            })
            .collect();
        Mock::given(method("POST"))
            .and(path("/v1/chat/completions"))
            .respond_with(SequencedResponder {
                responses: templates,
                counter: AtomicUsize::new(0),
            })
            .mount(&self.server)
            .await;
    }

    /// Verify all expected requests were received.
    /// Panics if any enqueued mock was not consumed.
    pub async fn verify(&self) {
        // WireMock's `expect()` assertions are checked on drop,
        // but calling this explicitly gives better error messages.
        self.server.verify().await;
    }

    /// How many requests have been received so far.
    pub async fn request_count(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap_or_default()
            .len()
    }
}

/// Stateful responder that serves different responses in sequence.
///
/// Returns `responses[0]` for the first request, `responses[1]` for the second,
/// etc. After the list is exhausted, the last response repeats indefinitely.
struct SequencedResponder {
    responses: Vec<ResponseTemplate>,
    counter: AtomicUsize,
}

impl Respond for SequencedResponder {
    fn respond(&self, _request: &Request) -> ResponseTemplate {
        let idx = self.counter.fetch_add(1, Ordering::SeqCst);
        // Clamp to the last response for overflow
        let effective = idx.min(self.responses.len() - 1);
        self.responses[effective].clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::golden::Golden;

    #[tokio::test]
    async fn mock_serves_golden_response() {
        let mock = MockLlmServer::start().await;
        mock.enqueue_response(Golden::chat_simple()).await;

        // Simulate what ironclad-llm's client does
        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/chat/completions", mock.base_url()))
            .json(&serde_json::json!({"model": "test", "messages": []}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await.unwrap();
        assert_eq!(
            body["choices"][0]["message"]["role"].as_str(),
            Some("assistant")
        );
        assert_eq!(body["usage"]["prompt_tokens"].as_u64(), Some(42));
    }

    #[tokio::test]
    async fn mock_serves_error() {
        let mock = MockLlmServer::start().await;
        mock.enqueue_error(429, Golden::error_429()).await;

        let client = reqwest::Client::new();
        let resp = client
            .post(format!("{}/v1/chat/completions", mock.base_url()))
            .json(&serde_json::json!({"model": "test", "messages": []}))
            .send()
            .await
            .unwrap();

        assert_eq!(resp.status(), 429);
    }
}
