# Ironclad Ubiquitous Language

> A single source of truth for domain terminology across the Ironclad codebase,
> documentation, and team communication. If a term has multiple names, **one is
> canonical**; the rest are listed as legacy/alias for disambiguation.

---

## How to Use This Document

1. **Writing code** — use the **Canonical Term** in identifiers, doc comments,
   log messages, and error strings.
2. **Writing docs** — use the **Canonical Term** on first mention. You may
   parenthetically note the legacy term if readers may know it by the old name.
3. **Code review** — flag deviations from this glossary as naming violations.
4. **Onboarding** — read this document end-to-end before diving into the code.

---

## Table of Contents

- [Terminology Evolution & Disambiguation](#terminology-evolution--disambiguation)
- [Agent Lifecycle](#agent-lifecycle)
- [OS & Firmware (Agent Identity)](#os--firmware-agent-identity)
- [Memory & Retrieval](#memory--retrieval)
- [Inference Pipeline](#inference-pipeline)
- [Routing & Model Selection](#routing--model-selection)
- [Security & Policy](#security--policy)
- [Channels & Messaging](#channels--messaging)
- [Multimodal & Media](#multimodal--media)
- [Orchestration & Multi-Agent](#orchestration--multi-agent)
- [Skills & Extensibility](#skills--extensibility)
- [Scheduling & Daemons](#scheduling--daemons)
- [Financial & Wallet](#financial--wallet)
- [Browser Automation](#browser-automation)
- [Server & Operations](#server--operations)
- [Persistence & Database](#persistence--database)
- [Architectural Patterns](#architectural-patterns)

---

## Terminology Evolution & Disambiguation

Terms in this section have caused confusion or have evolved over time. Each row
maps **the canonical term** to any legacy/alternate names that may still appear
in older comments, commits, or conversations.

| Canonical Term | Legacy / Alternate Names | Disambiguation |
|---|---|---|
| **Subagent** | specialist, delegate | A taskable child agent. "Specialist" was used pre-v0.8.2 and normalized via role validation. "Delegate" appears in some orchestration literature but is not used in code. |
| **Orchestrator** | commander | The primary agent that manages subagents. "Commander" is historical terminology retained only for disambiguating old discussions and audit history. |
| **Model-proxy** | _(none)_ | A routing/proxy record in the `sub_agents` table. Explicitly **not** a subagent — cannot own skills, cannot be tasked. |
| **OS** | soul, personality, character, persona | The agent's identity layer — the frequently updated surface. Loaded from `OS.toml` as `OsConfig` (containing `OsIdentity` + `OsVoice` + `prompt_text`). Identity tuning, voice adjustments, and personality evolution happen here. "Soul" and "personality" are acceptable as casual descriptors but **OS** is the canonical term in code, config files, and docs. |
| **Firmware** | system prompt, system message, rules | The foundational behavioral rule layer — essentially immutable once defined. Loaded from `FIRMWARE.toml` as `FirmwareConfig`. Composed into the system prompt via `compose_firmware_text()`. Not the same as a raw "system prompt" — firmware is structured rules, the system prompt is the final composed text sent to the LLM. |
| **PersonalityState** | personality | Runtime container struct holding the composed OS + Firmware text. The struct name is legacy; the _concept_ it represents is "the loaded OS and Firmware." Scheduled for rename — see [Pending Renames](#pending-renames-via-mechanic---repair). |
| **Unified Pipeline** | dual path, agent_message path, channel_message path | Since v0.9.0: both API (`agent_message`) and channel (`process_channel_message`) entry points converge into `prepare_inference` → `execute_inference_pipeline` in `core.rs`. The two entry-point function names remain for routing purposes, but the inference path is shared. |
| **Metascore** | quality score, model score, performance score | 5-dimension composite score (efficacy, cost, availability, locality, confidence) used for model selection. Defined in `profile.rs`. Not the same as `QualityTracker` accuracy metrics, which feed _into_ the metascore. |
| **QualityTracker** | accuracy tracker | Per-model accuracy/correctness metrics in `accuracy.rs`. Feeds data into metascore computation. |
| **Soft trim** | context pruning, needs_pruning | Token-budget-aware removal of oldest turns from context. Renamed from `needs_pruning()` → `soft_trim()` in v0.9.0. |
| **Digest** | summary, compaction | Episodic memory summarization via `digest_on_close()`. "Summary" is too generic; "compaction" refers to a different database-level operation. |
| **Hippocampus** | memory consolidation, long-term memory | Named database module + table for agent-created memory structures and decay scheduling. The neuroscience metaphor is intentional and canonical. |
| **Inbound** | incoming, received | Canonical direction for messages arriving from external platforms. All adapters use `parse_inbound()`. |
| **Outbound** | outgoing, sent, reply | Canonical direction for messages leaving the system. `OutboundMessage` is the struct; `send()` is the method. "Reply" is acceptable in conversational context but not in type names. |
| **Poll loop** | listener, watcher | The periodic recv/dispatch cycle in `poll_loops.rs`. "Listener" is acceptable for IMAP (which uses IDLE), but the general pattern is "poll loop." |
| **Governor** | manager, controller, lifecycle manager | `SessionGovernor` manages session expiry, compaction, rotation, and decay. "Manager" and "controller" are avoided to prevent confusion with other `*Manager` types. |
| **Heartbeat** | tick, pulse | The periodic daemon cycle in `heartbeat.rs`. `TickContext` is the per-tick execution context, but the daemon itself is "heartbeat." |
| **Abuse** | threat, violation, breach | Canonical term for malicious/suspicious actor behavior. "Threat" is used only as a modifier (e.g., `threat_downgraded` on `SecurityClaim`). |
| **Budget** | limit, cap, quota | Token/context allocation term. "Limit" is for hard ceilings (e.g., `max_image_size`). "Cap" is for treasury spending. "Budget" is for proportional allocation across tiers. |
| **Capacity** | rate limit, throttle, quota | Provider-level TPM/RPM tracking in `capacity.rs`. Not the same as "budget" (which is about token allocation, not rate limiting). |
| **Mechanic** | admin CLI, operator tools | The administrative CLI subsystem for repair, setup, and diagnostics. "Admin" refers to the HTTP API routes (`/api/admin/*`). "Operator" is the human role. |
| **Mechanic checks** | state hygiene, startup hygiene | Instance-local integrity checks/repairs run by mechanic, startup/update hooks, and periodic maintenance loops. |
| **Workspace** | project directory, vault | The on-disk directory containing agent configs, personality files, skills, and data. "Vault" refers specifically to Obsidian integration. |
| **Interview** | personality generation, onboarding | The multi-turn conversation flow that elicits agent personality (soul, firmware, identity). Not to be confused with "onboarding" (which is broader). |

---

## Agent Lifecycle

| Term | Type | Definition |
|---|---|---|
| **AgentState** | `enum` | Lifecycle state: `Setup → Waking → Running → Sleeping → Dead`. Defined in `ironclad-core/types.rs`. |
| **ReactState** | `enum` | ReAct loop FSM state: `Thinking → Acting → Observing → Persisting → Idle → Done`. Defined in `ironclad-agent/loop.rs`. |
| **ReactAction** | `enum` | FSM transition: `Think`, `Act{tool,params}`, `Observe`, `Persist`, `NoOp`, `Finish`. |
| **AgentLoop** | `struct` | The ReAct state machine. Tracks turn count, max turns, idle count, and recent tool calls for loop detection. |
| **Turn** | concept | A single round of the ReAct loop. Persisted as `TurnRecord` in the database. Not the same as a "message" — one turn may produce multiple messages. |
| **Session** | `struct` | A conversation container. Scoped by `SessionScope`: `agent` (self-talk), `peer` (1:1), or `group` (multi-party). Has status: `active`, `archived`, `expired`. |

---

## OS & Firmware (Agent Identity)

The agent's identity is defined by two workspace files: **`OS.toml`** (who the
agent _is_) and **`FIRMWARE.toml`** (how the agent _behaves_). Together they
compose the system prompt sent to the LLM.

> **Why "OS" and "Firmware"?** The metaphor mirrors real hardware:
> **Firmware** is the foundational behavioral rule layer — like flash ROM, it
> is essentially immutable once defined and rarely needs updating.
> **OS** is the agent's identity layer — like an operating system, it is the
> more frequently updated surface (personality tuning, voice adjustments,
> identity evolution). "Soul" and "personality" are acceptable in casual
> conversation but **must not** be used in new code, config keys, or
> documentation headings.

| Term | Type | Definition |
|---|---|---|
| **OsConfig** | `struct` | The agent's identity layer, loaded from `OS.toml`. Contains `identity` (`OsIdentity`), `voice` (`OsVoice`), and `prompt_text` (freeform identity prose). |
| **OsIdentity** | `struct` | Agent metadata: `name`, `version`, `generated_by`. Displayed in status endpoints and logs. |
| **OsVoice** | `struct` | Communication style: `formality`, `proactiveness`, `domain`, `humor`, `language`. Appended to the composed text as `## Voice Profile` when non-default. |
| **FirmwareConfig** | `struct` | Behavioral rules loaded from `FIRMWARE.toml`. A list of structured rules composed into the system prompt via `compose_firmware_text()`. |
| **PersonalityState** | `struct` | _(Legacy struct name)_ Runtime container holding the composed text. Fields: `soul_text` (composed OS text — legacy name), `firmware_text`, `identity`, `voice`. Loaded via `PersonalityState::from_workspace()`. |
| **compose_soul()** | function | _(Legacy function name)_ Full composition: OS + Firmware + Operator + Directives → system prompt text. Kept for backward compat; internally calls the canonical loaders. |
| **Interview** | workflow | Multi-turn conversation flow that elicits OS and Firmware configuration. State tracked in `InterviewSession`. Produces `OS.toml` + `FIRMWARE.toml` artifacts. |

### Workspace Files

| File | Canonical Term | Loaded By | Contains |
|---|---|---|---|
| `OS.toml` | **OS** | `load_os()` | Identity (name, version), voice style, prompt text |
| `FIRMWARE.toml` | **Firmware** | `load_firmware()` | Behavioral rules (structured constraints) |
| `OPERATOR.toml` | **Operator context** | `load_operator()` | Operator-provided context about the deployment |
| `DIRECTIVES.toml` | **Directives** | `load_directives()` | Additional instructional directives |

---

## Memory & Retrieval

| Term | Type | Definition |
|---|---|---|
| **5-Tier Memory** | architecture | Working (30%), Episodic (25%), Semantic (20%), Procedural (15%), Relationship (10%). Percentages are configurable defaults for token budget allocation. |
| **Working Memory** | tier | Short-term context: current goals, active state. Stored as `WorkingEntry`. Highest churn. |
| **Episodic Memory** | tier | Event log: tool use, financial ops, conversations. Stored as `EpisodicEntry` with `importance` (0–10). Subject to decay. |
| **Semantic Memory** | tier | Factual knowledge: learned facts, domain knowledge. Stored as `SemanticEntry` with `confidence` (0.0–1.0). |
| **Procedural Memory** | tier | Skills and recipes: how to do things. Stored as `ProceduralEntry` with `steps` (JSON). |
| **Relationship Memory** | tier | Entity relationships: who/what relates to whom/what. Stored as `RelationshipEntry`. |
| **MemoryBudgetManager** | `struct` | Allocates token budget per tier based on turn complexity (`Simple`, `Medium`, `Complex`). |
| **MemoryRetriever** | `struct` | Hybrid FTS5 + vector cosine search across all 5 tiers. Configurable blend weight. |
| **Hippocampus** | module + table | Long-term memory consolidation system. Manages agent-created memory tables and importance decay scheduling. |
| **Digest** | function | `digest_on_close()` — summarizes a session's episodic content when it closes. Wired into `SessionGovernor` since v0.9.0. |
| **Decay** | process | `decay_importance()` — reduces episodic entry importance over a configurable half-life (`decay_half_life_days`). Promotes stale entries to semantic memory. |
| **Soft trim** | function | `soft_trim()` — removes oldest non-system turns when assembled context exceeds `soft_trim_ratio × max_tokens`. |
| **ANN Index** | `struct` | HNSW approximate nearest-neighbor index (via `instant-distance`) for fast vector similarity search. Rebuilt periodically. |

---

## Inference Pipeline

| Term | Type | Definition |
|---|---|---|
| **LlmService** | `struct` | Facade composing all LLM pipeline stages: cache, circuit breakers, dedup, router, client, providers, capacity, quality, confidence, escalation, embedding. |
| **prepare_inference** | function | Builds a `PreparedInference` from an `InferenceInput`: resolves model, assembles context, applies compression, checks cache. Shared by API and channel entry points. |
| **execute_inference_pipeline** | function | Executes the prepared inference: sends to LLM, streams response, records metrics, updates quality tracker. |
| **PreparedInference** | `struct` | Everything needed to call the LLM: resolved model, composed messages, tool definitions, metadata. |
| **InferenceInput** | `struct` | Raw input to the pipeline: user content, session, claim, config references. |
| **SemanticCache** | `struct` | 3-level response cache: exact hash (instant), tool-specific TTL, semantic cosine similarity (configurable threshold). |
| **DedupTracker** | `struct` | In-flight duplicate detection. Prevents re-querying the same prompt while a prior request is pending. |
| **PromptCompressor** | `struct` | Token pruning and summarization to fit content within context limits. |
| **CircuitBreaker** | per-provider | Tracks failures per provider. Opens after `failure_threshold` consecutive failures; recovers after `recovery_timeout` with exponential backoff. |

---

## Routing & Model Selection

| Term | Type | Definition |
|---|---|---|
| **ModelRouter** | `struct` | Primary router using heuristic feature extraction and complexity scoring. |
| **HeuristicBackend** | `struct` | Feature-based model selection: extracts complexity, token estimate, task type from content. |
| **LogisticBackend** | `struct` | ML-based router using logistic regression preference learning from historical outcomes. |
| **Metascore** | composite score | 5-dimension model quality metric: efficacy, cost, availability, locality, confidence. Defined in `MetascoreBreakdown`. Used by `select_by_metascore()`. |
| **ModelProfile** | `struct` | Per-model capability profile including metascore, cost estimate, latency. |
| **ModelTier** | `enum` | Capability tier: T1 (fastest/cheapest) → T4 (most capable). Used in tiered inference. |
| **Tiered Inference** | strategy | Start at T1, escalate to higher tiers if confidence falls below threshold. Managed by `ConfidenceEvaluator` + `EscalationTracker`. |
| **ConfidenceEvaluator** | `struct` | Measures LLM output confidence against a configurable floor. |
| **EscalationTracker** | `struct` | Records when and why inference escalated to a higher model tier. |
| **QualityTracker** | `struct` | Per-model accuracy metrics. Seeded from `inference_costs` table on startup (warm start). |
| **select_routed_model_with_audit** | function | The routing hot path: extracts features, classifies complexity, builds profiles, selects via metascore, records audit trail. |
| **Cascade** | strategy | Cheapest-first fallback chain. If primary model fails or is unavailable, try the next in the configured fallback list. Distinct from tiered inference (which escalates based on quality, not failure). |
| **ApiFormat** | `enum` | Protocol flavor: `AnthropicMessages`, `OpenAiCompletions`, `OpenAiResponses`, `GoogleGenerativeAi`. |
| **Provider** | `struct` | LLM provider credentials and limits: `api_key`, `base_url`, `tpm_limit`, `rpm_limit`. |
| **CapacityTracker** | `struct` | Sliding-window TPM/RPM tracking per provider. Prevents exceeding rate limits. |

---

## Security & Policy

| Term | Type | Definition |
|---|---|---|
| **SecurityClaim** | `struct` | Immutable principal after auth resolution. Contains `authority` (InputAuthority), `sources` (list of ClaimSource), `ceiling`, `threat_downgraded`, `sender_id`, `channel`. |
| **InputAuthority** | `enum` | Authority level: `External < Peer < SelfGenerated < Creator`. Ordered for min/max composition. |
| **ClaimSource** | `enum` | Authentication layer that contributed a grant: `ChannelAllowList`, `TrustedSenderId`, `ApiKey`, `WsTicket`, `A2aSession`, `Anonymous`. |
| **PolicyEngine** | `struct` | Evaluates all registered `PolicyRule` instances against a `ToolCallRequest`. Returns `Allow` or `Deny{rule, reason}`. |
| **PolicyRule** | `trait` | Single policy rule: `name()`, `priority()`, `evaluate(request, context) → PolicyDecision`. |
| **PolicyDecision** | `enum` | `Allow` or `Deny{rule, reason}`. |
| **RiskLevel** | `enum` | Tool risk tier: `Safe < Caution < Dangerous < Forbidden`. Determines which authority levels can invoke a tool. |
| **Abuse** | subsystem | Detection and response for malicious actors: signal aggregation (per-actor, per-origin, per-channel), quarantine, slowdown, audit trail. |
| **SurvivalTier** | `enum` | Financial health: `High → Normal → LowCompute → Critical → Dead`. Derived from USD balance. Gates expensive operations. |

---

## Channels & Messaging

| Term | Type | Definition |
|---|---|---|
| **ChannelAdapter** | `trait` | Unified interface implemented by every platform: `platform_name()`, `recv()`, `send(OutboundMessage)`. |
| **InboundMessage** | `struct` | Normalized input from any channel: `id`, `platform`, `sender_id`, `content`, `timestamp`, `metadata`. |
| **OutboundMessage** | `struct` | Normalized output to any channel: `content`, `recipient_id`, `metadata`. |
| **ChannelRouter** | `struct` | Multi-channel dispatch and addressability filtering. Routes outbound messages to the correct adapter. |
| **DeliveryQueue** | `struct` | Durable outbound queue with retry logic, persistence, and status tracking. |
| **Poll loop** | pattern | Periodic `recv()` cycle in `poll_loops.rs`. Each adapter has its own loop with configurable interval. |

### Platform Adapters

| Adapter | Platform | Transport | Notes |
|---|---|---|---|
| **TelegramAdapter** | Telegram | Bot API (long-poll + webhook) | Markdown V2 formatting |
| **WhatsAppAdapter** | WhatsApp | Cloud API (webhook) | Message templates, media via Graph API |
| **DiscordAdapter** | Discord | Gateway WebSocket + REST | Slash commands, embeds, CDN attachments |
| **SignalAdapter** | Signal | signal-cli daemon (JSON-RPC) | End-to-end encrypted |
| **EmailAdapter** | Email | IMAP listener + SMTP sender | OAuth2 (XOAUTH2), IDLE support, RFC 5322 parsing |
| **VoicePipeline** | Voice | WebRTC | STT (Whisper), TTS, bidirectional audio |
| **A2aProtocol** | Agent-to-Agent | Custom | Zero-trust: ECDH key exchange, AES-256-GCM |

---

## Multimodal & Media

| Term | Type | Definition |
|---|---|---|
| **MediaAttachment** | `struct` | Attachment metadata: `media_type`, `source_url`, `local_path`, `filename`, `content_type`, `size_bytes`, `caption`. Serialized into `InboundMessage.metadata`. |
| **MediaType** | `enum` | Classification: `Image`, `Audio`, `Video`, `Document`. |
| **MediaService** | `struct` | Downloads, validates, and stores media. SSRF-safe URL validation, size limits per type, streaming download with early abort. |
| **MultimodalConfig** | `struct` | Controls media handling: `enabled`, `media_dir`, per-type size limits, `vision_model`, `transcription_model`, `auto_transcribe_audio`, `auto_describe_images`. |

---

## Orchestration & Multi-Agent

| Term | Type | Definition |
|---|---|---|
| **Orchestrator** | role | The primary agent that manages subagents. Singular per deployment. Uses `orchestration.rs` for task decomposition and delegation. |
| **Subagent** | role + struct | A taskable child agent with fixed skills, no personality. Role value: `"subagent"`. Stored in `sub_agents` table. _(Legacy: "specialist")_ |
| **Model-proxy** | role | A routing/proxy record. Role value: `"model-proxy"`. Cannot own skills, cannot be tasked. Not a subagent. |
| **Subtask** | `struct` | A work item assigned to a subagent: `description`, `status` (Pending/Assigned/Running/Completed/Failed). |
| **Workflow** | `struct` | Multi-step orchestration pattern: `Sequential`, `Parallel`, `FanOutFanIn`, `Handoff`. |
| **SubagentRegistry** | `struct` | Runtime registry of enabled subagents with their roles, skills, and model overrides. |

**See also:** `docs/architecture/subagent-ubiquitous-language.md` for the full
subagent contract, model selection flowchart, and dataflow diagrams.

---

## Skills & Extensibility

| Term | Type | Definition |
|---|---|---|
| **Skill** | concept | A reusable capability definition. Two formats: Structured (TOML tool-chain) or Instruction (Markdown body). |
| **SkillKind** | `enum` | `Structured` (tool-chain) or `Instruction` (markdown). |
| **SkillManifest** | `struct` | Structured skill: triggers, priority, risk_level, policy_overrides, steps (ToolChainStep list). |
| **InstructionSkill** | `struct` | Markdown-based skill: body text, triggers, name, description, priority. |
| **SkillTrigger** | `struct` | Activation rules: keywords, tool_names, regex_patterns. |
| **SkillRegistry** | `struct` | In-memory skill store. `match_skills(keywords)` returns ranked matches. |
| **Tool** | `trait` | Async execution interface: `execute(params, context) → ToolResult`. |
| **ToolRegistry** | `struct` | Registry of 10+ tool categories (filesystem, bash, database, browser, etc.). |
| **ToolContext** | `struct` | Execution context: agent_id, session_id, workspace_root, user_id. |
| **PluginRegistry** | `struct` | WASM and external plugin management. Configured via `PluginsConfig`. |
| **McpClientManager** | `struct` | Model Context Protocol client connections to external tool servers. |
| **McpServerRegistry** | `struct` | MCP server-side resource registry (when Ironclad acts as an MCP server). |
| **ObsidianVault** | `struct` | Obsidian vault integration: indexing, search, read/write of markdown notes. |

---

## Scheduling & Daemons

| Term | Type | Definition |
|---|---|---|
| **HeartbeatDaemon** | `struct` | Periodic tick loop that drives all registered `HeartbeatTask` instances. |
| **HeartbeatTask** | `trait` | Pluggable periodic task: `async fn run(TickContext) → TaskResult`. |
| **TickContext** | `struct` | Per-tick execution context: db, wallet, config, instance_id. |
| **DurableScheduler** | `struct` | Cron expression and fixed-interval evaluation. Persists state for crash recovery. |
| **ScheduleKind** | `enum` | `Cron` (expression), `Every` (fixed interval), `At` (absolute time). |
| **Durable Lease** | pattern | Cron jobs acquire database-backed leases (`lease_acquired`, `instance_id`) to prevent duplicate execution across restarts. |
| **Governor** | `struct` | `SessionGovernor` — manages session lifecycle: expiry, rotation, compaction, digest trigger, importance decay. Runs on heartbeat tick. |

---

## Financial & Wallet

| Term | Type | Definition |
|---|---|---|
| **WalletService** | `struct` | Facade: wallet (Ethereum), treasury (policy), yield engine (DeFi). |
| **Wallet** | `struct` | HD wallet on Base (chain 8453): address, balance tracking, `load_or_generate()`. |
| **Money** | `struct` | USDC amount type with formatting and arithmetic. |
| **TreasuryPolicy** | `struct` | Spending limits: `per_payment_cap`, `minimum_reserve`, survival-tier-aware multipliers. |
| **YieldEngine** | `struct` | DeFi yield optimization (Aave/Compound on Base). |
| **X402Handler** | `struct` | x402 (EIP-3009) payment protocol: `transferWithAuthorization()` flow. |
| **SurvivalTier** | `enum` | Financial health derived from balance: `High → Normal → LowCompute → Critical → Dead`. |

---

## Browser Automation

| Term | Type | Definition |
|---|---|---|
| **Browser** | `struct` | High-level facade: process manager, CDP session, action executor. |
| **BrowserAction** | `enum` | 12 action variants: navigate, click, type, screenshot, scroll, evaluate, etc. |
| **ActionExecutor** | `struct` | Executes `BrowserAction` → `ActionResult`. |
| **BrowserManager** | `struct` | Chrome/Chromium process lifecycle (detect, start, stop). |
| **CdpSession** | `struct` | Chrome DevTools Protocol WebSocket session (connect, send commands, close). |
| **PageContent** | `struct` | Extracted page data: url, title, text, html_length. |

---

## Server & Operations

| Term | Type | Definition |
|---|---|---|
| **AppState** | `struct` | The god-object shared state for all API routes. Contains db, config, llm, wallet, adapters, policy engine, tool registry, event bus, and ~30 other subsystem handles. |
| **EventBus** | `type` | Tokio broadcast channel for real-time WebSocket event push to connected clients. |
| **TicketStore** | `struct` | Session-scoped WebSocket authentication tokens. Short-lived, single-use. |
| **Dashboard** | SPA | Embedded single-page application for operator monitoring: metrics, sessions, model selection, diagnostics. |
| **Mechanic** | CLI | Administrative command interface: `mechanic --repair`, setup wizards, model selection, plugin management. |
| **Operator** | role (human) | The person administering an Ironclad deployment. Uses mechanic CLI and/or dashboard. Not the same as "user" (who sends messages to the agent). |

---

## Persistence & Database

| Term | Type | Definition |
|---|---|---|
| **Database** | `struct` | Thread-safe SQLite handle: `Arc<Mutex<Connection>>` with WAL mode for concurrent reads. |
| **Message** | `struct` | A conversation turn record: id, session_id, parent_id, role (`user`/`assistant`/`system`), content, usage_json, created_at. |
| **TurnRecord** | `struct` | Structured metadata for a turn: turn_index, speaker_role, complexity, summary. |
| **TurnFeedback** | `struct` | User feedback on a turn: rating, thumbs_up, issues_found. |
| **Checkpoint** | module | Save/restore session context snapshots (`context_snapshots` table). |

---

## Architectural Patterns

| Pattern | Canonical Name | Definition |
|---|---|---|
| **ReAct Loop** | ReAct state machine | Think → Act → Observe → Persist cycle with idle detection, loop detection, and max-turn enforcement. |
| **Unified Pipeline** | unified inference pipeline | `prepare_inference()` → `execute_inference_pipeline()` in `core.rs`. Both API and channel entry points converge here. |
| **Claim-based RBAC** | security claim composition | Multiple `ClaimSource` layers contribute grants; the final `SecurityClaim` is capped by threat ceiling. |
| **5-Tier Memory** | 5-tier hybrid retrieval | FTS5 full-text + vector cosine across five memory tiers with configurable budget allocation. |
| **Tiered Inference** | tiered inference with escalation | Start cheap (T1), escalate on low confidence. Distinct from cascade (which handles _failure_, not _quality_). |
| **Cascade Fallback** | cascade optimizer | Cheapest-first fallback chain on provider failure. Distinct from tiered (which handles _quality_, not _failure_). |
| **Circuit Breaker** | per-provider circuit breaker | Opens after N failures, exponential backoff recovery. |
| **Durable Lease** | durable lease coordination | Database-backed mutex for cron jobs — prevents duplicate execution across restarts. |
| **Poll Loop** | channel poll loop | Periodic `recv()` → dispatch cycle. Per-adapter, configurable interval. |
| **Governor Tick** | session governor | Periodic session lifecycle management: expire, compact, rotate, decay, digest. |

---

## Deprecated / Removed Terms

These terms are **no longer valid** and should not be used in new code or docs:

| Removed Term | Replaced By | When | Notes |
|---|---|---|---|
| `specialist` (role) | `subagent` | v0.8.2 | Role validation normalizes on input |
| `commander` | `orchestrator` | pre-v0.8 | Legacy alias; may exist in old persisted data and is normalized/removed by mechanic repair |
| `needs_pruning()` | `soft_trim()` | v0.9.0 | Function renamed |
| `uniroute.rs` | `router.rs` + `profile.rs` | v0.9.2 | Dead code removed (`ModelVector`, `QueryRequirements`) |
| `select_for_complexity()` | `select_routed_model_with_audit()` | v0.9.2 | Dead code removed |
| `select_cheapest_qualified()` | `select_routed_model_with_audit()` | v0.9.2 | Dead code removed |

### Pending Renames (via `mechanic --repair`)

These legacy names still appear in code but are **scheduled for rename in
v0.9.4**. The migration will be handled by the `mechanic --repair`
transformation pipeline, which can update both code identifiers and on-disk
config files atomically. **Do not** treat these as permanent — new code must
use the canonical terms.

| Legacy Name | Target Rename | Where It Appears | Migration Notes |
|---|---|---|---|
| `soul_text` (field) | `os_text` | `PersonalityState.soul_text` | Field rename + serde alias for one release cycle |
| `compose_soul()` (fn) | `compose_os_firmware()` | `personality.rs` | Deprecate → remove after callers migrate |
| `PersonalityState` (struct) | `IdentityState` or `OsFirmwareState` | `api/routes/mod.rs` | Struct used across crate boundary; type alias transition |
| `personality` (config key) | `identity` or `os` | `ironclad.toml`, admin routes | `mechanic --repair` rewrites config files on upgrade |

---

_Last updated: v0.9.3 — Channel Expansion release._
