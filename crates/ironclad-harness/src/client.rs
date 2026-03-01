//! Typed HTTP client wrapper for harness tests.
//!
//! [`HarnessClient`] wraps `reqwest::Client` with the sandbox's base URL
//! and optional API key, providing ergonomic methods for common HTTP operations.

use reqwest::{Response, StatusCode};
use serde_json::Value;

/// HTTP client pre-configured for a sandboxed server.
#[derive(Clone)]
pub struct HarnessClient {
    http: reqwest::Client,
    base_url: String,
    api_key: Option<String>,
}

impl HarnessClient {
    pub fn new(base_url: &str, api_key: Option<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.to_string(),
            api_key,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base_url)
    }

    fn apply_auth(&self, req: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        if let Some(ref key) = self.api_key {
            req.header("x-api-key", key)
        } else {
            req
        }
    }

    // ── Raw request methods ──────────────────────────────────

    pub async fn get(&self, path: &str) -> Result<Response, reqwest::Error> {
        let req = self.http.get(self.url(path));
        self.apply_auth(req).send().await
    }

    pub async fn post_json(&self, path: &str, body: &Value) -> Result<Response, reqwest::Error> {
        let req = self.http.post(self.url(path)).json(body);
        self.apply_auth(req).send().await
    }

    pub async fn put_json(&self, path: &str, body: &Value) -> Result<Response, reqwest::Error> {
        let req = self.http.put(self.url(path)).json(body);
        self.apply_auth(req).send().await
    }

    pub async fn delete(&self, path: &str) -> Result<Response, reqwest::Error> {
        let req = self.http.delete(self.url(path));
        self.apply_auth(req).send().await
    }

    // ── Convenience methods (assert 2xx + parse JSON) ────────

    /// GET and assert success, return parsed JSON body.
    pub async fn get_ok(&self, path: &str) -> Result<Value, Box<dyn std::error::Error>> {
        let resp = self.get(path).await?;
        let status = resp.status();
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GET {path} returned {status}: {body}").into());
        }
        Ok(resp.json().await?)
    }

    /// POST JSON and assert success, return parsed JSON body.
    pub async fn post_ok(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<Value, Box<dyn std::error::Error>> {
        let resp = self.post_json(path, body).await?;
        let status = resp.status();
        if !status.is_success() {
            let body_text = resp.text().await.unwrap_or_default();
            return Err(format!("POST {path} returned {status}: {body_text}").into());
        }
        Ok(resp.json().await?)
    }

    /// GET and assert a specific status code.
    pub async fn get_expect(
        &self,
        path: &str,
        expected: StatusCode,
    ) -> Result<Response, Box<dyn std::error::Error>> {
        let resp = self.get(path).await?;
        let actual = resp.status();
        if actual != expected {
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("GET {path}: expected {expected}, got {actual}: {body}").into());
        }
        Ok(resp)
    }
}
