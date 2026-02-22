use std::collections::HashMap;

use reqwest::Client;
use serde_json::json;
use tracing::{debug, warn};

use ironclad_core::{ApiFormat, IroncladError, Result};

const NGRAM_DIM: usize = 128;
const EMBED_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Debug, Clone)]
pub struct EmbeddingConfig {
    pub base_url: String,
    pub embedding_path: String,
    pub model: String,
    pub dimensions: usize,
    pub format: ApiFormat,
    pub api_key_env: String,
    pub auth_header: String,
    pub extra_headers: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct EmbeddingClient {
    http: Client,
    config: Option<EmbeddingConfig>,
}

impl EmbeddingClient {
    pub fn new(config: Option<EmbeddingConfig>) -> Result<Self> {
        let http = Client::builder()
            .timeout(EMBED_TIMEOUT)
            .connect_timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| IroncladError::Network(e.to_string()))?;
        Ok(Self { http, config })
    }

    pub fn has_provider(&self) -> bool {
        self.config.is_some()
    }

    pub fn dimensions(&self) -> usize {
        self.config
            .as_ref()
            .map(|c| c.dimensions)
            .unwrap_or(NGRAM_DIM)
    }

    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>> {
        match &self.config {
            Some(cfg) => self.embed_via_provider(cfg, texts).await,
            None => Ok(texts.iter().map(|t| fallback_ngram(t, NGRAM_DIM)).collect()),
        }
    }

    pub async fn embed_single(&self, text: &str) -> Result<Vec<f32>> {
        let results = self.embed(&[text]).await?;
        results
            .into_iter()
            .next()
            .ok_or_else(|| IroncladError::Llm("empty embedding response".into()))
    }

    async fn embed_via_provider(
        &self,
        cfg: &EmbeddingConfig,
        texts: &[&str],
    ) -> Result<Vec<Vec<f32>>> {
        let api_key = std::env::var(&cfg.api_key_env).unwrap_or_default();
        let url = build_embedding_url(cfg, texts.len());
        let body = build_embedding_request(cfg, texts);

        let log_url = if url.contains('?') { url.split('?').next().unwrap_or(&url) } else { &url };
        debug!(url = %log_url, model = %cfg.model, count = texts.len(), "embedding request");

        let is_query_auth = cfg.auth_header.starts_with("query:");

        let mut request = self.http.post(&url).header("Content-Type", "application/json");

        if !is_query_auth {
            let auth_value = if cfg.auth_header.eq_ignore_ascii_case("authorization") {
                format!("Bearer {api_key}")
            } else {
                api_key.clone()
            };
            request = request.header(&cfg.auth_header, &auth_value);
        }

        for (key, value) in &cfg.extra_headers {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request
            .json(&body)
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("embedding request failed: {e}")))?;

        let status = response.status();
        if !status.is_success() {
            let error_body = response.text().await.unwrap_or_default();
            warn!(%status, %error_body, "embedding provider error");
            return Err(IroncladError::Llm(format!(
                "embedding provider returned {status}: {error_body}"
            )));
        }

        let resp_json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| IroncladError::Llm(format!("embedding response parse error: {e}")))?;

        parse_embedding_response(cfg, &resp_json, texts.len())
    }
}

fn build_embedding_url(cfg: &EmbeddingConfig, count: usize) -> String {
    let mut path = cfg.embedding_path.replace("{model}", &cfg.model);

    if cfg.format == ApiFormat::GoogleGenerativeAi && count > 1 {
        path = path.replace(":embedContent", ":batchEmbedContents");
    }

    let mut url = format!("{}{}", cfg.base_url, path);

    if cfg.format == ApiFormat::GoogleGenerativeAi && !cfg.api_key_env.is_empty() {
        let key = std::env::var(&cfg.api_key_env).unwrap_or_default();
        url = format!("{url}?key={key}");
    }

    if cfg.auth_header.starts_with("query:") {
        let param = &cfg.auth_header["query:".len()..];
        let key = std::env::var(&cfg.api_key_env).unwrap_or_default();
        let sep = if url.contains('?') { '&' } else { '?' };
        url = format!("{url}{sep}{param}={key}");
    }

    url
}

fn build_embedding_request(cfg: &EmbeddingConfig, texts: &[&str]) -> serde_json::Value {
    match cfg.format {
        ApiFormat::GoogleGenerativeAi => {
            // Google uses a different request shape per-text (we batch by calling once)
            if texts.len() == 1 {
                json!({
                    "model": format!("models/{}", cfg.model),
                    "content": { "parts": [{ "text": texts[0] }] }
                })
            } else {
                let requests: Vec<serde_json::Value> = texts
                    .iter()
                    .map(|t| {
                        json!({
                            "model": format!("models/{}", cfg.model),
                            "content": { "parts": [{ "text": t }] }
                        })
                    })
                    .collect();
                json!({ "requests": requests })
            }
        }
        _ => {
            // OpenAI-compatible (also used by Ollama)
            json!({
                "model": cfg.model,
                "input": texts,
            })
        }
    }
}

fn parse_embedding_response(
    cfg: &EmbeddingConfig,
    resp: &serde_json::Value,
    expected_count: usize,
) -> Result<Vec<Vec<f32>>> {
    match cfg.format {
        ApiFormat::GoogleGenerativeAi => {
            // Single response: { "embedding": { "values": [...] } }
            if let Some(values) = resp.pointer("/embedding/values").and_then(|v| v.as_array()) {
                let emb = parse_f32_array(values);
                return Ok(vec![emb]);
            }
            // Batch response: { "embeddings": [{ "values": [...] }, ...] }
            if let Some(embeddings) = resp.get("embeddings").and_then(|v| v.as_array()) {
                let result: Vec<Vec<f32>> = embeddings
                    .iter()
                    .filter_map(|e| {
                        e.get("values")
                            .and_then(|v| v.as_array())
                            .map(|a| parse_f32_array(a))
                    })
                    .collect();
                if result.len() == expected_count {
                    return Ok(result);
                }
            }
            Err(IroncladError::Llm(
                "failed to parse Google embedding response".into(),
            ))
        }
        _ => {
            // OpenAI-compatible: { "data": [{ "embedding": [...] }, ...] }
            if let Some(data) = resp.get("data").and_then(|v| v.as_array()) {
                let result: Vec<Vec<f32>> = data
                    .iter()
                    .filter_map(|d| {
                        d.get("embedding")
                            .and_then(|v| v.as_array())
                            .map(|a| parse_f32_array(a))
                    })
                    .collect();
                if result.len() == expected_count {
                    return Ok(result);
                }
            }
            // Ollama alternative: { "embeddings": [[...], ...] }
            if let Some(embeddings) = resp.get("embeddings").and_then(|v| v.as_array()) {
                let result: Vec<Vec<f32>> = embeddings
                    .iter()
                    .filter_map(|e| e.as_array().map(|a| parse_f32_array(a)))
                    .collect();
                if result.len() == expected_count {
                    return Ok(result);
                }
            }
            Err(IroncladError::Llm(
                "failed to parse embedding response".into(),
            ))
        }
    }
}

fn parse_f32_array(arr: &[serde_json::Value]) -> Vec<f32> {
    arr.iter()
        .map(|v| v.as_f64().unwrap_or(0.0) as f32)
        .collect()
}

/// Character 3-gram embedding into a fixed-size vector with L2 normalization.
/// Used as a zero-dependency fallback when no embedding provider is configured.
pub fn fallback_ngram(text: &str, dim: usize) -> Vec<f32> {
    let mut vec = vec![0.0f32; dim];
    let lower = text.to_lowercase();
    let chars: Vec<char> = lower.chars().collect();
    if chars.len() < 3 {
        return vec;
    }
    for window in chars.windows(3) {
        let hash = window
            .iter()
            .fold(0u32, |acc, &c| acc.wrapping_mul(31).wrapping_add(c as u32));
        vec[(hash as usize) % dim] += 1.0;
    }
    let norm = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for v in &mut vec {
            *v /= norm;
        }
    }
    vec
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fallback_ngram_deterministic() {
        let a = fallback_ngram("hello world", 128);
        let b = fallback_ngram("hello world", 128);
        assert_eq!(a, b);
    }

    #[test]
    fn fallback_ngram_unit_normalized() {
        let emb = fallback_ngram("test embedding normalization", 128);
        let norm: f32 = emb.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 0.01);
    }

    #[test]
    fn fallback_ngram_short_text() {
        let emb = fallback_ngram("ab", 128);
        assert!(emb.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn fallback_ngram_empty() {
        let emb = fallback_ngram("", 64);
        assert_eq!(emb.len(), 64);
        assert!(emb.iter().all(|&v| v == 0.0));
    }

    #[test]
    fn fallback_ngram_different_dims() {
        let emb64 = fallback_ngram("test", 64);
        let emb256 = fallback_ngram("test", 256);
        assert_eq!(emb64.len(), 64);
        assert_eq!(emb256.len(), 256);
    }

    #[test]
    fn client_without_provider() {
        let client = EmbeddingClient::new(None).unwrap();
        assert!(!client.has_provider());
        assert_eq!(client.dimensions(), NGRAM_DIM);
    }

    #[test]
    fn client_with_provider() {
        let cfg = EmbeddingConfig {
            base_url: "http://localhost:11434".into(),
            embedding_path: "/api/embed".into(),
            model: "nomic-embed-text".into(),
            dimensions: 768,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: "OLLAMA_API_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let client = EmbeddingClient::new(Some(cfg)).unwrap();
        assert!(client.has_provider());
        assert_eq!(client.dimensions(), 768);
    }

    #[tokio::test]
    async fn embed_without_provider_uses_ngram() {
        let client = EmbeddingClient::new(None).unwrap();
        let results = client.embed(&["hello world", "goodbye"]).await.unwrap();
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].len(), NGRAM_DIM);
        assert_eq!(results[1].len(), NGRAM_DIM);
    }

    #[tokio::test]
    async fn embed_single_without_provider() {
        let client = EmbeddingClient::new(None).unwrap();
        let emb = client.embed_single("test input").await.unwrap();
        assert_eq!(emb.len(), NGRAM_DIM);
    }

    #[test]
    fn build_openai_request() {
        let cfg = EmbeddingConfig {
            base_url: "https://api.openai.com".into(),
            embedding_path: "/v1/embeddings".into(),
            model: "text-embedding-3-small".into(),
            dimensions: 1536,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: "OPENAI_API_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let body = build_embedding_request(&cfg, &["hello", "world"]);
        assert_eq!(body["model"], "text-embedding-3-small");
        assert!(body["input"].is_array());
        assert_eq!(body["input"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn build_google_request_single() {
        let cfg = EmbeddingConfig {
            base_url: "https://generativelanguage.googleapis.com".into(),
            embedding_path: "/v1beta/models/{model}:embedContent".into(),
            model: "text-embedding-004".into(),
            dimensions: 768,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: "GOOGLE_API_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let body = build_embedding_request(&cfg, &["hello"]);
        assert!(body.get("content").is_some());
    }

    #[test]
    fn build_google_request_batch() {
        let cfg = EmbeddingConfig {
            base_url: "https://generativelanguage.googleapis.com".into(),
            embedding_path: "/v1beta/models/{model}:embedContent".into(),
            model: "text-embedding-004".into(),
            dimensions: 768,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: "GOOGLE_API_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let body = build_embedding_request(&cfg, &["hello", "world"]);
        assert!(body.get("requests").is_some());
        assert_eq!(body["requests"].as_array().unwrap().len(), 2);
    }

    #[test]
    fn build_embedding_url_openai() {
        let cfg = EmbeddingConfig {
            base_url: "https://api.openai.com".into(),
            embedding_path: "/v1/embeddings".into(),
            model: "text-embedding-3-small".into(),
            dimensions: 1536,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: "OPENAI_API_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let url = build_embedding_url(&cfg, 1);
        assert_eq!(url, "https://api.openai.com/v1/embeddings");
    }

    #[test]
    fn build_embedding_url_google_substitutes_model() {
        unsafe { std::env::set_var("TEST_GOOGLE_KEY", "fake-key") };
        let cfg = EmbeddingConfig {
            base_url: "https://generativelanguage.googleapis.com".into(),
            embedding_path: "/v1beta/models/{model}:embedContent".into(),
            model: "text-embedding-004".into(),
            dimensions: 768,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: "TEST_GOOGLE_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let url = build_embedding_url(&cfg, 1);
        assert!(url.contains("text-embedding-004"));
        assert!(url.contains("key=fake-key"));
        assert!(url.contains(":embedContent"));
        unsafe { std::env::remove_var("TEST_GOOGLE_KEY") };
    }

    #[test]
    fn build_embedding_url_google_batch_uses_batch_endpoint() {
        unsafe { std::env::set_var("TEST_GOOGLE_BATCH_KEY", "fake-key") };
        let cfg = EmbeddingConfig {
            base_url: "https://generativelanguage.googleapis.com".into(),
            embedding_path: "/v1beta/models/{model}:embedContent".into(),
            model: "text-embedding-004".into(),
            dimensions: 768,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: "TEST_GOOGLE_BATCH_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let url = build_embedding_url(&cfg, 3);
        assert!(url.contains(":batchEmbedContents"));
        assert!(!url.contains(":embedContent"));
        unsafe { std::env::remove_var("TEST_GOOGLE_BATCH_KEY") };
    }

    #[test]
    fn parse_openai_response() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 3,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "data": [
                { "embedding": [0.1, 0.2, 0.3], "index": 0 },
                { "embedding": [0.4, 0.5, 0.6], "index": 1 }
            ]
        });
        let result = parse_embedding_response(&cfg, &resp, 2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
        assert_eq!(result[1], vec![0.4, 0.5, 0.6]);
    }

    #[test]
    fn parse_ollama_response() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 3,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "embeddings": [
                [0.1, 0.2, 0.3],
                [0.4, 0.5, 0.6]
            ]
        });
        let result = parse_embedding_response(&cfg, &resp, 2).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn parse_google_single_response() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 3,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "embedding": { "values": [0.1, 0.2, 0.3] }
        });
        let result = parse_embedding_response(&cfg, &resp, 1).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], vec![0.1, 0.2, 0.3]);
    }

    #[test]
    fn parse_google_batch_response() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 3,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "embeddings": [
                { "values": [0.1, 0.2, 0.3] },
                { "values": [0.4, 0.5, 0.6] }
            ]
        });
        let result = parse_embedding_response(&cfg, &resp, 2).unwrap();
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn parse_unparseable_returns_error() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 64,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({ "unexpected": "format" });
        let result = parse_embedding_response(&cfg, &resp, 2);
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn embed_with_unreachable_provider_falls_back() {
        let cfg = EmbeddingConfig {
            base_url: "http://127.0.0.1:1".into(),
            embedding_path: "/v1/embeddings".into(),
            model: "test".into(),
            dimensions: 64,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: "NONEXISTENT_KEY".into(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let client = EmbeddingClient::new(Some(cfg)).unwrap();
        let err = client.embed(&["test"]).await;
        assert!(err.is_err());
    }

    #[test]
    fn parse_google_unparseable_returns_error() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 64,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({ "unexpected": "garbage" });
        let result = parse_embedding_response(&cfg, &resp, 3);
        assert!(result.is_err());
    }

    #[test]
    fn parse_google_mismatched_count_returns_error() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 32,
            format: ApiFormat::GoogleGenerativeAi,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "embeddings": [
                { "values": [0.1, 0.2] }
            ]
        });
        let result = parse_embedding_response(&cfg, &resp, 3);
        assert!(result.is_err());
    }

    #[test]
    fn build_embedding_url_non_google() {
        let cfg = EmbeddingConfig {
            base_url: "http://localhost:11434".into(),
            embedding_path: "/api/embed".into(),
            model: "nomic-embed-text".into(),
            dimensions: 768,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: String::new(),
            auth_header: "Authorization".into(),
            extra_headers: HashMap::new(),
        };
        let url = build_embedding_url(&cfg, 1);
        assert_eq!(url, "http://localhost:11434/api/embed");
    }

    #[test]
    fn build_embedding_url_query_auth() {
        unsafe { std::env::set_var("TEST_QUERY_KEY", "my-secret") };
        let cfg = EmbeddingConfig {
            base_url: "https://api.example.com".into(),
            embedding_path: "/v1/embeddings".into(),
            model: "test-model".into(),
            dimensions: 768,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: "TEST_QUERY_KEY".into(),
            auth_header: "query:api_key".into(),
            extra_headers: HashMap::new(),
        };
        let url = build_embedding_url(&cfg, 1);
        assert!(url.contains("api_key=my-secret"));
        unsafe { std::env::remove_var("TEST_QUERY_KEY") };
    }

    #[test]
    fn parse_f32_array_handles_non_numbers() {
        let arr = vec![
            serde_json::json!(1.5),
            serde_json::json!("not a number"),
            serde_json::json!(null),
            serde_json::json!(3.0),
        ];
        let result = parse_f32_array(&arr);
        assert_eq!(result, vec![1.5, 0.0, 0.0, 3.0]);
    }

    #[test]
    fn parse_openai_mismatched_count_returns_error() {
        let cfg = EmbeddingConfig {
            base_url: String::new(),
            embedding_path: String::new(),
            model: String::new(),
            dimensions: 16,
            format: ApiFormat::OpenAiCompletions,
            api_key_env: String::new(),
            auth_header: String::new(),
            extra_headers: HashMap::new(),
        };
        let resp = json!({
            "data": [
                { "embedding": [0.1, 0.2] }
            ]
        });
        let result = parse_embedding_response(&cfg, &resp, 3);
        assert!(result.is_err());
    }
}
