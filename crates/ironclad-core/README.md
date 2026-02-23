# ironclad-core

> **Version 0.5.0**

Shared types, configuration parsing, encrypted credential storage, personality system, and error types for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime.

This is the **leaf crate** in the dependency graph -- every other Ironclad crate depends on it, and it depends on no internal crates.

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `IroncladConfig` | `config` | Top-level configuration parsed from `ironclad.toml` |
| `IroncladError` / `Result` | `error` | Unified error type (13 variants) used across all crates |
| `Keystore` | `keystore` | Encrypted key-value store for API keys and secrets |
| `SurvivalTier` | `types` | Financial health tier (T1--T4) derived from balance |
| `AgentState` | `types` | Agent lifecycle state |
| `ApiFormat` | `types` | LLM provider API format (OpenAI, Ollama, Google, Anthropic) |
| `ModelTier` | `types` | Model capability tier |
| `PolicyDecision` | `types` | Allow / Deny / Escalate |
| `RiskLevel` | `types` | Tool risk classification |
| `SkillManifest` | `types` | Skill metadata and trigger configuration |
| `Theme` | `style` | Terminal theme (CRT, orange, green) |

## Usage

```toml
[dependencies]
ironclad-core = "0.5"
```

```rust
use ironclad_core::{IroncladConfig, IroncladError, Result, Keystore};

// Load configuration
let config = IroncladConfig::from_file("ironclad.toml")?;

// Access encrypted credentials
let ks = Keystore::new(Keystore::default_path());
if let Some(key) = ks.get("openai_api_key") {
    println!("Key loaded from keystore");
}
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-core).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
