# ironclad-llm

LLM client pipeline with circuit breaker, ML model router, 3-level semantic cache (with SQLite persistence), multi-format API translation, and multi-provider embedding client (OpenAI/Ollama/Google with n-gram fallback) for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime.

## Usage

```toml
[dependencies]
ironclad-llm = "0.1"
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-llm).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
