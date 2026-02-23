# ironclad-llm

> **Version 0.5.0**

LLM client pipeline for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Features a multi-stage request pipeline with semantic caching, heuristic + ML model routing, circuit breaker, tiered inference with confidence-based escalation, cascade optimization, prompt compression, OAuth token management, capacity tracking, and multi-provider embedding support.

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `LlmService` | `lib` | Top-level facade wiring cache, router, breakers, client, and embeddings |
| `SemanticCache` | `cache` | 3-level cache (exact hash, semantic cosine, tool TTL) with SQLite persistence |
| `ModelRouter` | `router` | Heuristic complexity classification and model selection |
| `LogisticBackend` | `ml_router` | ML-based routing with preference learning |
| `CircuitBreakerRegistry` | `circuit` | Per-provider circuit breaker (Closed/Open/HalfOpen) |
| `LlmClient` | `client` | HTTP/2 client pool (reqwest) with streaming support |
| `EmbeddingClient` | `embedding` | Multi-provider embeddings (OpenAI, Ollama, Google) with n-gram fallback |
| `ProviderRegistry` | `provider` | Provider definitions and lookup |
| `CascadeOptimizer` | `cascade` | Cheapest-first model cascade with fallback chains |
| `CapacityTracker` | `capacity` | TPM/RPM sliding-window rate tracking |
| `QualityTracker` | `accuracy` | Per-model EMA quality scoring |
| `PromptCompressor` | `compression` | Structural dedup and token estimation |
| `OAuthManager` | `oauth` | OAuth2 token refresh for provider authentication |
| `SseChunkStream` | `lib` | SSE-to-`StreamChunk` adapter for streaming responses |

## Usage

```toml
[dependencies]
ironclad-llm = "0.5"
```

```rust
use ironclad_llm::LlmService;
use ironclad_core::IroncladConfig;

let config = IroncladConfig::from_file("ironclad.toml")?;
let service = LlmService::new(&config)?;

// Use service.client, service.cache, service.router, etc.
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-llm).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
