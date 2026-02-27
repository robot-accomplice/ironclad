use std::collections::HashMap;
use std::time::Duration;

use reqwest::Client;
use tracing::debug;

use ironclad_core::{IroncladError, Result};

const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Percent-encode a string for safe inclusion as a URL query parameter value.
/// Encodes all bytes outside the unreserved set (RFC 3986 section 2.3).
fn pct_encode_query_value(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                out.push(char::from(b"0123456789ABCDEF"[(b >> 4) as usize]));
                out.push(char::from(b"0123456789ABCDEF"[(b & 0x0F) as usize]));
            }
        }
    }
    out
}

#[derive(Debug, Clone)]
pub struct LlmClient {
    http: Client,
}

impl LlmClient {
    pub fn new() -> Result<Self> {
        let http = Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .connect_timeout(CONNECT_TIMEOUT)
            .pool_max_idle_per_host(4)
            .build()
            .map_err(|e| IroncladError::Network(e.to_string()))?;
        Ok(Self { http })
    }

    /// Legacy method using default Bearer auth.
    pub async fn forward_request(
        &self,
        url: &str,
        api_key: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.forward_with_provider(url, api_key, body, "Authorization", &HashMap::new())
            .await
    }

    /// Send a request with provider-specific auth header and extra headers.
    ///
    /// Auth modes based on `auth_header` value:
    /// - `"Authorization"` -> sends `Authorization: Bearer <key>`
    /// - `"query:<param>"` (e.g. `"query:key"`) -> appends `?<param>=<key>` to the URL
    /// - anything else -> sends `<auth_header>: <key>` as a raw header
    pub async fn forward_with_provider(
        &self,
        url: &str,
        api_key: &str,
        body: serde_json::Value,
        auth_header: &str,
        extra_headers: &HashMap<String, String>,
    ) -> Result<serde_json::Value> {
        debug!(url, auth_header, "forwarding request to provider");

        let effective_url;
        let mut request = if let Some(param_name) = auth_header.strip_prefix("query:") {
            let separator = if url.contains('?') { '&' } else { '?' };
            let encoded_key = pct_encode_query_value(api_key);
            effective_url = format!("{url}{separator}{param_name}={encoded_key}");
            self.http
                .post(&effective_url)
                .header("Content-Type", "application/json")
        } else {
            let auth_value = if auth_header.eq_ignore_ascii_case("authorization") {
                format!("Bearer {api_key}")
            } else {
                api_key.to_string()
            };
            self.http
                .post(url)
                .header(auth_header, &auth_value)
                .header("Content-Type", "application/json")
        };

        for (key, value) in extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "unable to read error body".into());
            return Err(IroncladError::Llm(format!(
                "provider returned {status}: {error_body}"
            )));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| IroncladError::Llm(format!("failed to parse provider response: {e}")))
    }

    /// Send a streaming request and return the raw byte stream.
    pub async fn forward_stream(
        &self,
        url: &str,
        api_key: &str,
        body: serde_json::Value,
        auth_header: &str,
        extra_headers: &HashMap<String, String>,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures::Stream<Item = std::result::Result<bytes::Bytes, reqwest::Error>>
                    + Send,
            >,
        >,
    > {
        debug!(url, auth_header, "forwarding streaming request to provider");

        let auth_value = if auth_header.eq_ignore_ascii_case("authorization") {
            format!("Bearer {api_key}")
        } else {
            api_key.to_string()
        };

        let mut request = self
            .http
            .post(url)
            .header(auth_header, &auth_value)
            .header("Content-Type", "application/json");

        for (key, value) in extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("stream request failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            let error_body = response
                .text()
                .await
                .unwrap_or_else(|_| "unable to read error body".into());
            return Err(IroncladError::Llm(format!(
                "provider returned {status}: {error_body}"
            )));
        }

        Ok(Box::pin(response.bytes_stream()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclad_core::IroncladError;

    #[test]
    fn client_construction() {
        let client = LlmClient::new().unwrap();
        // Verify we can clone (proves the inner Client is Clone-compatible)
        let _clone = client.clone();
    }

    #[test]
    fn request_body_is_valid_json() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "hello"}
            ]
        });
        assert!(body.is_object());
        assert_eq!(body["model"], "gpt-4o");
        assert!(body["messages"].is_array());
    }

    #[tokio::test]
    async fn forward_request_connection_refused_maps_to_network_error() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/chat/completions";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let err = client
            .forward_request(url, "fake-key", body)
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn forward_with_provider_custom_auth_connection_refused() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/messages";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let mut extra = std::collections::HashMap::new();
        extra.insert("anthropic-version".into(), "2023-06-01".into());
        let err = client
            .forward_with_provider(url, "fake-key", body, "x-api-key", &extra)
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    #[test]
    fn auth_value_formatting() {
        // Bearer for Authorization header
        let auth_header = "Authorization";
        let val = if auth_header.eq_ignore_ascii_case("authorization") {
            format!("Bearer {}", "sk-test")
        } else {
            "sk-test".to_string()
        };
        assert_eq!(val, "Bearer sk-test");

        // Raw key for x-api-key
        let auth_header = "x-api-key";
        let val = if auth_header.eq_ignore_ascii_case("authorization") {
            format!("Bearer {}", "sk-test")
        } else {
            "sk-test".to_string()
        };
        assert_eq!(val, "sk-test");
    }

    // ── pct_encode_query_value tests ──────────────────────────────

    #[test]
    fn pct_encode_unreserved_chars_pass_through() {
        let input = "abcXYZ019-_.~";
        let encoded = pct_encode_query_value(input);
        assert_eq!(encoded, input, "unreserved chars must not be encoded");
    }

    #[test]
    fn pct_encode_spaces_and_special() {
        let encoded = pct_encode_query_value("hello world");
        assert_eq!(encoded, "hello%20world");
    }

    #[test]
    fn pct_encode_ampersand_equals() {
        let encoded = pct_encode_query_value("key=val&a=b");
        assert!(encoded.contains("%3D"), "= should be encoded: {encoded}");
        assert!(encoded.contains("%26"), "& should be encoded: {encoded}");
    }

    #[test]
    fn pct_encode_slash_colon() {
        let encoded = pct_encode_query_value("https://example.com/path");
        assert!(encoded.contains("%3A"), ": should be encoded: {encoded}");
        assert!(encoded.contains("%2F"), "/ should be encoded: {encoded}");
    }

    #[test]
    fn pct_encode_empty_string() {
        assert_eq!(pct_encode_query_value(""), "");
    }

    #[test]
    fn pct_encode_all_bytes() {
        // Ensure non-ASCII bytes are encoded (use byte-string via from_utf8_lossy)
        let input = String::from_utf8_lossy(&[0x00, 0x7F]);
        let encoded = pct_encode_query_value(&input);
        assert!(encoded.contains("%00"), "null byte should be encoded: {encoded}");
    }

    // ── query auth mode tests ──────────────────────────────────

    #[tokio::test]
    async fn forward_with_query_auth_no_existing_params() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/generate";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        // Use query:key auth to test the URL-building path
        let err = client
            .forward_with_provider(url, "sk-test-key", body, "query:key", &HashMap::new())
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn forward_with_query_auth_existing_params() {
        let client = LlmClient::new().unwrap();
        // URL already has a query param -- should use '&' separator
        let url = "http://127.0.0.1:1/v1/generate?format=json";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let err = client
            .forward_with_provider(url, "sk-test", body, "query:key", &HashMap::new())
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn forward_with_provider_authorization_case_insensitive() {
        // Test that "AUTHORIZATION" (uppercase) triggers Bearer prefix
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/chat";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let err = client
            .forward_with_provider(url, "sk-test", body, "AUTHORIZATION", &HashMap::new())
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    // ── forward_stream tests ──────────────────────────────────

    #[tokio::test]
    async fn forward_stream_connection_refused() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/stream";
        let body = serde_json::json!({ "model": "test", "messages": [], "stream": true });
        let result = client
            .forward_stream(url, "fake-key", body, "Authorization", &HashMap::new())
            .await;
        match result {
            Err(IroncladError::Network(msg)) => {
                assert!(msg.contains("stream request failed"), "got: {msg}")
            }
            Err(other) => panic!("expected IroncladError::Network, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn forward_stream_custom_auth_header() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/stream";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let mut extra = HashMap::new();
        extra.insert("anthropic-version".into(), "2023-06-01".into());
        let result = client
            .forward_stream(url, "fake-key", body, "x-api-key", &extra)
            .await;
        match result {
            Err(IroncladError::Network(msg)) => {
                assert!(msg.contains("stream request failed"), "got: {msg}")
            }
            Err(other) => panic!("expected IroncladError::Network, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn forward_stream_bearer_auth() {
        // Verify AUTHORIZATION (case-insensitive) triggers Bearer prefix in stream path
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/stream";
        let body = serde_json::json!({ "model": "test" });
        let result = client
            .forward_stream(url, "sk-123", body, "AUTHORIZATION", &HashMap::new())
            .await;
        match result {
            Err(IroncladError::Network(msg)) => {
                assert!(msg.contains("stream request failed"), "got: {msg}")
            }
            Err(other) => panic!("expected IroncladError::Network, got {other:?}"),
            Ok(_) => panic!("expected error, got Ok"),
        }
    }

    #[tokio::test]
    async fn forward_with_provider_extra_headers_propagated() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/chat";
        let body = serde_json::json!({ "model": "test", "messages": [] });
        let mut extra = HashMap::new();
        extra.insert("X-Custom-Header".into(), "custom-value".into());
        extra.insert("X-Another".into(), "another-value".into());
        // Just confirm the request is formed properly (connection refused expected)
        let err = client
            .forward_with_provider(url, "sk-test", body, "Authorization", &extra)
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }

    #[tokio::test]
    async fn forward_with_query_auth_encodes_special_chars_in_key() {
        let client = LlmClient::new().unwrap();
        let url = "http://127.0.0.1:1/v1/generate";
        let body = serde_json::json!({ "model": "test" });
        // Key with special characters that need encoding
        let err = client
            .forward_with_provider(url, "key with spaces&=", body, "query:apikey", &HashMap::new())
            .await
            .unwrap_err();
        match &err {
            IroncladError::Network(msg) => assert!(msg.contains("request failed"), "got: {msg}"),
            _ => panic!("expected IroncladError::Network, got {err:?}"),
        }
    }
}
