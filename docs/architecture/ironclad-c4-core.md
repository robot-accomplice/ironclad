<!-- last_updated: 2026-02-23, version: 0.5.0 -->
# C4 Level 3: Component Diagram -- ironclad-core

*Leaf crate with zero internal dependencies. Provides shared types, configuration parsing, and error definitions used by every other crate.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladCore ["ironclad-core"]
        CONFIG["config.rs<br/>Unified Configuration"]
        ERROR["error.rs<br/>Error Types (thiserror)"]
        TYPES["types.rs<br/>Shared Domain Types"]
        PERSONALITY["personality.rs<br/>OS/Soul/Firmware"]
        STYLE["style.rs<br/>Theme, CRT, Typewriter"]
        KEYSTORE["keystore.rs<br/>Encrypted Credential<br/>Storage"]
    end

    subgraph ConfigDetail ["config.rs internals"]
        TOML_PARSE["parse ironclad.toml<br/>(toml crate, serde)"]

        subgraph CfgInfra["Infrastructure"]
            direction LR
            AGENT_CFG["AgentConfig<br/>name, id, workspace"]
            SERVER_CFG["ServerConfig<br/>port, bind"]
            DB_CFG["DatabaseConfig<br/>path"]
            CHANNELS_CFG["ChannelsConfig<br/>telegram, whatsapp"]
        end

        subgraph CfgAI["AI Pipeline"]
            direction LR
            MODELS_CFG["ModelsConfig<br/>primary, fallbacks,<br/>RoutingConfig"]
            PROVIDERS_CFG["ProvidersConfig<br/>HashMap of ProviderConfig"]
            CB_CFG["CircuitBreakerConfig<br/>threshold, windows"]
            MEMORY_CFG["MemoryConfig<br/>5× budget pct"]
            CACHE_CFG["CacheConfig<br/>TTL, threshold, max"]
        end

        subgraph CfgFinancial["Financial"]
            direction LR
            TREASURY_CFG["TreasuryConfig<br/>caps, limits, reserve"]
            YIELD_CFG["YieldConfig<br/>protocol, min_deposit"]
            WALLET_CFG["WalletConfig<br/>path, chain_id, rpc_url"]
        end

        subgraph CfgExtensions["Extensions"]
            direction LR
            A2A_CFG["A2aConfig<br/>max_message_size,<br/>rate_limit_per_peer"]
            SKILLS_CFG["SkillsConfig<br/>skills_dir, interpreters,<br/>sandbox, hot_reload"]
        end
    end

    subgraph TypesDetail ["types.rs enums"]
        subgraph CoreTypes["Core"]
            direction LR
            SURVIVAL["SurvivalTier"]
            AGENT_STATE["AgentState"]
            API_FMT["ApiFormat (4 variants)"]
            MODEL_TIER["ModelTier T1–T4"]
        end
        subgraph PolicyTypes["Policy & Security"]
            direction LR
            POLICY_DEC["PolicyDecision"]
            RISK["RiskLevel"]
            INPUT_AUTH["InputAuthority"]
        end
        subgraph SkillTypes["Skills"]
            direction LR
            SKILL_KIND["SkillKind"]
            SKILL_TRIGGER["SkillTrigger"]
            SKILL_MANIFEST["SkillManifest"]
            INSTRUCTION_SKILL["InstructionSkill"]
            TOOL_CHAIN_STEP["ToolChainStep"]
        end
        SCHED_KIND["ScheduleKind<br/>Cron, Every, At"]
    end

    subgraph ErrorDetail ["error.rs"]
        IRONCLAD_ERR["IroncladError (thiserror)<br/>13 variants: Config, Channel,<br/>Database, Llm, Network, Policy,<br/>Tool, Wallet, Injection,<br/>Schedule, A2a, Io, Skill"]
    end

    subgraph KeystoreDetail ["keystore.rs — Encrypted Key-Value Storage"]
        KS_STORE["Keystore:<br/>encrypted JSON file on disk,<br/>machine-key auto-unlock"]
        KS_OPS["get(key), set(key, value),<br/>delete(key), list_keys()"]
        KS_UNLOCK["unlock_machine():<br/>derive key from OS<br/>machine identity"]
        KS_PATH["default_path():<br/>~/.ironclad/keystore.json"]
    end

    CONFIG --> TOML_PARSE
    TOML_PARSE --> CfgInfra
    TOML_PARSE --> CfgAI
    TOML_PARSE --> CfgFinancial
    TOML_PARSE --> CfgExtensions
    KEYSTORE --> KS_STORE
```

## Module Responsibilities

| Module | Responsibility | Key Types |
|--------|---------------|-----------|
| `config.rs` | Parse `ironclad.toml` into strongly-typed config structs. **Tilde expansion** applied to `database.path`, `agent.workspace`, `server.log_dir`, `skills.skills_dir`, `wallet.path`, `plugins.dir`, `browser.profile_dir`, `daemon.pid_file`. Validates at load (e.g., memory budget percentages sum to 100, `treasury.per_payment_cap` > 0). | `IroncladConfig`, `AgentConfig`, `ModelsConfig`, `RoutingConfig` (default `mode = "heuristic"`), `TreasuryConfig`, `A2aConfig`, `SkillsConfig`, etc. |
| `types.rs` | Domain enums and structs shared across crates. All enums are exhaustive — adding a variant is a compile-time breaking change. `SurvivalTier::from_balance(usd, hours_below_zero)` derives tier from balance. | `SurvivalTier`, `AgentState`, `ApiFormat`, `ModelTier`, `PolicyDecision`, `RiskLevel`, `SkillKind`, `SkillTrigger`, `SkillManifest`, `ToolChainStep`, `InstructionSkill`, `InputAuthority`, `ScheduleKind` |
| `error.rs` | Unified error type with `thiserror` derive. Each variant wraps crate-specific errors so the top-level binary can handle them uniformly. | `IroncladError` |
| `personality.rs` | Load OS/soul/firmware/operator/directives from workspace; compose identity and firmware text. | `load_os`, `load_firmware`, `compose_identity_text` |
| `style.rs` | Theme (CRT green/orange, terminal), typewriter effect, icons. | `Theme`, `sleep_ms`, `typewrite` |
| `keystore.rs` | Encrypted key-value store for API keys and secrets. Machine-key auto-unlock, JSON file on disk. | `Keystore` |

## Dependencies

**External crates**: `serde`, `toml`, `thiserror`

**Internal crates**: None (leaf node in dependency graph)

**Depended on by**: All 10 other crates
