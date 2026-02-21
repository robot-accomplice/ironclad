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
    end

    subgraph ConfigDetail ["config.rs internals"]
        TOML_PARSE["parse ironclad.toml<br/>(toml crate, serde)"]
        AGENT_CFG["AgentConfig<br/>name, id, workspace, log_level"]
        SERVER_CFG["ServerConfig<br/>port, bind"]
        DB_CFG["DatabaseConfig<br/>path"]
        MODELS_CFG["ModelsConfig<br/>primary, fallbacks,<br/>RoutingConfig (mode, threshold, local_first)"]
        PROVIDERS_CFG["ProvidersConfig<br/>HashMap of ProviderConfig<br/>(url, tier)"]
        CB_CFG["CircuitBreakerConfig<br/>threshold, windows, cooldowns"]
        MEMORY_CFG["MemoryConfig<br/>5x budget percentages"]
        CACHE_CFG["CacheConfig<br/>enabled, TTL, threshold, max_entries"]
        TREASURY_CFG["TreasuryConfig<br/>caps, limits, reserve, budget"]
        YIELD_CFG["YieldConfig<br/>enabled, protocol, chain,<br/>min_deposit, withdrawal_threshold"]
        WALLET_CFG["WalletConfig<br/>path, chain_id, rpc_url"]
        A2A_CFG["A2aConfig<br/>enabled, max_message_size,<br/>rate_limit_per_peer,<br/>session_timeout_seconds,<br/>require_on_chain_identity"]
        CHANNELS_CFG["ChannelsConfig<br/>telegram (enabled, token_env),<br/>whatsapp (enabled)"]
        SKILLS_CFG["SkillsConfig<br/>skills_dir, script_timeout_seconds,<br/>script_max_output_bytes,<br/>allowed_interpreters,<br/>sandbox_env, hot_reload"]
    end

    subgraph TypesDetail ["types.rs enums"]
        SURVIVAL["SurvivalTier<br/>High, Normal, LowCompute,<br/>Critical, Dead"]
        AGENT_STATE["AgentState<br/>Setup, Waking, Running,<br/>Sleeping, Dead"]
        API_FMT["ApiFormat<br/>AnthropicMessages,<br/>OpenAiCompletions,<br/>OpenAiResponses,<br/>GoogleGenerativeAi"]
        MODEL_TIER["ModelTier<br/>T1, T2, T3, T4"]
        POLICY_DEC["PolicyDecision<br/>Allow,<br/>Deny (rule, reason)"]
        RISK["RiskLevel<br/>Safe, Caution,<br/>Dangerous, Forbidden"]
        SKILL_KIND["SkillKind<br/>Structured, Instruction"]
        SKILL_TRIGGER["SkillTrigger<br/>keywords, tool_names,<br/>regex_patterns"]
        SKILL_MANIFEST["SkillManifest<br/>name, description, kind,<br/>triggers, tool_chain,<br/>policy_overrides, script_path"]
        INSTRUCTION_SKILL["InstructionSkill<br/>name, triggers, priority,<br/>body (markdown)"]
    end

    subgraph ErrorDetail ["error.rs"]
        IRONCLAD_ERR["IroncladError (thiserror)<br/>variants: Config, Database,<br/>Llm, Network, Policy, Tool,<br/>Wallet, Injection, Schedule,<br/>A2a, Io, Skill"]
    end

    CONFIG --> TOML_PARSE
    TOML_PARSE --> AGENT_CFG & SERVER_CFG & DB_CFG & MODELS_CFG & PROVIDERS_CFG & CB_CFG & MEMORY_CFG & CACHE_CFG & TREASURY_CFG & YIELD_CFG & WALLET_CFG & A2A_CFG & CHANNELS_CFG & SKILLS_CFG
```

## Module Responsibilities

| Module | Responsibility | Key Types |
|--------|---------------|-----------|
| `config.rs` | Parse `ironclad.toml` into strongly-typed config structs. Validates at load time (e.g., budget percentages sum to 100, chain_id is valid). | `IroncladConfig`, `AgentConfig`, `ModelsConfig`, `TreasuryConfig`, `A2aConfig`, `SkillsConfig`, etc. |
| `types.rs` | Domain enums and structs shared across crates. All enums are exhaustive -- adding a variant is a compile-time breaking change that forces all consumers to handle it. | `SurvivalTier`, `AgentState`, `ApiFormat`, `ModelTier`, `PolicyDecision`, `RiskLevel`, `SkillKind`, `SkillTrigger`, `SkillManifest`, `InstructionSkill` |
| `error.rs` | Unified error type with `thiserror` derive. Each variant wraps crate-specific errors so the top-level binary can handle them uniformly. | `IroncladError` |

## Dependencies

**External crates**: `serde`, `toml`, `thiserror`

**Internal crates**: None (leaf node in dependency graph)

**Depended on by**: All 7 other crates
