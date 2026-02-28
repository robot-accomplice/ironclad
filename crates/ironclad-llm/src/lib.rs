//! # ironclad-llm
//!
//! LLM client pipeline for the Ironclad agent runtime. Requests flow through a
//! multi-stage pipeline: cache check, routing (heuristic or ML), circuit
//! breaker, dedup, format translation, prompt compression, tier adaptation,
//! and HTTP forwarding.
//!
//! ## Key Types
//!
//! - [`LlmService`] -- Top-level facade composing all pipeline stages
//! - [`SemanticCache`] -- 3-level cache (exact hash, tool TTL, semantic cosine)
//! - [`ModelRouter`] -- Heuristic complexity classification and model selection
//! - [`LlmClient`] -- HTTP/2 client pool with streaming support
//! - [`EmbeddingClient`] -- Multi-provider embedding client with n-gram fallback
//! - [`SseChunkStream`] -- SSE byte stream to parsed `StreamChunk` adapter
//!
//! ## Modules
//!
//! - `cache` -- Semantic cache with HashMap + SQLite persistence
//! - `router` -- Heuristic model router (feature extraction, complexity scoring)
//! - `ml_router` -- Logistic regression backend + preference learning
//! - `uniroute` -- Unified routing via model capability vectors
//! - `tiered` -- Tiered inference with confidence evaluation and escalation
//! - `cascade` -- Cascade optimizer (cheapest-first, fallback chain)
//! - `circuit` -- Per-provider circuit breaker with exponential backoff
//! - `dedup` -- In-flight duplicate request detection
//! - `format` -- API format translation (OpenAI, Ollama, Google, Anthropic)
//! - `compression` -- Prompt compression and token estimation
//! - `tier` -- Tier-based prompt adaptation (T1 strip, T2 preamble, T3/T4 pass)
//! - `client` -- HTTP client pool, request forwarding, cost tracking
//! - `provider` -- Provider definitions and registry
//! - `embedding` -- Multi-provider embedding client
//! - `capacity` -- TPM/RPM sliding-window capacity tracking
//! - `accuracy` -- Per-model quality tracking and quality-target selection
//! - `oauth` -- OAuth2 token management and refresh
//! - `transform` -- Request/response transform pipeline

pub mod accuracy;
pub mod cache;
pub mod capacity;
pub mod cascade;
pub mod circuit;
pub mod client;
pub mod compression;
pub mod dedup;
pub mod embedding;
pub mod format;
pub mod ml_router;
pub mod oauth;
pub mod provider;
pub mod router;
pub mod tier;
pub mod tiered;
pub mod uniroute;

pub use accuracy::{QualityTracker, select_for_quality_target};
pub use cache::{CachedResponse, ExportedCacheEntry, SemanticCache};
pub use capacity::CapacityTracker;
pub use cascade::{CascadeOptimizer, CascadeOutcome, CascadeStrategy};
pub use circuit::{CircuitBreakerRegistry, CircuitState};
pub use client::LlmClient;
pub use compression::{CompressionEstimate, PromptCompressor};
pub use dedup::DedupTracker;
pub use embedding::{EmbeddingClient, EmbeddingConfig};
pub use ml_router::{LogisticBackend, PreferenceCollector, PreferenceRecord};
pub use oauth::OAuthManager;
pub use provider::{Provider, ProviderRegistry};
pub use router::{ModelRouter, classify_complexity, extract_features};
pub use tiered::{ConfidenceEvaluator, EscalationTracker, InferenceTier, TieredResult};
pub use uniroute::{ModelVector, ModelVectorRegistry, QueryRequirements};

pub use format::StreamChunk;

use std::collections::HashMap;
use std::pin::Pin;
use std::task::{Context, Poll};

use bytes::Bytes;
use futures::Stream;
use ironclad_core::{ApiFormat, IroncladConfig, Result};
use router::HeuristicBackend;

pub struct LlmService {
    pub cache: SemanticCache,
    pub breakers: CircuitBreakerRegistry,
    pub dedup: DedupTracker,
    pub router: ModelRouter,
    pub client: LlmClient,
    pub providers: ProviderRegistry,
    pub capacity: CapacityTracker,
    pub embedding: EmbeddingClient,
}

impl LlmService {
    pub fn new(config: &IroncladConfig) -> Result<Self> {
        let cache = SemanticCache::with_threshold(
            config.cache.enabled,
            config.cache.exact_match_ttl_seconds,
            config.cache.max_entries,
            config.cache.semantic_threshold as f32,
        );

        let breakers = CircuitBreakerRegistry::new(&config.circuit_breaker);

        let dedup = DedupTracker::default();

        let routing_config = config.models.routing.clone();

        let router = ModelRouter::new(
            config.models.primary.clone(),
            config.models.fallbacks.clone(),
            routing_config,
            Box::new(HeuristicBackend),
        );

        let client = LlmClient::new()?;

        let providers = ProviderRegistry::from_config(&config.providers);

        let capacity = CapacityTracker::new(60);
        for provider in providers.list() {
            capacity.register(&provider.name, provider.tpm_limit, provider.rpm_limit);
        }

        let embedding_config = Self::resolve_embedding_config(&config.memory, &providers);
        let embedding = EmbeddingClient::new(embedding_config)?;

        Ok(Self {
            cache,
            breakers,
            dedup,
            router,
            client,
            providers,
            capacity,
            embedding,
        })
    }

    /// Stream a request to the given provider, returning parsed `StreamChunk`s.
    ///
    /// The caller is responsible for provider selection and key resolution.
    /// `body` should already be translated via `format::translate_request`.
    /// This method injects `"stream": true` into the body before sending.
    pub async fn stream_to_provider(
        &self,
        url: String,
        api_key: String,
        mut body: serde_json::Value,
        auth_header: String,
        extra_headers: HashMap<String, String>,
        api_format: ApiFormat,
    ) -> Result<SseChunkStream> {
        body["stream"] = serde_json::json!(true);

        let raw_stream = self
            .client
            .forward_stream(&url, &api_key, body, &auth_header, &extra_headers)
            .await?;

        Ok(SseChunkStream::new(raw_stream, api_format))
    }

    fn resolve_embedding_config(
        memory: &ironclad_core::config::MemoryConfig,
        providers: &ProviderRegistry,
    ) -> Option<EmbeddingConfig> {
        let provider_name = memory.embedding_provider.as_deref()?;
        let provider = providers.get(provider_name)?;
        let embedding_path = provider.embedding_path.as_deref()?;

        let model = memory
            .embedding_model
            .clone()
            .or_else(|| provider.embedding_model.clone())?;

        let dimensions = provider.embedding_dimensions.unwrap_or(768);

        Some(EmbeddingConfig {
            base_url: provider.url.clone(),
            embedding_path: embedding_path.to_string(),
            model,
            dimensions,
            format: provider.format,
            api_key_env: provider.api_key_env.clone(),
            auth_header: provider.auth_header.clone(),
            extra_headers: provider.extra_headers.clone(),
        })
    }
}

/// Maximum SSE buffer size (10 MB). Streams exceeding this are terminated to
/// prevent unbounded memory growth from a misbehaving provider.
const MAX_SSE_BUFFER: usize = 10 * 1024 * 1024;

/// A `Stream` adapter that converts raw SSE byte chunks from an LLM provider
/// into parsed `StreamChunk` items. Handles buffering across chunk boundaries
/// with proper incremental UTF-8 decoding.
pub struct SseChunkStream {
    inner: Pin<Box<dyn Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send>>,
    format: ApiFormat,
    /// Validated UTF-8 text ready for line parsing.
    text_buffer: String,
    /// Raw byte buffer holding trailing bytes from an incomplete UTF-8 sequence.
    /// These bytes are prepended to the next incoming chunk before decoding.
    raw_tail: Vec<u8>,
    /// Chunks parsed from the buffer remainder when the inner stream ends.
    /// Drained before returning `None` to avoid dropping trailing data.
    pending: std::collections::VecDeque<format::StreamChunk>,
    inner_done: bool,
}

impl SseChunkStream {
    pub fn new(
        inner: Pin<Box<dyn Stream<Item = std::result::Result<Bytes, reqwest::Error>> + Send>>,
        format: ApiFormat,
    ) -> Self {
        Self {
            inner,
            format,
            text_buffer: String::new(),
            raw_tail: Vec::new(),
            pending: std::collections::VecDeque::new(),
            inner_done: false,
        }
    }
}

impl Stream for SseChunkStream {
    type Item = Result<format::StreamChunk>;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let this = self.get_mut();

        // Drain any chunks buffered from the final flush before signaling end-of-stream
        if let Some(chunk) = this.pending.pop_front() {
            return Poll::Ready(Some(Ok(chunk)));
        }
        if this.inner_done {
            return Poll::Ready(None);
        }

        loop {
            // First, try to parse a complete line from the text buffer
            if let Some(newline_pos) = this.text_buffer.find('\n') {
                let line = this.text_buffer[..newline_pos].trim().to_string();
                this.text_buffer = this.text_buffer[newline_pos + 1..].to_string();

                if line.is_empty() {
                    continue;
                }

                if let Some(chunk) = format::parse_sse_chunk(&line, &this.format) {
                    return Poll::Ready(Some(Ok(chunk)));
                }
                continue;
            }

            // No complete line in buffer -- poll for more bytes
            match Pin::new(&mut this.inner).poll_next(cx) {
                Poll::Ready(Some(Ok(bytes))) => {
                    // Prepend any leftover incomplete UTF-8 bytes from the previous chunk
                    let combined = if this.raw_tail.is_empty() {
                        bytes.to_vec()
                    } else {
                        let mut buf = std::mem::take(&mut this.raw_tail);
                        buf.extend_from_slice(&bytes);
                        buf
                    };

                    // Decode as much valid UTF-8 as possible, keeping any
                    // incomplete trailing sequence for the next chunk.
                    match std::str::from_utf8(&combined) {
                        Ok(valid) => {
                            this.text_buffer.push_str(valid);
                        }
                        Err(e) => {
                            let valid_up_to = e.valid_up_to();
                            // valid_up_to is a confirmed UTF-8 boundary from Utf8Error.
                            let valid = std::str::from_utf8(&combined[..valid_up_to])
                                .expect("valid_up_to guarantees valid UTF-8");
                            this.text_buffer.push_str(valid);
                            this.raw_tail = combined[valid_up_to..].to_vec();
                        }
                    }

                    // Guard against unbounded buffer growth
                    if this.text_buffer.len() + this.raw_tail.len() > MAX_SSE_BUFFER {
                        return Poll::Ready(Some(Err(ironclad_core::IroncladError::Llm(
                            "SSE stream buffer exceeded 10 MB limit".into(),
                        ))));
                    }
                }
                Poll::Ready(Some(Err(e))) => {
                    return Poll::Ready(Some(Err(ironclad_core::IroncladError::Network(format!(
                        "stream error: {e}"
                    )))));
                }
                Poll::Ready(None) => {
                    this.inner_done = true;

                    // Convert any remaining raw tail bytes lossily (stream ended
                    // mid-character, so these are genuinely malformed).
                    if !this.raw_tail.is_empty() {
                        let tail = std::mem::take(&mut this.raw_tail);
                        this.text_buffer.push_str(&String::from_utf8_lossy(&tail));
                    }

                    // Parse ALL remaining lines and queue them for delivery
                    if !this.text_buffer.trim().is_empty() {
                        let remaining = std::mem::take(&mut this.text_buffer);
                        for line in remaining.lines() {
                            let line = line.trim();
                            if line.is_empty() {
                                continue;
                            }
                            if let Some(chunk) = format::parse_sse_chunk(line, &this.format) {
                                this.pending.push_back(chunk);
                            }
                        }
                    }
                    return match this.pending.pop_front() {
                        Some(chunk) => Poll::Ready(Some(Ok(chunk))),
                        None => Poll::Ready(None),
                    };
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn llm_service_construction() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
fallbacks = ["openai/gpt-4o"]

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"

[providers.openai]
url = "https://api.openai.com"
tier = "T3"
"#;
        let config = IroncladConfig::from_str(toml).unwrap();
        let service = LlmService::new(&config).unwrap();

        assert_eq!(service.router.select_model(), "ollama/qwen3:8b");
        assert_eq!(service.cache.size(), 0);
        assert!(service.providers.get("ollama").is_some());
        assert!(service.providers.get("openai").is_some());
        assert!(!service.embedding.has_provider());
    }

    #[test]
    fn llm_service_with_embedding_provider() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[memory]
embedding_provider = "ollama"

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"
embedding_path = "/api/embed"
embedding_model = "nomic-embed-text"
embedding_dimensions = 768
"#;
        let config = IroncladConfig::from_str(toml).unwrap();
        let service = LlmService::new(&config).unwrap();
        assert!(service.embedding.has_provider());
        assert_eq!(service.embedding.dimensions(), 768);
    }

    #[test]
    fn resolve_embedding_config_no_provider() {
        let memory = ironclad_core::config::MemoryConfig::default();
        let providers = ProviderRegistry::new();
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_missing_provider() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("nonexistent".into()),
            ..Default::default()
        };
        let providers = ProviderRegistry::new();
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_provider_no_embedding_path() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("anthropic".into()),
            ..Default::default()
        };
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert(
            "anthropic".to_string(),
            ironclad_core::config::ProviderConfig::new("https://api.anthropic.com", "T3"),
        );
        let providers = ProviderRegistry::from_config(&providers_cfg);
        let result = LlmService::resolve_embedding_config(&memory, &providers);
        assert!(result.is_none());
    }

    #[test]
    fn resolve_embedding_config_uses_memory_model_override() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("openai".into()),
            embedding_model: Some("text-embedding-3-large".into()),
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("https://api.openai.com", "T3");
        cfg.embedding_path = Some("/v1/embeddings".into());
        cfg.embedding_model = Some("text-embedding-3-small".into());
        cfg.embedding_dimensions = Some(1536);
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("openai".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.model, "text-embedding-3-large");
        assert_eq!(result.dimensions, 1536);
    }

    #[test]
    fn resolve_embedding_config_falls_back_to_provider_model() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("ollama".into()),
            embedding_model: None,
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("http://localhost:11434", "T1");
        cfg.embedding_path = Some("/api/embed".into());
        cfg.embedding_model = Some("nomic-embed-text".into());
        cfg.embedding_dimensions = Some(768);
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("ollama".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.model, "nomic-embed-text");
        assert_eq!(result.dimensions, 768);
        assert_eq!(result.base_url, "http://localhost:11434");
        assert_eq!(result.embedding_path, "/api/embed");
    }

    // ── SseChunkStream tests ──────────────────────────────────

    use futures::stream;

    /// Helper: drive an `SseChunkStream` to completion and collect all chunks.
    fn collect_sse_chunks(data: Vec<Vec<u8>>) -> Vec<format::StreamChunk> {
        let byte_stream = stream::iter(
            data.into_iter()
                .map(|b| Ok::<_, reqwest::Error>(Bytes::from(b))),
        );
        let mut sse = SseChunkStream::new(Box::pin(byte_stream), ApiFormat::OpenAiCompletions);

        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut chunks = vec![];
            while let Some(item) = futures::StreamExt::next(&mut sse).await {
                chunks.push(item.unwrap());
            }
            chunks
        })
    }

    #[test]
    fn sse_chunk_stream_multiple_trailing_chunks() {
        let data = vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\n".to_vec(),
            b"data: {\"choices\":[{\"delta\":{\"content\":\"C\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"D\"}}]}".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        let text: String = chunks.iter().map(|c| c.delta.as_str()).collect();
        assert_eq!(text, "ABCD", "all four chunks should be yielded");
    }

    #[test]
    fn sse_chunk_stream_trailing_done_not_lost() {
        let data = vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\ndata: [DONE]".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "hello");
    }

    #[test]
    fn sse_chunk_stream_empty_buffer_at_end() {
        let data = vec![b"data: {\"choices\":[{\"delta\":{\"content\":\"only\"}}]}\n".to_vec()];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "only");
    }

    #[test]
    fn resolve_embedding_config_default_dimensions() {
        let memory = ironclad_core::config::MemoryConfig {
            embedding_provider: Some("custom".into()),
            embedding_model: Some("my-model".into()),
            ..Default::default()
        };
        let mut cfg = ironclad_core::config::ProviderConfig::new("http://localhost:8080", "T1");
        cfg.embedding_path = Some("/embed".into());
        cfg.embedding_model = Some("my-model".into());
        // No dimensions set — should default to 768
        let mut providers_cfg = std::collections::HashMap::new();
        providers_cfg.insert("custom".to_string(), cfg);
        let providers = ProviderRegistry::from_config(&providers_cfg);

        let result = LlmService::resolve_embedding_config(&memory, &providers).unwrap();
        assert_eq!(result.dimensions, 768);
    }

    // ── SseChunkStream additional edge cases ──────────────────────

    #[test]
    fn sse_chunk_stream_empty_input() {
        let chunks = collect_sse_chunks(vec![]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn sse_chunk_stream_empty_bytes() {
        let chunks = collect_sse_chunks(vec![b"".to_vec()]);
        assert!(chunks.is_empty());
    }

    #[test]
    fn sse_chunk_stream_only_whitespace_lines() {
        let data = vec![b"\n\n\n".to_vec()];
        let chunks = collect_sse_chunks(data);
        assert!(chunks.is_empty());
    }

    #[test]
    fn sse_chunk_stream_non_data_lines_skipped() {
        let data = vec![
            b"event: message\nid: 123\ndata: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n"
                .to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "ok");
    }

    #[test]
    fn sse_chunk_stream_split_across_boundaries() {
        // Split a single SSE line across two byte chunks
        let data = vec![
            b"data: {\"choices\":[{\"del".to_vec(),
            b"ta\":{\"content\":\"split\"}}]}\n".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "split");
    }

    #[test]
    fn sse_chunk_stream_split_utf8_boundary() {
        // Multi-byte UTF-8 char split across chunk boundary
        // "Hello\xC3" in chunk 1, "\xA9world" in chunk 2 (copyright sign = 0xC3 0xA9)
        let data = vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"Hello\xC3".to_vec(),
            b"\xA9world\"}}]}\n".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        // The content should contain the copyright symbol
        assert!(chunks[0].delta.contains("Hello"));
        assert!(chunks[0].delta.contains("world"));
    }

    #[test]
    fn sse_chunk_stream_multiple_lines_in_one_chunk() {
        let data = vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"A\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"B\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"C\"}}]}\n".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 3);
        assert_eq!(chunks[0].delta, "A");
        assert_eq!(chunks[1].delta, "B");
        assert_eq!(chunks[2].delta, "C");
    }

    /// Helper: drive an SseChunkStream and collect all items (including errors).
    fn collect_sse_results(data: Vec<Vec<u8>>) -> Vec<Result<format::StreamChunk>> {
        let byte_stream = stream::iter(
            data.into_iter()
                .map(|b| Ok::<_, reqwest::Error>(Bytes::from(b))),
        );
        let mut sse = SseChunkStream::new(Box::pin(byte_stream), ApiFormat::OpenAiCompletions);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        rt.block_on(async {
            let mut items = vec![];
            while let Some(item) = futures::StreamExt::next(&mut sse).await {
                items.push(item);
            }
            items
        })
    }

    #[test]
    fn sse_chunk_stream_buffer_overflow_error() {
        // Create a chunk large enough to exceed the 10 MB limit
        let huge = vec![b'x'; 11 * 1024 * 1024];
        let results = collect_sse_results(vec![huge]);
        let last = results.last().unwrap();
        assert!(last.is_err());
        let err_msg = format!("{}", last.as_ref().unwrap_err());
        assert!(
            err_msg.contains("10 MB"),
            "error should mention buffer limit: {err_msg}"
        );
    }

    #[test]
    fn sse_chunk_stream_anthropic_format() {
        // Test with Anthropic format
        let data = vec![
            b"data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"Hi\"}}\n".to_vec(),
        ];
        let byte_stream = stream::iter(
            data.into_iter()
                .map(|b| Ok::<_, reqwest::Error>(Bytes::from(b))),
        );
        let mut sse = SseChunkStream::new(Box::pin(byte_stream), ApiFormat::AnthropicMessages);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let chunks: Vec<_> = rt.block_on(async {
            let mut chunks = vec![];
            while let Some(item) = futures::StreamExt::next(&mut sse).await {
                chunks.push(item.unwrap());
            }
            chunks
        });
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "Hi");
    }

    #[test]
    fn sse_chunk_stream_google_format() {
        let data = vec![
            b"data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Gemini\"}],\"role\":\"model\"}}]}\n".to_vec(),
        ];
        let byte_stream = stream::iter(
            data.into_iter()
                .map(|b| Ok::<_, reqwest::Error>(Bytes::from(b))),
        );
        let mut sse = SseChunkStream::new(Box::pin(byte_stream), ApiFormat::GoogleGenerativeAi);
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        let chunks: Vec<_> = rt.block_on(async {
            let mut chunks = vec![];
            while let Some(item) = futures::StreamExt::next(&mut sse).await {
                chunks.push(item.unwrap());
            }
            chunks
        });
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "Gemini");
    }

    #[test]
    fn sse_chunk_stream_trailing_data_no_newline() {
        // Data that doesn't end with a newline should still be parsed on stream end
        let data = vec![b"data: {\"choices\":[{\"delta\":{\"content\":\"tail\"}}]}".to_vec()];
        let chunks = collect_sse_chunks(data);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0].delta, "tail");
    }

    #[test]
    fn sse_chunk_stream_pending_queue_drains_correctly() {
        // Multiple trailing lines with no final newline
        let data = vec![
            b"data: {\"choices\":[{\"delta\":{\"content\":\"X\"}}]}\ndata: {\"choices\":[{\"delta\":{\"content\":\"Y\"}}]}".to_vec(),
        ];
        let chunks = collect_sse_chunks(data);
        let text: String = chunks.iter().map(|c| c.delta.as_str()).collect();
        assert_eq!(text, "XY");
    }
}
