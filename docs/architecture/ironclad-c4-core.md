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
    end

    subgraph ConfigDetail ["config.rs internals"]
        TOML_PARSE["parse ironclad.toml<br/>(toml crate, serde)"]
        AGENT_CFG["AgentConfig<br/>name, id, workspace, log_level"]
        SERVER_CFG["ServerConfig<br/>port, bind"]
        DB_CFG["DatabaseConfig<br/>path"]
        MODELS_CFG["ModelsConfig<br/>primary, fallbacks,<br/>RoutingConfig (mode default 'heuristic',<br/>'ml' backward-compat alias)"]
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
        TOOL_CHAIN_STEP["ToolChainStep<br/>tool_name, params"]
        INPUT_AUTH["InputAuthority<br/>Creator, SelfGenerated,<br/>Peer, External"]
        SCHED_KIND["ScheduleKind<br/>Cron, Every, At"]
    end

    subgraph ErrorDetail ["error.rs"]
        IRONCLAD_ERR["IroncladError (thiserror)<br/>variants: Config, Channel, Database,<br/>Llm, Network, Policy, Tool,<br/>Wallet, Injection, Schedule,<br/>A2a, Io, Skill"]
    end

    CONFIG --> TOML_PARSE
    TOML_PARSE --> AGENT_CFG & SERVER_CFG & DB_CFG & MODELS_CFG & PROVIDERS_CFG & CB_CFG & MEMORY_CFG & CACHE_CFG & TREASURY_CFG & YIELD_CFG & WALLET_CFG & A2A_CFG & CHANNELS_CFG & SKILLS_CFG
```

## Module Responsibilities

| Module | Responsibility | Key Types |
|--------|---------------|-----------|
| `config.rs` | Parse `ironclad.toml` into strongly-typed config structs. **Tilde expansion** applied to `database.path`, `agent.workspace`, `server.log_dir`, `skills.skills_dir`, `wallet.path`, `plugins.dir`, `browser.profile_dir`, `daemon.pid_file`. Validates at load (e.g., memory budget percentages sum to 100, `treasury.per_payment_cap` > 0). | `IroncladConfig`, `AgentConfig`, `ModelsConfig`, `RoutingConfig` (default `mode = "heuristic"`), `TreasuryConfig`, `A2aConfig`, `SkillsConfig`, etc. |
| `types.rs` | Domain enums and structs shared across crates. All enums are exhaustive — adding a variant is a compile-time breaking change. `SurvivalTier::from_balance(usd, hours_below_zero)` derives tier from balance. | `SurvivalTier`, `AgentState`, `ApiFormat`, `ModelTier`, `PolicyDecision`, `RiskLevel`, `SkillKind`, `SkillTrigger`, `SkillManifest`, `ToolChainStep`, `InstructionSkill`, `InputAuthority`, `ScheduleKind` |
| `error.rs` | Unified error type with `thiserror` derive. Each variant wraps crate-specific errors so the top-level binary can handle them uniformly. | `IroncladError` |
| `personality.rs` | Load OS/soul/firmware/operator/directives from workspace; compose identity and firmware text. | `load_os`, `load_firmware`, `compose_identity_text` |
| `style.rs` | Theme (CRT green/orange, terminal), typewriter effect, icons. | `Theme`, `sleep_ms`, `typewrite` |

## Dependencies

**External crates**: `serde`, `toml`, `thiserror`

**Internal crates**: None (leaf node in dependency graph)

**Depended on by**: All 10 other crates
