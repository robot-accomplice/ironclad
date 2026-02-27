# Architecture Drift Report — v0.8.0

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Diagrams audited against v0.8.0 code. Diagrams were last updated at v0.5.0-v0.6.0.

## Summary

| File | Diagrams | Structural | Relationship | Behavioral | Naming | Status |
|------|----------|-----------|-------------|-----------|--------|--------|
| `ironclad-c4-system-context.md` | 1 (C4Context) | 7 missing nodes, 1 stale node | 2 relationship-label gaps | 0 | 1 vague label | Drifted |
| `ironclad-c4-container.md` | 1 (C4Container) | 0 | 2 spurious arrows, 1 missing arrow, 1 missing table dep, 8 missing `core` arrows, 6 missing `server` arrows | 0 | 0 | Drifted |
| `ironclad-c4-core.md` | 1 (flowchart) | 1 missing module, 18+ missing config structs | 0 | 0 | 2 stale labels (error variant count, ChannelsConfig fields) | Drifted |
| `ironclad-c4-db.md` | 1 (flowchart) | 3 missing modules | 0 | 0 | 1 stale "Depended on by" list | Drifted |
| `ironclad-c4-llm.md` | 1 (flowchart) | 1 phantom module (transform.rs not in pub mod), 1 missing top-level struct (LlmService) | 0 | 0 | 0 | Minor drift |
| `ironclad-c4-agent.md` | 2 (flowchart + sequence) | 0 missing modules | 0 | 0 | 0 | Accurate |
| `ironclad-c4-wallet.md` | 2 (flowchart + sequence) | 0 | 0 | 0 | 1 (money.rs misplaced as wallet.rs child) | Minor drift |
| `ironclad-c4-channels.md` | 2 (flowchart + sequence) | 0 | 0 | 0 | 1 stale dep list, 1 stale struct field names | Drifted |
| `ironclad-c4-schedule.md` | 2 (flowchart + sequence) | 0 | 0 | 1 (agentTurn is legacy noop) | 1 stale enum variant count | Minor drift |
| `ironclad-c4-server.md` | 1 (flowchart) | 5 missing modules from diagram (present in table) | 0 | 0 | 0 | Drifted |
| `ironclad-c4-browser.md` | 1 (flowchart) | 0 | 0 | 0 | 0 | Accurate |
| `ironclad-c4-plugin-sdk.md` | 1 (flowchart) | 0 | 0 | 0 | 1 stale ToolDef fields, 1 stale dep-by list | Drifted |
| `ironclad-dataflow.md` | 20 (flowcharts) | 0 | 0 | 4 behavioral mismatches | 5 stale counts/labels | Drifted |
| `ironclad-sequences.md` | 13 (sequence diagrams) | 1 unimplemented diagram (TLS) | 0 | 3 behavioral mismatches | 8 stale counts/labels, 4 phantom types, 2 wrong table names | Drifted |
| `circuit-breaker-audit.md` | 3 (sequences) + 1 (flowchart) | 0 | 0 | 3 of 5 findings now fixed; 1 stale diagram path | 1 dead config field | Mostly fixed |
| `router-audit.md` | 3 (sequences) + 1 (flowchart) | 0 | 0 | 1 of 4 findings now fixed; 1 stale diagram path | 0 | Partially fixed |

## Detailed Findings

### ironclad-c4-system-context.md

**Audit scope:** All `Person`, `System`, `System_Ext` nodes and `Rel` edges in the
Mermaid `C4Context` block (lines 10-42), cross-referenced against v0.8.0 source code
in `ironclad-llm`, `ironclad-channels`, `ironclad-wallet`, `ironclad-browser`, and
`ironclad-server`.

#### Nodes confirmed present and accurate

| Diagram Node | Code Evidence | Status |
|---|---|---|
| `Person(creator)` | All channel adapters accept inbound messages from human operators | OK |
| `System(ironclad)` | Single-binary confirmed (`ironclad-server/src/main.rs`) | OK |
| `System_Ext(anthropic)` | `bundled_providers.toml` line 52, `ApiFormat::AnthropicMessages` | OK |
| `System_Ext(openai)` | `bundled_providers.toml` line 40, chat + embedding paths | OK |
| `System_Ext(ollama)` | `bundled_providers.toml` line 1, `infer_is_local()` | OK |
| `System_Ext(telegram)` | `ironclad-channels/src/telegram.rs`, `ChannelsConfig::telegram` | OK |
| `System_Ext(whatsapp)` | `ironclad-channels/src/whatsapp.rs`, `ChannelsConfig::whatsapp` | OK |
| `System_Ext(web)` | `ironclad-channels/src/web.rs`, `ironclad-server/src/ws.rs`, `dashboard.rs` | OK |
| `System_Ext(baseChain)` | `ironclad-wallet`: alloy-rs, chain_id 8453, Aave V3 pool | OK |
| `System_Ext(peerAgents)` | `ironclad-channels/src/a2a.rs`: ECDH + AES-256-GCM sessions | OK |

#### S-1: Stale node -- Groq

`System_Ext(groq)` (line 20) declares Groq as a first-class LLM provider. Groq is
**not** in `bundled_providers.toml` and has no dedicated integration code. It appears
only in the dashboard SPA known-providers dropdown, meaning it can be added by users
manually but is not bundled. The diagram overstates its status by giving it an
explicit node equal to Anthropic/OpenAI/Ollama.

**Recommendation:** Demote Groq into the "Other LLM Providers" catch-all node, or
remove and note it as user-configurable.

#### S-2: Missing node -- Discord

`ironclad-channels/src/discord.rs` implements a full `DiscordAdapter` (Discord
Gateway + REST API v10), with `DiscordConfig` in `ironclad-core`. No `System_Ext`
node exists in the diagram.

#### S-3: Missing node -- Signal

`ironclad-channels/src/signal.rs` implements `SignalAdapter` via signal-cli JSON-RPC
daemon, with `SignalConfig` in `ironclad-core`. No `System_Ext` node exists in the
diagram.

#### S-4: Missing node -- Email (IMAP/SMTP)

`ironclad-channels/src/email.rs` implements `EmailAdapter` with `lettre` SMTP
transport and IMAP listener, with `EmailConfig` in `ironclad-core`. No `System_Ext`
node exists in the diagram.

#### S-5: Missing node -- Voice (STT/TTS)

`ironclad-channels/src/voice.rs` implements voice processing with Whisper STT and
OpenAI TTS, with `VoiceChannelConfig` in `ironclad-core`. No `System_Ext` node
exists. This represents an external dependency on OpenAI's Audio API (or a local
Whisper instance).

#### S-6: Missing node -- Chrome/Chromium (CDP)

The entire `ironclad-browser` crate provides headless browser automation via Chrome
DevTools Protocol. It manages a Chromium process, establishes CDP WebSocket sessions,
and executes 12 browser actions. No `System_Ext` node for Chrome/Chromium exists in
the diagram.

#### S-7: Missing node -- OpenRouter

`bundled_providers.toml` line 76 defines OpenRouter (`https://openrouter.ai/api`) as
a bundled T2 provider. This is a distinct aggregator service that routes to many
backends. Not represented in the diagram.

#### S-8: Missing node -- Google Generative AI (Gemini)

`bundled_providers.toml` line 65 defines Google as a bundled T3 provider with its own
`ApiFormat::GoogleGenerativeAi`, dedicated request/response translation in
`format.rs`, and embedding support. The diagram's "Other LLM Providers" label says
"Google, Moonshot, etc." but Google now has first-class format support comparable to
Anthropic and OpenAI and warrants its own explicit node.

#### R-1: Relationship label gap -- Creator channels

The `Rel(creator, ironclad)` edge (line 31) lists "Telegram / WhatsApp / WebSocket /
HTTP API / Dashboard" but omits Discord, Signal, Email, and Voice, all of which are
now implemented channel adapters with config support.

#### R-2: Relationship label gap -- "Other LLM Providers" too vague

The `otherLlms` node label "Google, Moonshot, etc. -- configurable" does not
communicate that v0.8.0 bundles 11 providers by default (ollama, sglang, vllm,
docker-model-runner, openai, anthropic, google, openrouter, moonshot, llama-cpp, and
a catch-all for user-defined ones). At minimum, Google and OpenRouter should be
explicit nodes given their distinct API formats or aggregator role.

#### N-1: Naming -- "Other LLM Providers" label vagueness

The label "Google, Moonshot, etc." is outdated. The bundled provider set now includes
SGLang, vLLM, Docker Model Runner, llama-cpp, and OpenRouter in addition to Google
and Moonshot. Recommend updating the label to be more representative or adding
individual nodes for the most significant ones.

#### Notes

- **SQLite** is referenced in the `ironclad` system description ("unified SQLite
  DB") and implemented in `ironclad-db`. This is correct as an internal component,
  not an external system, so no `System_Ext` is needed.
- **x402 payment protocol** (`ironclad-wallet/src/x402.rs`) interacts with external
  services when handling HTTP 402 responses. This is a protocol, not a distinct
  system, so the existing `baseChain` node adequately covers it.
- **Compound** is mentioned in `ironclad-wallet` doc comments ("Aave/Compound") but
  the actual yield engine code only implements Aave V3. This is a minor doc comment
  inaccuracy in the crate, not a diagram issue.
- The local inference providers (SGLang, vLLM, Docker Model Runner, llama-cpp) are
  arguably peers to Ollama and could share a "Local Inference Runtimes" `System_Ext`
  node rather than individual nodes.

### ironclad-c4-container.md

**Audit scope:** All `Container` nodes, the `Crates (Workspace Members)` table, and
every `Rel` edge in the Mermaid `C4Container` block (lines 9-60), cross-referenced
against the actual `Cargo.toml` inter-crate dependencies for all 11 workspace members
in v0.8.0.

**Method:** Ran `grep 'ironclad-' crates/ironclad-*/Cargo.toml` to extract every
inter-crate dependency, then compared against both the Mermaid relationship arrows and
the "Depends On" column in the crate table.

#### Containers confirmed present and accurately described

All 10 production containers are present in the diagram with correct names and
descriptions. `ironclad-tests` is listed in the table as "Integration tests" and
correctly omitted from the Mermaid diagram itself.

| Container | Cargo.toml deps | Diagram Table "Depends On" | Table Match? |
|---|---|---|---|
| `ironclad-core` | -- | -- | OK |
| `ironclad-db` | core | core | OK |
| `ironclad-llm` | core | core | OK |
| `ironclad-agent` | core, db, llm | core, db, llm | OK |
| `ironclad-wallet` | core, db | core, db | OK |
| `ironclad-schedule` | core, db, agent, wallet | core, db, agent, wallet | OK |
| `ironclad-channels` | **core, db** | **core only** | **MISMATCH** |
| `ironclad-plugin-sdk` | core | core | OK |
| `ironclad-browser` | core | core | OK |
| `ironclad-server` | core, db, llm, agent, wallet, schedule, channels, plugin-sdk, browser | "All of the above (except tests)" | OK (vague) |
| `ironclad-tests` | core, db, llm, agent, wallet, schedule, channels, server, plugin-sdk, browser | "Multiple crates" | OK (vague) |

#### C-1: Spurious arrow -- `channels -> agent` (no Cargo.toml dependency)

Line 46: `Rel(channels, agent, "In-process")` implies `ironclad-channels` directly
depends on `ironclad-agent`. **In reality, `ironclad-channels/Cargo.toml` lists only
`ironclad-core` and `ironclad-db` as dependencies.** The channel-to-agent wiring is
performed by `ironclad-server`, which depends on both crates and connects them at
bootstrap. This arrow is structurally wrong -- it shows a direct dependency that does
not exist in the crate graph.

**Impact:** Medium. This misrepresents the dependency graph and could mislead
developers about crate layering. A developer might expect to find agent imports in the
channels crate and fail.

**Recommendation:** Remove `Rel(channels, agent, ...)` from the Mermaid diagram. Add
`Rel(server, channels, "In-process")` (which is a real dependency). The runtime
channel-to-agent dispatch should be documented as server-mediated, not as a direct
dependency.

#### C-2: Spurious arrow -- `llm -> db` (no Cargo.toml dependency)

Line 50: `Rel(llm, db, "Indirect via server: inference_costs recording mediated by
ironclad-server")` shows an arrow from `ironclad-llm` to `ironclad-db`. **However,
`ironclad-llm/Cargo.toml` lists only `ironclad-core` as a dependency.** The label
acknowledges the relationship is "indirect via server", but drawing it as a `Rel` edge
in a C4 Container diagram implies a direct dependency. C4 Rel edges represent runtime
communication or compile-time coupling. This is neither -- it is a server-mediated
side-effect.

**Impact:** Low-Medium. The label is honest about the indirection, but the arrow is
still misleading in a dependency-graph context.

**Recommendation:** Remove the `Rel(llm, db, ...)` arrow. If the cost-recording
pathway needs documentation, add it as a note or document it on the `server -> llm`
and `server -> db` arrows.

#### C-3: Missing dependency -- `channels -> db` (in Cargo.toml, absent from diagram)

`ironclad-channels/Cargo.toml` declares `ironclad-db.workspace = true`.
`ironclad-channels/src/delivery.rs` and `src/router.rs` import and use
`ironclad_db::Database` directly. The diagram has NO `Rel(channels, db, ...)` arrow,
and the crate table lists channels as depending only on `ironclad-core`.

**Impact:** Medium. This is a real compile-time dependency that is completely invisible
in the diagram. The channels crate uses the DB for its delivery queue system.

**Recommendation:** Add `Rel(channels, db, "In-process: delivery queue")` to the
Mermaid diagram. Update the table row for `ironclad-channels` to read
`ironclad-core`, `ironclad-db`.

#### C-4: Missing `ironclad-core` dependency arrows (8 arrows)

Every crate except `ironclad-core` itself depends on `ironclad-core` per Cargo.toml:
`db`, `llm`, `agent`, `wallet`, `schedule`, `channels`, `plugin-sdk`, `browser`. The
Mermaid diagram shows **zero** `Rel(*, core, ...)` arrows. While omitting foundational
dependencies is a common C4 diagram simplification to reduce visual clutter, this
means the diagram does not accurately represent the dependency graph.

**Impact:** Low. This is a deliberate diagram simplification. The crate table correctly
lists `ironclad-core` in the "Depends On" column for all crates that use it. However,
since this diagram is described as defining "the dependency graph that drives our
inside-out validation strategy," the omission is more significant than in a typical
overview diagram.

**Recommendation:** Either (a) add a diagram note stating "All containers depend on
ironclad-core; arrows omitted for clarity" or (b) add the arrows. Option (a) is
preferred for readability.

#### C-5: Missing `ironclad-server` dependency arrows (6 arrows)

`ironclad-server/Cargo.toml` depends on 9 internal crates: core, db, llm, agent,
wallet, schedule, channels, plugin-sdk, browser. The diagram shows only 3 arrows from
server: `server -> agent`, `server -> pluginSdk`, `server -> browser`. Missing arrows:

- `server -> core`
- `server -> db`
- `server -> llm`
- `server -> wallet`
- `server -> schedule`
- `server -> channels`

**Impact:** Medium. The server is the integration hub and top-level crate. Missing 6
of 9 dependency arrows (including the critical `server -> channels` arrow) obscures
the actual dependency fan-out and makes the diagram unreliable for understanding the
build graph.

**Recommendation:** Add at minimum `Rel(server, db, ...)`, `Rel(server, llm, ...)`,
`Rel(server, schedule, ...)`, `Rel(server, wallet, ...)`, and `Rel(server, channels,
...)`. The `server -> core` arrow can be omitted along with other core arrows per
the recommendation in C-4.

#### Notes

- **Omitted core arrows are a recognized C4 convention.** Many C4 diagrams omit
  arrows to foundational/utility containers to reduce clutter. The crate table
  compensates for this. If the project decides this is acceptable, C-4 can be
  downgraded to informational. However, the missing `server` arrows (C-5) and the
  missing `channels -> db` arrow (C-3) are NOT justifiable as simplification -- they
  represent real, non-obvious dependency paths.

- **The `Rel(channels, agent)` arrow (C-1) is the most significant finding.** It
  represents a dependency that does not exist and inverts the actual architecture (the
  server mediates the connection). This could mislead someone attempting to refactor
  the channels crate or reason about its compile-time surface.

- **The diagram table is mostly accurate.** Only one cell is wrong (channels missing
  db). The table provides a useful ground truth even where the arrows are incomplete.

- **ironclad-tests** is correctly excluded from the Mermaid diagram but present in the
  table. Its "Multiple crates" description is vague but acceptable since it depends on
  10 of 11 workspace members.

### ironclad-c4-core.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 11-97),
cross-referenced against v0.8.0 source code in `crates/ironclad-core/src/`.

**Method:** Compared every component and subgraph node declared in the diagram against
the actual `lib.rs` module declarations, `config.rs` struct definitions, `types.rs`
enum/struct definitions, and `error.rs` variant list.

#### Modules confirmed present and accurately described

| Diagram Module | Code File | Status |
|---|---|---|
| `config.rs` | `config.rs` (57,556 bytes) | OK |
| `error.rs` | `error.rs` (7,509 bytes) | OK |
| `types.rs` | `types.rs` (7,057 bytes) | OK |
| `personality.rs` | `personality.rs` (39,289 bytes) | OK |
| `style.rs` | `style.rs` (18,496 bytes) | OK |
| `keystore.rs` | `keystore.rs` (18,198 bytes) | OK |

#### Types confirmed present and accurate

| Diagram Type | Code Evidence | Status |
|---|---|---|
| `SurvivalTier` | `types.rs` line 6: 5 variants (High, Normal, LowCompute, Critical, Dead) | OK |
| `AgentState` | `types.rs` line 31: 5 variants (Setup, Waking, Running, Sleeping, Dead) | OK |
| `ApiFormat` | `types.rs` line 40: 4 variants (AnthropicMessages, OpenAiCompletions, OpenAiResponses, GoogleGenerativeAi) | OK |
| `ModelTier` | `types.rs` line 48: T1-T4 | OK |
| `PolicyDecision` | `types.rs` line 56: Allow, Deny | OK |
| `RiskLevel` | `types.rs` line 68: Safe, Caution, Dangerous, Forbidden | OK |
| `InputAuthority` | `types.rs` line 131: Creator, SelfGenerated, Peer, External | OK |
| `SkillKind` | `types.rs` line 76: Structured, Instruction | OK |
| `SkillTrigger` | `types.rs` line 82: struct with keywords, tool_names, regex_patterns | OK |
| `SkillManifest` | `types.rs` line 92 | OK |
| `InstructionSkill` | `types.rs` line 113 | OK |
| `ToolChainStep` | `types.rs` line 107 | OK |
| `ScheduleKind` | `types.rs` line 139: Cron, Every, At | OK |

#### CORE-1: Missing module -- `input_capability_scan`

`lib.rs` line 26 declares `pub mod input_capability_scan;`. This module
(`input_capability_scan.rs`, 199 lines) provides `InputCapabilityScan` struct and
`scan_input_capabilities()` function that analyzes JSON tool inputs to detect
filesystem, network, and environment access requirements. This is a security-relevant
module used by the policy engine. It has no representation in the diagram -- not as a
component, not in the module table.

**Impact:** Medium. This is a public module that other crates can use for input
sandboxing decisions. Its absence from the diagram means the security scanning
capability of `ironclad-core` is invisible.

#### CORE-2: Stale label -- IroncladError variant count

The diagram (line 81) and the module table state `IroncladError` has "13 variants".
The actual code (`error.rs`) has **14 variants**: Config, Channel, Database, Llm,
Network, Policy, Tool, Wallet, Injection, Schedule, A2a, Io, Skill, **Keystore**. The
`Keystore` variant was added after the diagram was written.

**Impact:** Low. The variant list in the diagram node (line 81) enumerates 13 names
and omits Keystore. This is a minor label staleness.

#### CORE-3: Stale label -- ChannelsConfig field list

The diagram (line 29) shows `ChannelsConfig` with fields "telegram, whatsapp". The
actual `ChannelsConfig` struct in `config.rs` (line 1126) has **8 fields**: `telegram`,
`whatsapp`, `discord`, `signal`, `email`, `voice`, `trusted_sender_ids`,
`thinking_threshold_seconds`, plus `startup_announcements`. This significantly
understates the channel configuration surface.

**Impact:** Medium. The diagram gives the impression that only Telegram and WhatsApp
are configured here, when in fact Discord, Signal, Email, and Voice are all first-class
channel configs.

#### CORE-4: Missing config structs (18+ structs absent from diagram)

The diagram's `ConfigDetail` subgraph shows 13 config structs organized into 4
groups (Infrastructure, AI Pipeline, Financial, Extensions). The actual `config.rs`
contains **40+ pub structs**. The following significant structs are absent from the
diagram:

**Infrastructure:** `ContextConfig`, `ApprovalsConfig`, `PluginsConfig`,
`BrowserConfig`, `DaemonConfig`, `UpdateConfig`, `PersonalityConfig`, `SessionConfig`,
`McpConfig`, `McpClientConfig`, `DiscoveryConfig`, `DeviceConfig`, `WorkspaceConfig`

**AI Pipeline:** `TieredInferenceConfig`, `TierAdaptConfig`, `ModelOverride`,
`MultimodalConfig`, `KnowledgeConfig`, `KnowledgeSourceEntry`, `DigestConfig`

**Channel-specific:** `DiscordConfig`, `SignalConfig`, `EmailConfig`,
`VoiceChannelConfig`, `TelegramConfig`, `WhatsAppConfig` (last two are present
indirectly but not as nodes)

**Impact:** Medium. The config.rs file has grown from ~15 structs to 40+. The diagram
captures fewer than a third of the actual configuration surface. New subsystems
(browser automation, MCP integration, plugins, approvals, context management, daemon
mode, auto-update, multimodal, knowledge/digest) all have configuration structs that
are invisible in the diagram.

**Recommendation:** Either (a) add the missing config groups (at minimum: Context,
Approvals, Plugins, Browser, Daemon, MCP, Multimodal, Knowledge) as subgraph nodes, or
(b) add a note acknowledging the diagram shows a subset and pointing readers to the
source for the full configuration schema.

#### Notes

- The **ApiFormat "4 variants"** label is currently correct. The `OpenAiResponses`
  variant was likely added post-v0.5.0 but the count was already stated as 4 in the
  diagram, so this happens to be accurate.
- The `bundled_providers.toml` file exists in the `ironclad-core/src/` directory but
  is not a Rust module -- it is an embedded data file. The diagram does not mention it,
  which is acceptable since it is consumed by `config.rs` at compile time.
- The `personality.rs` module has grown substantially (39,289 bytes) but its documented
  responsibilities (load OS/soul/firmware, compose identity text) remain accurate.
- The `style.rs` module has also grown (18,496 bytes) but its documented
  responsibilities remain accurate.

### ironclad-c4-db.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 11-114), the
Tables Managed table, and the Dependencies section, cross-referenced against v0.8.0
source code in `crates/ironclad-db/src/`.

**Method:** Compared every component node and subgraph in the diagram against the
actual `lib.rs` module declarations and source files in the `src/` directory.

#### Modules confirmed present and accurately described

| Diagram Module | Code File | Status |
|---|---|---|
| `schema.rs` | `schema.rs` (23,535 bytes) | OK |
| `sessions.rs` | `sessions.rs` (47,207 bytes) | OK |
| `memory.rs` | `memory.rs` (25,511 bytes) | OK |
| `tools.rs` | `tools.rs` (9,772 bytes) | OK |
| `policy.rs` | `policy.rs` (4,578 bytes) | OK |
| `metrics.rs` | `metrics.rs` (5,969 bytes) | OK |
| `cron.rs` | `cron.rs` (18,456 bytes) | OK |
| `skills.rs` | `skills.rs` (17,538 bytes) | OK |
| `embeddings.rs` | `embeddings.rs` (13,069 bytes) | OK |
| `ann.rs` | `ann.rs` (11,834 bytes) | OK |
| `cache.rs` | `cache.rs` (7,793 bytes) | OK |
| `hippocampus.rs` | `hippocampus.rs` (19,055 bytes) | OK |
| `checkpoint.rs` | `checkpoint.rs` (5,793 bytes) | OK |
| `backend.rs` | `backend.rs` (10,062 bytes) | OK |
| `agents.rs` | `agents.rs` (6,820 bytes) | OK |
| `migrations/` | `migrations/` directory | OK |

#### DB-1: Missing module -- `approvals`

`lib.rs` line 33 declares `pub mod approvals;`. The file `approvals.rs` (1,317 bytes)
provides CRUD operations for the `approval_requests` table (gated tool approvals with
pending/approved/denied states and timeout). The table IS listed in the Tables Managed
section (line 144) but has no component node in the diagram and no detail subgraph.

**Impact:** Low. The table is documented but the module is not shown as a component.

#### DB-2: Missing module -- `delivery_queue`

`lib.rs` line 38 declares `pub mod delivery_queue;`. The file `delivery_queue.rs`
(8,810 bytes) provides CRUD operations for the `delivery_queue` table (outbound channel
message delivery with status tracking, retry logic, and next_retry_at). The table IS
listed in the Tables Managed section (line 143) but has no component node in the
diagram and no detail subgraph.

**Impact:** Medium. This is a significant module (8,810 bytes) that manages outbound
message delivery state. It is used by `ironclad-channels` for reliable message delivery.

#### DB-3: Missing module -- `efficiency`

`lib.rs` line 39 declares `pub mod efficiency;`. The file `efficiency.rs` (25,454
bytes) provides efficiency metrics tracking and queries. This is a substantial module
-- larger than `memory.rs` -- with no representation in the diagram at all. It is not
listed in the module doc comment's module list, though it does exist as a `pub mod`
declaration.

**Impact:** Medium. This is one of the largest modules in the crate (25,454 bytes) and
is completely invisible in the diagram.

#### DB-4: Missing module -- `model_selection`

`lib.rs` line 44 declares `pub mod model_selection;`. The file `model_selection.rs`
(4,232 bytes) provides model selection history tracking and queries. No representation
in the diagram.

**Impact:** Low. Smaller module but still a public API surface.

#### DB-5: Stale "Depended on by" list

The Dependencies section (line 158) states: "Depended on by: ironclad-agent,
ironclad-schedule, ironclad-wallet, ironclad-server". This omits `ironclad-channels`,
which depends on `ironclad-db` in its Cargo.toml and uses it for the delivery queue.
Also omits `ironclad-tests`.

**Impact:** Low. This was already identified in the container diagram audit (C-3/
BUG-012) but is independently wrong in this diagram's Dependencies section.

#### Notes

- The **Tables Managed** section is impressively comprehensive. It lists 28 tables
  including `delivery_queue`, `approval_requests`, `plugins`, and `turn_feedback` --
  all of which are accurately described. The table documentation is more up-to-date
  than the component diagram itself.
- The `sessions.rs` module has grown to 47,207 bytes, making it the largest module in
  the crate. The diagram's detail subgraph shows 5 functions which is a reasonable
  high-level summary.
- The `efficiency.rs` module at 25,454 bytes is the second-largest and completely
  absent from the diagram, making it the most significant omission.

### ironclad-c4-llm.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 11-143),
the Request Pipeline section, and the Dependencies section, cross-referenced against
v0.8.0 source code in `crates/ironclad-llm/src/`.

**Method:** Compared every component node in the diagram against the actual `lib.rs`
`pub mod` declarations and source files.

#### Modules confirmed present and accurately described

All 17 diagram component nodes map to actual source files:

| Diagram Module | Code File | `pub mod`? | Status |
|---|---|---|---|
| `cache.rs` | `cache.rs` (22,557 bytes) | Yes | OK |
| `router.rs` | `router.rs` (30,209 bytes) | Yes | OK |
| `circuit.rs` | `circuit.rs` (11,655 bytes) | Yes | OK |
| `dedup.rs` | `dedup.rs` (3,738 bytes) | Yes | OK |
| `format.rs` | `format.rs` (29,240 bytes) | Yes | OK |
| `tier.rs` | `tier.rs` (8,262 bytes) | Yes | OK |
| `client.rs` | `client.rs` (8,355 bytes) | Yes | OK |
| `provider.rs` | `provider.rs` (10,320 bytes) | Yes | OK |
| `embedding.rs` | `embedding.rs` (23,656 bytes) | Yes | OK |
| `uniroute.rs` | `uniroute.rs` (8,681 bytes) | Yes | OK |
| `tiered.rs` | `tiered.rs` (8,426 bytes) | Yes | OK |
| `ml_router.rs` | `ml_router.rs` (10,355 bytes) | Yes | OK |
| `cascade.rs` | `cascade.rs` (6,912 bytes) | Yes | OK |
| `capacity.rs` | `capacity.rs` (8,518 bytes) | Yes | OK |
| `accuracy.rs` | `accuracy.rs` (9,713 bytes) | Yes | OK |
| `compression.rs` | `compression.rs` (8,809 bytes) | Yes | OK |
| `oauth.rs` | `oauth.rs` (16,534 bytes) | Yes | OK |

#### LLM-1: Phantom module -- `transform.rs` in diagram but not in `pub mod`

The diagram (line 30) shows `TRANSFORM["transform.rs<br/>Request/Response<br/>Transform Pipeline"]`
and the file `transform.rs` (11,158 bytes) exists on disk. However, `lib.rs` does NOT
declare `pub mod transform;`. The file is dead code -- present in the repository but
not wired into the module tree, making it unreachable from other crates.

The doc comment in `lib.rs` line 36 references it (`//! - \`transform\` -- Request/
response transform pipeline`), so the doc comment is also stale.

**Impact:** Low. The diagram shows a module that technically exists as a file but is
not compiled into the crate. This could mislead a developer who reads the diagram and
expects to `use ironclad_llm::transform::*`.

#### LLM-2: Missing top-level struct -- `LlmService`

`lib.rs` defines `pub struct LlmService` (line 83) which is the top-level facade
composing all pipeline stages (cache, breakers, dedup, router, client, providers,
capacity, embedding). This is the main entry point for the crate and is not shown in
the diagram. The `SseChunkStream` adapter struct is also defined in `lib.rs` but not
shown.

**Impact:** Low. The LlmService is an integration struct, and the diagram focuses on
individual modules. However, it is the primary public API surface of the crate.

#### Notes

- The LLM crate diagram is **the most accurate** of all component diagrams audited
  so far. All 17 `pub mod` modules have corresponding diagram nodes.
- The pipeline flow arrows (lines 127-142) accurately reflect the request processing
  order documented in the Request Pipeline section.
- The detail subgraphs provide useful function-level documentation that matches the
  actual code.

### ironclad-c4-agent.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 11-182), the
sequence diagram (lines 186-232), and the Dependencies section, cross-referenced
against v0.8.0 source code in `crates/ironclad-agent/src/`.

**Method:** Compared every component node and detail subgraph in the diagram against
the actual `lib.rs` `pub mod` declarations and source files.

#### Result: Fully accurate

All 31 `pub mod` modules in `lib.rs` have corresponding component nodes in the
diagram. Every source file in `src/` is represented. The diagram includes detail
subgraphs for the most significant modules (loop, tools, policy, injection, prompt,
context, memory, retrieval, skills, script_runner, analyzer, orchestration, obsidian)
and grouped summaries for smaller modules.

| Code Module | Diagram Node | Status |
|---|---|---|
| `agent_loop` (loop.rs) | `LOOP` | OK |
| `tools` | `TOOLS` | OK |
| `policy` | `POLICY` | OK |
| `prompt` | `PROMPT` | OK |
| `context` | `CONTEXT` | OK |
| `injection` | `INJECTION` | OK |
| `memory` | `MEMORY` | OK |
| `retrieval` | `RETRIEVAL` | OK |
| `skills` | `SKILLS_MOD` | OK |
| `script_runner` | `SCRIPT_RUN` | OK |
| `approvals` | `APPROVALS` | OK |
| `interview` | `INTERVIEW` | OK |
| `subagents` | `SUBAGENTS` | OK |
| `analyzer` | `ANALYZER` | OK |
| `recommendations` | `RECOMMENDATIONS` | OK |
| `workspace` | `WORKSPACE` | OK |
| `knowledge` | `KNOWLEDGE` | OK |
| `discovery` | `DISCOVERY` | OK |
| `digest` | `DIGEST` | OK |
| `device` | `DEVICE` | OK |
| `governor` | `GOVERNOR` | OK |
| `manifest` | `MANIFEST` | OK |
| `services` | `SERVICES` | OK |
| `orchestration` | `ORCHESTRATION` | OK |
| `mcp` | `MCP` | OK |
| `spawning` | `SPAWNING` | OK |
| `speculative` | `SPECULATIVE` | OK |
| `typestate` | `TYPESTATE` | OK |
| `wasm` | `WASM` | OK |
| `obsidian` | `VAULT` (in subgraph) | OK |
| `obsidian_tools` | `OBS_READ/WRITE/SEARCH` (in subgraph) | OK |

#### Notes

- This is the **most comprehensive** C4 component diagram in the project. Despite
  the agent crate being the largest (31 modules, ~400K bytes total), every module is
  accounted for.
- The sequence diagram accurately represents the ReAct loop flow including all 4
  injection layers, skill matching, embedding, retrieval, and persistence.
- The dependency section correctly states ironclad-core, ironclad-db, ironclad-llm.
- No bugs filed -- diagram is current.

### ironclad-c4-wallet.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-55), the
Financial Flow sequence diagram (lines 59-92), and the Dependencies section,
cross-referenced against v0.8.0 source code in `crates/ironclad-wallet/src/`.

**Method:** Compared every component node against `lib.rs` `pub mod` declarations and
source files.

#### Modules confirmed present and accurately described

| Diagram Module | Code File | Status |
|---|---|---|
| `wallet.rs` | `wallet.rs` (27,762 bytes) | OK |
| `x402.rs` | `x402.rs` (6,300 bytes) | OK |
| `treasury.rs` | `treasury.rs` (12,322 bytes) | OK |
| `yield_engine.rs` | `yield_engine.rs` (23,975 bytes) | OK |

#### WALLET-1: money.rs shown as wallet.rs child but is a separate pub mod

The diagram places `MONEY["money.rs: Money(i64 cents)..."]` inside the `WalletDetail`
subgraph (line 19), implying it is part of `wallet.rs`. In reality, `money.rs` is
declared as a separate `pub mod money;` in `lib.rs` (line 24) and re-exported as
`pub use money::Money`. It is a peer module to wallet, not a child.

**Impact:** Low. The module IS documented in the diagram -- just misplaced
hierarchically. A reader might look for `Money` inside `wallet.rs` instead of
`money.rs`.

#### Notes

- The `WalletService` facade struct (`lib.rs` line 39) is not shown in the diagram,
  consistent with the pattern seen in `ironclad-llm` (LlmService also not shown).
- The Financial Flow sequence diagram accurately represents the yield engine flow.
- Dependencies are correctly listed.
- Overall this is a well-maintained diagram with only a minor organizational issue.

### ironclad-c4-channels.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-98), detail
subgraphs, the SharedTrait subgraph, and the Dependencies section, cross-referenced
against v0.8.0 source code in `crates/ironclad-channels/src/`.

**Method:** Compared every component node and detail subgraph in the diagram against
the actual `lib.rs` `pub mod` declarations and source files. Verified dependency claims
against Cargo.toml.

#### Modules confirmed present and accurately described

All 11 `pub mod` modules in `lib.rs` have corresponding component nodes in the diagram:

| Diagram Module | Code File | Status |
|---|---|---|
| `router.rs` | `router.rs` | OK |
| `telegram.rs` | `telegram.rs` | OK |
| `whatsapp.rs` | `whatsapp.rs` | OK |
| `web.rs` | `web.rs` | OK |
| `a2a.rs` | `a2a.rs` | OK |
| `delivery.rs` | `delivery.rs` | OK |
| `discord.rs` | `discord.rs` | OK |
| `signal.rs` | `signal.rs` | OK |
| `voice.rs` | `voice.rs` | OK |
| `email.rs` | `email.rs` | OK |
| `filter.rs` | `filter.rs` | OK |

#### CHANNELS-1: Stale "Internal crates" dependency list

The Dependencies section (line 138) states: "Internal crates: `ironclad-core` (types,
config)". However, `ironclad-channels/Cargo.toml` also declares `ironclad-db.workspace
= true`. The `delivery.rs` and `router.rs` modules use `ironclad_db::Database` for
the delivery queue. This was already identified in the container audit (BUG-012) but
the channels diagram independently repeats the error.

**Impact:** Medium. Same as BUG-012. The channels crate has a real compile-time
dependency on ironclad-db that is invisible in this diagram's Dependencies section.

#### CHANNELS-2: Stale InboundMessage / OutboundMessage field names in diagram

The SharedTrait subgraph (line 60-61) shows:
- `InboundMessage: source, text, media, platform_metadata`
- `OutboundMessage: text, attachments, reply_to, format_hints`

The actual structs in `lib.rs` have different field names:
- `InboundMessage`: `id`, `platform`, `sender_id`, `content`, `timestamp`, `metadata`
- `OutboundMessage`: `content`, `recipient_id`, `metadata`

The diagram's field names do not match the actual struct fields. This is a naming drift
that could mislead developers expecting to find fields named `source`, `text`, `media`,
`platform_metadata`, `attachments`, `reply_to`, or `format_hints`.

**Impact:** Medium. The actual struct API is significantly different from what the
diagram shows. A developer using the diagram as reference would write code against
non-existent field names.

#### Notes

- The **detail subgraphs** for each adapter (Telegram, WhatsApp, Web, A2A, Discord,
  Signal, Voice, Email, Filter) are comprehensive and provide useful internal
  documentation.
- The **A2A Handshake Sequence** diagram is well-documented and appears accurate.
- The `ChannelAdapter` trait signature in the diagram simplifies the actual signatures
  (omits `&self`, `Result<Option<...>>` wrapping, `Send + Sync` bounds). This is
  acceptable for a high-level diagram.
- The `sanitize_platform()` function and `InboundMessage::sanitize()` method added in
  v0.8.0 are not shown in the diagram, but these are minor additions.

### ironclad-c4-schedule.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-66), the Wake
Signal Flow sequence diagram (lines 70-85), and the Dependencies section,
cross-referenced against v0.8.0 source code in `crates/ironclad-schedule/src/`.

**Method:** Compared every component node, detail subgraph, and enum variant in the
diagram against the actual `lib.rs` module declarations, struct/enum definitions in
`heartbeat.rs`, `scheduler.rs`, and `tasks.rs`.

#### Modules confirmed present and accurately described

| Diagram Module | Code File | Status |
|---|---|---|
| `heartbeat.rs` | `heartbeat.rs` | OK |
| `scheduler.rs` | `scheduler.rs` | OK |
| `tasks.rs` | `tasks.rs` | OK |

#### SCHEDULE-1: Stale HeartbeatTask enum variant count

The diagram's TasksDetail subgraph (line 35) lists 7 `HeartbeatTask` variants:
`SurvivalCheck`, `UsdcMonitor`, `YieldTask`, `MemoryPrune`, `CacheEvict`,
`MetricSnapshot`, `AgentCardRefresh`. The actual enum in `tasks.rs` has **8 variants**,
adding `SessionGovernor` which invokes `ironclad_agent::governor::SessionGovernor` to
enforce session timeout/cleanup policies.

**Impact:** Low. One variant missing from the enum listing. The `SessionGovernor` task
is a meaningful addition that ties heartbeat execution to session lifecycle management.

#### SCHEDULE-2: Stale Execution subgraph -- agentTurn is legacy noop

The diagram's Execution subgraph (lines 38-50) shows `agentTurn -> inject message` as
an active execution pathway with session selection (main vs. isolated). However, in the
actual code (`lib.rs` lines 212-220), the `agent_turn_legacy` action explicitly logs a
warning and returns `("success", None)` as a noop: "legacy agentTurn cron payload
detected; treating as noop". The diagram implies agent turns are actively executed by
the scheduler, but they are not.

**Impact:** Medium. The diagram shows a feature (cron-triggered agent turns with session
selection) that was deprecated and is now a noop. This could mislead someone trying to
configure scheduled agent interactions.

#### Notes

- The **DurableScheduler** struct and its evaluation methods (`evaluate_cron`,
  `evaluate_interval`, and the at-style evaluator) match the diagram's description.
- The **HeartbeatDaemon** tier-based interval adjustment logic matches the diagram
  (LowCompute 2x, Critical 2x, Dead 10x).
- Dependencies are **correctly listed**: ironclad-core, ironclad-db, ironclad-agent,
  ironclad-wallet (all confirmed in Cargo.toml and code imports).
- The `run_cron_worker()` function in `lib.rs` is a complete implementation matching
  the Post-Execution subgraph (UPDATE cron_jobs, INSERT cron_runs).
- The Wake Signal Flow sequence diagram describes MPSC channel integration that is
  implemented in `heartbeat.rs` via the wallet and agent governor imports.

### ironclad-c4-server.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-78), the API
Route Map table (lines 84-143), the Server Module Layout table (lines 145-169), the CLI
Commands section, and the Dependencies section, cross-referenced against v0.8.0 source
code in `crates/ironclad-server/src/`.

**Method:** Compared every component node and detail subgraph in the diagram against the
actual `lib.rs` `pub mod` declarations and source file listing. Cross-referenced the
API Route Map and Module Layout tables against the actual code.

#### Modules shown in diagram (5)

| Diagram Module | Code Evidence | Status |
|---|---|---|
| `main.rs` | `main.rs` (entry point) | OK |
| `api/routes/` | `pub mod api` in lib.rs | OK |
| `dashboard.rs` | `pub mod dashboard` in lib.rs | OK |
| `ws.rs` | `pub mod ws` in lib.rs | OK |
| `cli/` | `pub mod cli` in lib.rs | OK |

#### SERVER-1: 5 pub modules missing from diagram component nodes

`lib.rs` declares 10 `pub mod` entries. The Mermaid diagram's top-level `IroncladServer`
subgraph shows only 5 nodes (main.rs, api, dashboard, ws, cli). The following 5 modules
have no diagram component node:

- `auth` -- API key authentication middleware layer (mentioned only in Server Module
  Layout table as `auth.rs`)
- `config_runtime` -- Runtime configuration hot-reload (added post-v0.5.0)
- `daemon` -- Daemon install/status/uninstall (mentioned in table as `daemon.rs`)
- `migrate` -- Migration engine, skill import/export (mentioned in table as
  `migrate/*.rs`)
- `plugins` -- Plugin registry initialization (mentioned in table as `plugins.rs`)
- `rate_limit` -- Global + per-IP rate limiting middleware (mentioned in table as
  `rate_limit.rs`)

Note: `config_runtime` is 6th missing module but may have been added after v0.5.0. The
other 5 modules (`auth`, `daemon`, `migrate`, `plugins`, `rate_limit`) are all mentioned
in the Server Module Layout table, so they are documented but not visible in the diagram.

**Impact:** Medium. The diagram's component view is incomplete -- it shows only 5 of 10
modules. Security-relevant modules (`auth`, `rate_limit`) and operational modules
(`daemon`, `plugins`, `migrate`) are invisible in the visual diagram despite being
documented in the table.

#### Notes

- The **Server Module Layout** table is comprehensive and accurately lists all source
  files with their responsibilities. It compensates for the diagram's incomplete module
  coverage.
- The **API Route Map** table (54 routes) appears thorough and matches the actual route
  structure.
- The **Bootstrap Sequence** detail subgraph (steps 1-12) matches the actual bootstrap
  flow in `lib.rs::bootstrap()`.
- The **CLI Commands** section is comprehensive.
- Dependencies are correctly listed: "All workspace crates".
- The diagram is better understood as a high-level architecture overview rather than a
  complete module inventory. The table sections provide the detail that the diagram lacks.

### ironclad-c4-browser.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-68), the Types
table, the Browser Automation Capabilities table, and the Dependencies section,
cross-referenced against v0.8.0 source code in `crates/ironclad-browser/src/`.

**Method:** Compared every component node and detail subgraph against `lib.rs` `pub mod`
declarations, struct definitions, and the `BrowserAction` enum in `actions.rs`.

#### Result: Fully accurate

All 4 `pub mod` modules in `lib.rs` have corresponding component nodes. The `Browser`
facade struct and `BrowserConfig` re-export are also correctly represented. The
`BrowserAction` enum has exactly 12 variants matching the diagram and the Capabilities
table.

| Code Module/Type | Diagram Node | Status |
|---|---|---|
| `pub mod actions` | `ACTIONS` | OK |
| `pub mod cdp` | `CDP_CLIENT` | OK |
| `pub mod manager` | `MANAGER` | OK |
| `pub mod session` | `SESSION` | OK |
| `pub struct Browser` | `BROWSER` | OK |
| `BrowserConfig` (re-export) | `CONFIG` | OK |
| 12 `BrowserAction` variants | `ActionsDetail` subgraph | OK |
| `PageInfo`, `ScreenshotResult`, `PageContent` | Types table | OK |
| `ActionResult`, `ActionExecutor` | Types table | OK |
| `SharedBrowser` (Arc alias) | Not shown | Minor (alias) |

#### Notes

- This diagram is **fully accurate**. No bugs filed.
- Dependencies correctly list ironclad-core only (confirmed by Cargo.toml).
- The `SharedBrowser` type alias (`Arc<Browser>`) defined in `lib.rs` is not shown, but
  this is a trivial alias, not a separate component.
- The detail subgraphs for Browser, BrowserManager, CdpSession, and BrowserAction
  accurately describe the public API surface.
- The Capabilities table correctly maps each action to its CDP method.

### ironclad-c4-plugin-sdk.md

**Audit scope:** All nodes in the Mermaid `flowchart TB` block (lines 10-55), the Types
table, the lifecycle description, and the Dependencies section, cross-referenced against
v0.8.0 source code in `crates/ironclad-plugin-sdk/src/`.

**Method:** Compared every component node, type definition, and detail subgraph against the
actual `lib.rs` `pub mod` declarations, struct/trait definitions, and `manifest.rs` types.
Verified dependency claims against Cargo.toml files of both this crate and claimed dependents.

#### Modules confirmed present and accurately described

All 4 `pub mod` modules in `lib.rs` have corresponding component nodes in the diagram:

| Diagram Module | Code File | Status |
|---|---|---|
| `loader.rs` | `loader.rs` | OK |
| `manifest.rs` | `manifest.rs` | OK |
| `registry.rs` | `registry.rs` | OK |
| `script.rs` | `script.rs` | OK |

#### Types confirmed present and accurate

| Diagram Type | Code Evidence | Status |
|---|---|---|
| `Plugin` trait | `lib.rs` line 54: async trait with name, version, tools, init, execute_tool, shutdown | OK |
| `ToolResult` | `lib.rs` line 47: success, output, metadata | OK |
| `PluginStatus` | `lib.rs` line 64: Loaded, Active, Disabled, Error | OK |
| `PluginManifest` | `manifest.rs`: name, version, description, author, permissions, tools | OK |

#### PLUGIN-SDK-1: Stale ToolDef field list

The diagram's `TOOL_DEF` node (line 13) shows `ToolDef` with 3 fields: `name, description,
parameters`. The actual struct in `lib.rs` (lines 32-41) has **5 fields**:

- `name: String`
- `description: String`
- `parameters: Value` (JSON Schema)
- `risk_level: RiskLevel` (defaults to `Caution` via serde default)
- `permissions: Vec<String>` (defaults to empty vec via serde default)

The `risk_level` and `permissions` fields were added post-v0.5.0 as part of the security
hardening effort. `risk_level` uses the `RiskLevel` enum from `ironclad-core` (Safe, Caution,
Dangerous, Forbidden) and defaults to `Caution` when missing from serialized input. The
`permissions` field declares what capabilities a tool requires.

**Impact:** Medium. These are security-relevant fields that affect tool execution policy.
A developer reading the diagram would not know that `ToolDef` carries risk classification
and permission declarations, which are essential for understanding the plugin security model.

#### PLUGIN-SDK-2: Stale "Depended on by" list claims ironclad-agent

The Dependencies section (line 95) states: "Depended on by: `ironclad-server` (wires
discovery, registry, and `/api/plugins/*`), `ironclad-agent` (tool registry can include
plugin tools)".

`ironclad-agent/Cargo.toml` does NOT list `ironclad-plugin-sdk` as a dependency. The agent
crate depends on `ironclad-core`, `ironclad-db`, and `ironclad-llm` only. The plugin tool
integration is server-mediated: `ironclad-server` registers plugin tools and exposes them
through its API, but the agent crate does not directly import or depend on the plugin SDK.

`ironclad-server/Cargo.toml` DOES list `ironclad-plugin-sdk` -- this part is correct.

**Impact:** Low. The incorrect "Depended on by" claim could mislead a developer into
expecting plugin SDK imports in the agent crate. The actual architecture has the server as
the integration point between plugins and the agent.

#### Notes

- The **lifecycle description** (Discovery, Registration, Initialization, Execution, Toggle)
  is comprehensive and accurate.
- The **detail subgraphs** for ManifestDetail, RegistryDetail, LoaderDetail, and ScriptDetail
  provide useful internal documentation that matches the actual code.
- The **ScriptPlugin** interpreter list in the diagram matches the code (gosh, go, sh, py,
  rb, js).
- The `PluginInfo` struct and `DiscoveredPlugin` struct documented in the Types table are
  accurate.
- Internal dependency is correctly listed as `ironclad-core` only (confirmed by Cargo.toml).

---

## Audit Conclusions

### Scope

This audit reviewed all 12 C4 architecture diagrams (2 system-level + 10 crate-level
component diagrams) against the v0.8.0 codebase. The diagrams were last updated between
v0.5.0 and v0.6.0, representing approximately 2 major versions of drift.

### Results by Diagram

| Status | Count | Diagrams |
|--------|-------|----------|
| **Accurate** | 2 | agent, browser |
| **Minor drift** | 3 | llm, wallet, schedule |
| **Drifted** | 7 | system-context, container, core, db, channels, server, plugin-sdk |

### Bug Statistics

- **Total bugs filed:** 33 (BUG-001 through BUG-033)
- **Medium severity:** 20
- **Low severity:** 13
- **Critical/High:** 0

### Top Drift Patterns

1. **Missing external system nodes (7 bugs, BUG-001--007):** The system context diagram was
   written when only Telegram, WhatsApp, and WebSocket channels existed. Discord, Signal,
   Email, Voice, Chrome/Chromium, OpenRouter, and Google Generative AI all lack diagram
   representation.

2. **Stale dependency lists and arrows (7 bugs, BUG-011--015, 027, 033):** The container
   and component diagrams have incorrect, missing, or spurious dependency arrows. The most
   significant is the spurious `channels -> agent` arrow (no such dependency exists) and the
   missing `channels -> db` arrow (real dependency, invisible in diagrams).

3. **Missing modules from component diagrams (6 bugs, BUG-016, 020--022, 031, and implicit
   in BUG-019):** As crates grew from v0.5.0 to v0.8.0, new modules were added without
   updating the corresponding diagrams. The db crate gained 4 modules (approvals,
   delivery_queue, efficiency, model_selection), the core crate gained input_capability_scan,
   and the server gained 5 visual module nodes (auth, config_runtime, daemon, migrate,
   plugins, rate_limit).

4. **Stale labels and field counts (8 bugs, BUG-008--010, 017--018, 028--029, 032):**
   Enum variant counts, struct field lists, and descriptive labels fell out of date as code
   evolved. The most impactful are the ChannelsConfig fields (BUG-018, shows 2 of 8+
   fields), the InboundMessage/OutboundMessage fields (BUG-028, completely different field
   names), and ToolDef fields (BUG-032, missing security-relevant risk_level and
   permissions).

5. **Behavioral drift (1 bug, BUG-030):** The schedule diagram shows `agentTurn` as an
   active execution pathway, but the code treats it as a deprecated noop.

### Crates Most Affected

| Crate | Bug Count | Most Significant Issue |
|-------|-----------|----------------------|
| docs (system-context) | 10 | 7 missing external system nodes |
| docs (container) | 5 | Spurious and missing dependency arrows |
| ironclad-core | 4 | 18+ missing config structs |
| ironclad-db | 4 | efficiency module (25K bytes) invisible |
| ironclad-server | 1 | 5 of 10 modules missing from visual diagram |
| ironclad-channels | 2 | Struct field names completely wrong |
| ironclad-plugin-sdk | 2 | Security-relevant ToolDef fields missing |
| ironclad-schedule | 2 | Deprecated feature shown as active |
| ironclad-llm | 2 | Dead code file shown in diagram |
| ironclad-wallet | 1 | Module hierarchy misplacement |
| ironclad-agent | 0 | -- |
| ironclad-browser | 0 | -- |

### Recommendations

1. **Immediate (before v0.8.1):** Fix the 3 most misleading issues:
   - BUG-011: Remove the spurious `channels -> agent` arrow in the container diagram
   - BUG-028: Update InboundMessage/OutboundMessage field names in the channels diagram
   - BUG-030: Mark `agentTurn` as deprecated/noop in the schedule diagram

2. **Short-term (v0.9.0 planning):** Update all 7 "Drifted" diagrams to reflect v0.8.0:
   - Add missing external system nodes to system-context diagram
   - Fix dependency arrows in container diagram
   - Add missing modules to core, db, server, and plugin-sdk diagrams

3. **Process improvement:** Consider adding a diagram freshness check to the release
   checklist. Each diagram already has a version tag in its header -- a CI lint could flag
   diagrams whose version is more than 1 minor release behind the crate version.

### Diagrams That Need No Changes

- **ironclad-c4-agent.md** -- Fully accurate despite being the largest diagram (31 modules).
  This is the gold standard for diagram maintenance.
- **ironclad-c4-browser.md** -- Fully accurate. All 4 modules, 12 action variants, and
  types match perfectly.

### ironclad-dataflow.md

**Audit scope:** All 20 dataflow diagrams (numbered 0-19) in the Mermaid flowcharts,
cross-referenced against v0.8.0 source code across all workspace crates. Diagrams were
last updated at v0.5.0-v0.6.0.

**Method:** For each diagram, traced the described data path through actual function
calls, struct definitions, and module boundaries. Verified node labels, function names,
counts, and behavioral claims against the live codebase.

#### Diagrams confirmed accurate (no drift)

| Diagram | Description | Status |
|---|---|---|
| 0. Runtime Config Reload | `config_runtime.rs` flow: parse -> validate -> backup -> atomic write -> apply -> sync router & A2A -> deferred hints | OK |
| 2. Semantic Cache | 3-level lookup order (L1 exact -> L3 tool TTL -> L2 semantic n-gram) matches `cache.rs` `lookup()` at line 190 | OK |
| 3. Heuristic Model Router | `HeuristicBackend::classify_complexity` formula, `select_for_complexity` behavior, `ml_router.rs` alternative backend all confirmed | OK |
| 4. Memory Lifecycle | 5-tier memory (working, episodic, semantic, procedural, relationship), budget percentages (30/25/20/15/10) match `MemoryConfig` defaults, FTS5 sync confirmed | OK |
| 5. Zero-Trust A2A | Challenge-response, ECDH session keys, AES-256-GCM encryption, trust score management all confirmed in `ironclad-channels/a2a.rs` | OK |
| 6. Multi-Layer Injection Defense | L1 (gatekeeping), L2 (HMAC boundaries), L3 (policy authority gate), L4 (`scan_output` with NFKC+homoglyph+regex) all confirmed | OK |
| 7. Financial + Yield Engine | SurvivalTier calculation, x402 EIP-3009 payment, Aave deposit/withdraw, treasury policy checks all confirmed | OK |
| 9. Skill Execution | Dual-format (TOML structured + MD instruction), SHA-256 hashing, trigger matching, script sandbox with env stripping all confirmed | OK |
| 10. Approval Workflow | `ApprovalManager` in `ironclad-agent/src/approvals.rs`, EventBus notification, oneshot pause/resume, DB persistence all confirmed | OK |
| 11. Browser Tool Execution | `BrowserManager`, `CdpSession`, action dispatch (navigate/click/type/screenshot), idle eviction all confirmed in `ironclad-browser/` | OK |
| 15. Addressability Filter | FilterChain with OR logic across MentionFilter, ReplyFilter, ConversationFilter confirmed in `ironclad-channels/` | OK |
| 17. Plugin SDK Execution | Discovery, manifest parsing, ToolDef registration with prefix, permission checks, sandboxed execution all confirmed | OK |
| 18. OAuth & Credential Resolution | `OAuthManager` in `ironclad-llm/src/oauth.rs`, multi-strategy resolution (env -> keystore -> OAuth refresh), token caching all confirmed | OK |
| 19. Channel Adapter Lifecycle | Webhook/polling modes, InboundMessage parsing, addressability filter, agent dispatch, rate-limited delivery, health reconnect all confirmed | OK |

#### Drift findings

##### DF-1: Diagram 1 (Primary Request Dataflow) -- version label says v0.6.0 but references v0.8.0 constructs

The diagram header comment says `version: 0.6.0` but the preamble text references
"Capacity-aware model selection" and "Session rotation now evaluates session.reset_schedule
cron expressions" which are v0.8.0 features. The Install/Setup subgraph (Apertus with
SGLang-first host recommendation) was also added post-v0.6.0. The version comment is stale.

**Impact:** Low. The diagram content is more current than its version tag suggests.

##### DF-2: Diagram 1 -- Provider list in LLM Pipeline says "Anthropic / Google / Moonshot / OpenAI / Ollama"

The actual `bundled_providers.toml` includes 11 providers: Anthropic, OpenAI, Google,
Moonshot, DeepSeek, Groq, SGLang, vLLM, Docker Model Runner, llama-cpp, and OpenRouter.
The diagram lists only 5 representative providers.

**Impact:** Low. This is a visual simplification, but the list diverges from the 11
bundled providers in v0.8.0.

##### DF-3: Diagram 8 (Cron + Heartbeat Scheduling) -- behavioral drift: `agentTurn` payload shown as active execution path

The diagram (section iii "Job Execution") shows `agentTurn` as the primary payload kind
with session selection (main vs isolated) and a full `Run ReAct loop turn` execution.
In v0.8.0, `ironclad-schedule/src/lib.rs` line 212-219 treats `agent_turn_legacy` as a
noop with a warning log: "legacy agentTurn cron payload detected; treating as noop".
The `agentTurn` kind is mapped to `agent_turn_legacy` at line 230.

**Impact:** Medium. The diagram implies a feature that was deprecated. A reader would
believe scheduled agent turns are functional, but they are not.

##### DF-4: Diagram 8 -- stale label: "register 6 default tasks" implied by heartbeat tasks list

The diagram's tick loop and evaluation subgraphs assume the default task set. In v0.8.0,
`default_tasks()` in `heartbeat.rs` line 76-87 returns 8 tasks: SurvivalCheck,
UsdcMonitor, YieldTask, MemoryPrune, CacheEvict, MetricSnapshot, AgentCardRefresh,
SessionGovernor. The `SessionGovernor` and `AgentCardRefresh` tasks were added post-v0.5.0.

**Impact:** Low. The diagram does not explicitly state a count, but the cross-reference
with the schedule C4 diagram (which states 7 variants) creates inconsistency. Actual
count is 8.

##### DF-5: Diagram 12 (Context Assembly) -- naming drift: `ContextBuilder`, `BudgetManager`, `ComplexityClassifier`, `SnapshotDB` not found as named types

The diagram references `ContextBuilder`, `BudgetManager`, `ComplexityClassifier`, and
`SnapshotDB` as distinct struct/module names. In v0.8.0:
- Context assembly is in `ironclad-agent/context.rs` via `build_context()` function,
  not a `ContextBuilder` struct
- Budget management is via `MemoryBudgetManager` (not `BudgetManager`)
- Complexity classification is via `classify_complexity()` free function in
  `ironclad-llm/router.rs` (not a `ComplexityClassifier` struct)
- Context snapshots go to the `context_snapshots` table but there is no `SnapshotDB`
  type

**Impact:** Low. The diagram describes conceptual components that map to real functions,
but uses aspirational type names that do not exist in code. A reader searching for these
types would not find them.

##### DF-6: Diagram 13 (Response Transform Pipeline) -- entire pipeline references dead code

The diagram describes a 4-stage response transform pipeline using `ReasoningExtractor`,
`FormatNormalizer`, `ContentGuard`, and `PII leak scan`. These concepts exist only in
`ironclad-llm/src/transform.rs` (11,158 bytes), which is NOT declared as `pub mod
transform` in `lib.rs`. This file is dead code -- unreachable from any other crate. This
was already identified as BUG-024 in the C4 audit.

The actual response processing in v0.8.0 happens inline in `agent.rs` (the
`infer_with_fallback` and `agent_message_stream` functions) without a dedicated transform
pipeline.

**Impact:** Medium. The diagram describes a feature that was designed but never wired
into the live code path. Readers would believe a PII-scan and reasoning-extraction
pipeline exists in the hot path, but it does not.

##### DF-7: Diagram 14 (Streaming LLM) -- behavioral accuracy confirmed with one exception

The streaming diagram correctly shows SSE chunk processing, accumulator pattern, EventBus
publish to WebSocket subscribers, and circuit breaker update on finalization. The error
handling section showing fallback on stream failure is also confirmed.

The diagram does NOT show the in-flight deduplication step that the actual code performs
(lines 1834-1854 of `agent.rs`). This is a missing step, not a behavioral mismatch.

**Impact:** Low. The dedup check is a safety rail that exists in code but is absent from
the diagram.

##### DF-8: Diagram 16 (Context Observatory) -- naming drift: `EfficiencyEngine` struct name does not exist

The diagram references `EfficiencyEngine` as a named struct that computes composite
efficiency scores. In v0.8.0, the efficiency computation is done by the
`compute_efficiency()` free function in `ironclad-db/src/efficiency.rs` (line 208), not
by an `EfficiencyEngine` struct. The `EfficiencyReport` struct (line 94) is the return
type. The diagram's `GradingSystem` and `RecommendationEngine` are similarly conceptual
names that map to functions in `recommendations.rs` rather than dedicated structs.

**Impact:** Low. Same pattern as DF-5: conceptual names in diagram that map to real
functions but do not exist as named types.

##### DF-9: Diagram 1 -- bootstrap step count: code has STEPS=12, not 13

The `cmd_serve()` function in `main.rs` line 1388 declares `const STEPS: u32 = 12` and
the step calls go from step 1 through step 12. The dataflow diagram itself does not
explicitly claim "13 steps" (that claim is in the sequence diagrams), but the Primary
Request Dataflow diagram's preamble references the bootstrap implicitly. This is noted
for cross-reference with the sequence diagram audit.

**Impact:** Informational (for cross-reference with Task 15).

#### Summary of dataflow drift by category

| Category | Count | Severity | Key Issues |
|---|---|---|---|
| Behavioral mismatch | 4 | Medium | agentTurn noop (DF-3), dead transform pipeline (DF-6), missing dedup in stream (DF-7), stale version tag (DF-1) |
| Naming/label drift | 5 | Low | Provider list (DF-2), task count (DF-4), conceptual type names (DF-5, DF-8), step count (DF-9) |
| Structural drift | 0 | -- | All 20 diagrams structurally reflect the actual data paths |

#### Dataflow diagrams that are fully accurate

14 of 20 diagrams are accurate with no meaningful drift: diagrams 0, 2, 3, 4, 5, 6, 7,
9, 10, 11, 15, 17, 18, 19. This is a notably better accuracy rate than the C4 component
diagrams, suggesting the dataflow diagrams were maintained more recently or describe more
stable subsystems.

---

### ironclad-sequences.md

**Audit scope:** All 13 cross-crate sequence diagrams plus the cross-reference matrix
(lines 1-918), cross-referenced against v0.8.0 source code across all crates. Version
tags in the diagrams claim v0.5.0; the document has not been updated for v0.8.0.

#### Sequence diagrams confirmed accurate

7 of 13 sequence diagrams are accurate with no meaningful drift:

| # | Diagram | Status |
|---|---------|--------|
| 1 | End-to-End Request Lifecycle | Accurate -- participant ordering, function labels, injection layers, cache levels, memory retrieval, prompt HMAC, and ReAct loop all match v0.8.0 code |
| 2 | Cache-Augmented Inference Pipeline | Accurate -- SemanticCache L1/L2/L3 levels, n-gram cosine fallback, tool-TTL, and provider deduplication match `cache.rs` |
| 3 | x402 Payment-Gated Inference | Accurate -- X402Handler, parse_payment_requirements, build_payment_header, wallet sign_message all confirmed in `x402.rs` |
| 5 | Injection Attack Blocked | Accurate -- L1-L4 layers, ThreatScore thresholds, scan_output L4, HMAC trust boundaries all match `injection.rs` |
| 6 | Skill-Triggered Script Execution | Accurate -- skill matching, policy evaluation, sandboxed execution confirmed |
| 8 | Approval Workflow: Gated Tool Execution | Accurate -- ApprovalManager, oneshot channel, EventBus, timeout handling all confirmed in `approvals.rs` |
| 13 | Browser Tool: CDP Session Lifecycle | Accurate -- BrowserManager, CdpSession, Target.createTarget, idle timeout all confirmed in `ironclad-browser/` |

#### SQ-1: Title and step count mismatch -- "13-Step Bootstrap" is 12 steps

**Diagram 4** (line 285) is titled "13-Step Bootstrap Sequence" but `main.rs` line 1388
declares `const STEPS: u32 = 12` and the step calls go from `step(1)` through
`step(12)`. The diagram itself actually shows steps 1-12 plus "Step 12: Await shutdown"
which is correct content-wise, but the title claims 13. The cross-reference matrix
(line 899) also says "13-Step Bootstrap".

**Impact:** Medium -- the title creates a search/reference mismatch. Developers looking
for "step 13" will find nothing.

#### SQ-2: Stale sub-struct count -- "14 sub-structs" vs ~30 actual

**Diagram 4** line 307 says `parse all 14 sub-structs, validate budget pct sum=100`.
The actual `IroncladConfig` struct in `config.rs` lines 101-160 has approximately 30
sub-struct fields. The budget pct sum=100 validation IS correct (confirmed at config.rs
lines 374-382), but the sub-struct count is stale since v0.5.0.

**Impact:** Low -- the count is informational; the validation behavior is correct.

#### SQ-3: Stale table count -- "28 tables" vs 34 actual

**Diagram 4** line 313 says `run_migrations() (28 tables incl. indexes + FTS5)` but
`schema.rs` line 587 comments say 34 tables total (30 regular + FTS5 + sub_agents +
hippocampus + turn_feedback + context_snapshots + model_selection_events). The count
was accurate at v0.5.0 but grew significantly since.

**Impact:** Low -- informational count; migration behavior is correct.

#### SQ-4: Stale tool count -- "10 categories" vs ~8 individual tools

**Diagram 4** line 339 says `register built-in tools (10 categories)`. The actual
`ToolRegistry` in `lib.rs` lines 473-491 registers approximately 8 individual tools
plus optional Obsidian tools. The number "10 categories" does not match any
observable grouping in the code.

**Impact:** Low -- informational label; tool registration behavior is correct.

#### SQ-5: Wrong scheduler struct name -- "DurableScheduler::start" vs HeartbeatDaemon::new

**Diagram 4** line 349 says `DurableScheduler::start(config, db, agent)`. The actual
code at `main.rs` line 1543 uses `HeartbeatDaemon::new(60_000)`. `DurableScheduler`
does exist as a struct in `scheduler.rs` line 7, but it is a stateless utility for
cron/interval evaluation, not the daemon. The daemon is `HeartbeatDaemon`. The diagram
also implies `DurableScheduler::start()` takes `(config, db, agent)` args; in reality
`HeartbeatDaemon::new()` takes only `interval_ms`.

**Impact:** Medium -- developers trying to trace startup would look for a
`DurableScheduler::start()` call that does not exist.

#### SQ-6: Stale default task count -- "6 default tasks" vs 8 actual

**Diagram 4** line 351 says `register 6 default tasks`. The actual `default_tasks()` in
`heartbeat.rs` lines 76-87 returns 8 tasks: SurvivalCheck, UsdcMonitor, YieldTask,
MemoryPrune, CacheEvict, MetricSnapshot, AgentCardRefresh, SessionGovernor. Same root
cause as BUG-029 and BUG-037.

**Impact:** Low -- informational count; heartbeat behavior is correct.

#### SQ-7: Massively stale route count -- "42 REST API routes" vs ~98 actual

**Diagram 4** line 366 says `mount 42 REST API routes + dashboard SPA + WebSocket
upgrade`. The actual router in `routes/mod.rs` lines 293-456 mounts 98 `.route()` calls
covering 95 distinct `/api/` paths plus `/`, `/.well-known/agent.json`, and a Telegram
webhook. The count has more than doubled since v0.5.0 with additions for approvals,
interviews, runtime surfaces, MCP, devices, subagents, and more.

**Impact:** Low -- informational count; routing behavior is correct.

#### SQ-8: agentTurn shown as active execution -- is legacy noop

**Diagram 7** (Cron Lease Acquisition) lines 571-574 show the `agentTurn` path as
active execution: `inject message -> run ReAct loop turn -> turn result`. The actual
code in `schedule/lib.rs` lines 212-219 treats `agent_turn_legacy` as a noop that logs
a warning. Same root cause as BUG-030 and BUG-036.

**Impact:** Medium -- diagram suggests a working feature that is deprecated.

#### SQ-9: Streaming diagram uses wrong endpoint -- "/api/chat" vs "/api/agent/message/stream"

**Diagram 9** (Streaming Response) line 659 shows `POST /api/chat (stream: true)`.
The actual streaming endpoint is `POST /api/agent/message/stream` (routes/mod.rs
line 364). There is no `/api/chat` endpoint. The endpoint name and streaming model
(dedicated SSE endpoint vs query parameter) are both wrong.

**Impact:** Medium -- developers would look for a non-existent endpoint.

#### SQ-10: Context Observatory references phantom types

**Diagram 10** (Context Observatory) references `TransformPipeline` (line 695/702),
`TurnRecorder` (line 707), and `Observatory Analyzer` (line 698) as distinct
participants. None of these exist as structs in the codebase:
- `TransformPipeline`: dead code in `transform.rs`, not in pub mod list
- `TurnRecorder`: no matches anywhere in codebase
- `Observatory Analyzer`: not a real type; efficiency computation is via
  `compute_efficiency()` free function in `efficiency.rs`

The tables `turn_observations` and `observatory_grades` referenced in the diagram
(lines 711, 726) do NOT exist in `schema.rs`. Actual tables are `turn_feedback` and
the efficiency metrics are computed on-the-fly from existing tables.

**Impact:** Medium -- diagram describes an entire subsystem architecture that does
not exist in the form shown.

#### SQ-11: Outcome Grading uses wrong table name -- "outcome_feedback" vs "turn_feedback"

**Diagram 11** (Outcome Grading) lines 754, 758, 762 reference `INSERT outcome_feedback`.
The actual table name is `turn_feedback` (schema.rs line 369). The `outcome_feedback`
table does not exist. The cross-reference matrix (line 906) also references
`outcome_feedback`. Additionally, `MetricEngine` (line 747) does not exist as a struct;
metrics aggregation uses `compute_efficiency()` free function.

**Impact:** Medium -- wrong table name would cause confusion during implementation
or debugging.

#### SQ-12: Network Binding TLS section describes unimplemented feature

**Diagram 12** (Network Binding) lines 806-826 describe a complete TLS configuration
flow: `TlsAcceptor`, rustls `ServerConfig`, ALPN negotiation, TLS 1.3 handshake, and
certificate loading. None of this exists in the codebase:
- No `TlsConfig`, `cert_path`, or `key_path` in `IroncladConfig`
- No `TlsAcceptor` or rustls server config in `ironclad-server`
- No `InterfaceResolver` struct (line 787) exists
- The actual server uses plain `axum::serve(listener, app)` with
  `TcpListener::bind()` (main.rs lines 1592-1646)
- The only rustls usage is in reqwest for outbound HTTPS client connections

The "development mode" plain-HTTP branch (lines 821-825) is the ONLY mode that
actually exists. The entire TLS section is aspirational.

**Impact:** Medium -- diagram describes a security feature (TLS termination) that
does not exist, which could mislead security auditors into thinking the server
supports TLS natively.

#### Summary of sequence diagram drift by category

| Category | Count | Severity | Key Issues |
|---|---|---|---|
| Unimplemented diagram | 1 | Medium | TLS Network Binding (SQ-12) |
| Behavioral mismatch | 3 | Medium | agentTurn noop (SQ-8), wrong endpoint (SQ-9), phantom types in Observatory (SQ-10) |
| Wrong table/type names | 4 | Medium | outcome_feedback (SQ-11), TransformPipeline/TurnRecorder/MetricEngine (SQ-10), DurableScheduler::start (SQ-5) |
| Stale counts/labels | 5 | Low | 13 vs 12 steps (SQ-1), 14 vs 30 sub-structs (SQ-2), 28 vs 34 tables (SQ-3), 10 vs 8 tools (SQ-4), 6 vs 8 tasks (SQ-6), 42 vs 98 routes (SQ-7) |

#### Sequence diagrams that are fully accurate

7 of 13 diagrams are accurate: diagrams 1, 2, 3, 5, 6, 8, 13. The accurate diagrams
tend to describe well-isolated subsystems (cache, wallet, injection, approval, browser)
while the drifted diagrams describe system-wide orchestration (bootstrap, scheduling,
observatory) where v0.5.0-to-v0.8.0 growth was concentrated.

---

### circuit-breaker-audit.md

**Audit scope:** 1 flowchart (Current Runtime Dataflow), 3 sequence diagrams (Transient
Failure Recovery, Credit Error Path, Bounded Fallback Policy), and 5 audit findings,
cross-referenced against v0.8.0 code in `circuit.rs`, `router.rs`, `agent.rs`,
`interview.rs`, and `cli/admin/misc.rs`.

#### Diagrams confirmed accurate

The core circuit breaker state machine (Closed/Open/HalfOpen) is accurately represented:
- `CircuitState` enum with 3 variants confirmed in `circuit.rs` lines 6-11
- `credit_tripped` sticky behavior confirmed (line 24, 48-49, 146-151)
- Cooldown-based effective HalfOpen transition confirmed (lines 51-54)
- Exponential backoff on HalfOpen failure confirmed (lines 126-127)
- `record_success`, `record_failure`, `record_credit_error` methods confirmed
- `reset()` clears all state including `credit_tripped` and `preemptive_half_open`
- Default thresholds: threshold=3, window=60s, cooldown=60s, max_cooldown=900s

Sequence diagram 1 (Transient Failure Recovery) is accurate.
Sequence diagram 2 (Credit Error Path) is accurate.
Sequence diagram 3 (Bounded Fallback Policy) is accurate.

#### CB-1: Finding #1 resolved -- CLI reset endpoint now correct

The audit document's finding #1 ("CLI currently posts to `/api/breaker/reset` (no
provider path)") has been **fixed** in v0.8.0. The CLI `cmd_circuit_reset()` in
`cli/admin/misc.rs` lines 555-604 now:
1. Fetches `GET /api/breaker/status` to get the provider list
2. Iterates each provider and posts to `POST /api/breaker/reset/{provider}` (line 591)

**Status:** Fixed. Audit finding is now stale.

#### CB-2: Finding #2 resolved -- Streaming path now uses fallback loop

The audit document's finding #2 ("Streaming path bypasses fallback loop") has been
**fixed** in v0.8.0. The `agent_message_stream()` in `agent.rs` lines 2069-2094 now:
1. Builds the same `fallback_candidates()` list as non-stream inference
2. Iterates candidates with `breakers.is_blocked()` check (line 2085)
3. Records `record_success` on successful stream (line 2137)
4. Continues to next candidate on failure

The comment at line 2069 explicitly states "Use the same fallback surface as
non-stream inference."

**Status:** Fixed. Audit finding is now stale.

#### CB-3: Finding #3 resolved -- Interview path now uses fallback

The audit document's finding #3 ("Interview path bypasses routing/fallback/breaker
accounting") has been **fixed** in v0.8.0. The `interview_turn()` in `interview.rs`
line 93 now calls `select_routed_model()` for model selection, and line 106 calls
`infer_content_with_fallback()` which delegates to `infer_with_fallback()`.

**Status:** Fixed. Audit finding is now stale.

#### CB-4: Findings #4 and #5 still valid

Finding #4 (Runtime control-plane drift risk): `sync_runtime()` exists
(config_runtime.rs line 205, admin.rs line 1763) but formal verification that ALL
mutation paths trigger synchronization is not tested.

Finding #5 (Insufficient integration coverage for breaker lifecycle): Integration
tests exist in `ironclad-tests/src/router_integration.rs` but do not yet cover the
full API-level breaker lifecycle end-to-end.

**Status:** Still valid. These are testing/verification gaps, not code defects.

#### CB-5: Stale dataflow diagram -- streaming path now uses fallback

The flowchart (lines 48-49) shows `A2 -> E1["single model/provider resolution"] ->
E2["stream_to_provider()"]` as a separate non-fallback path. This is now stale: the
streaming path uses the same candidate loop as `infer_with_fallback()`.

**Impact:** Medium -- diagram shows an architecture that no longer exists.

#### CB-6: Missing preemptive_half_open in diagrams

The `preemptive_half_open` field (circuit.rs line 27) allows capacity pressure to
soft-transition a Closed breaker to effective HalfOpen (line 58). This is set by
`set_capacity_pressure()` (line 186) and deprioritizes a provider before hard
failures occur. None of the three sequence diagrams show this path. It is a fourth
state transition pattern alongside: threshold trip, cooldown recovery, and credit trip.

**Impact:** Low -- the feature is a performance optimization, not a correctness issue.

#### CB-7: Dead config field -- credit_cooldown_seconds

The `CircuitBreakerConfig` struct includes `credit_cooldown_seconds` (default 300s)
in config.rs line 751, but this field is **never read** by `circuit.rs`. The
`CircuitBreaker::new()` constructor only uses `cooldown_seconds` and
`max_cooldown_seconds`. Credit-tripped breakers use `credit_tripped` boolean to
prevent auto-recovery entirely, so a separate cooldown is moot.

**Impact:** Low -- dead config field; no behavioral consequence.

#### Summary of circuit-breaker audit drift

| Category | Count | Severity | Details |
|---|---|---|---|
| Fixed findings | 3 | -- | CLI reset (CB-1), stream fallback (CB-2), interview fallback (CB-3) |
| Stale diagram path | 1 | Medium | Streaming shown as single-provider (CB-5) |
| Missing behavior | 1 | Low | preemptive_half_open not in diagrams (CB-6) |
| Dead config | 1 | Low | credit_cooldown_seconds never consumed (CB-7) |
| Still valid findings | 2 | Low | Sync verification gap (CB-4), integration test gap (CB-4) |

---

### router-audit.md

**Audit scope:** 1 flowchart (Current Router Dataflow), 3 sequence diagrams
(Complexity-Aware, Cost-Aware, Bounded Fallback), and 4 audit findings,
cross-referenced against v0.8.0 code in `router.rs`, `agent.rs`, `interview.rs`,
and `config.rs`.

#### Diagrams confirmed accurate

The core router behavior is accurately represented:
- Three selection modes: `primary`, `round-robin`, complexity-aware default (router.rs)
- `select_for_complexity()` with override check, breaker filtering, capacity filtering
  (router.rs line 106)
- `select_cheapest_qualified()` with breaker/capacity pruning before cost selection
  (router.rs line 178)
- `set_override()` / `clear_override()` short-circuit confirmed (lines 81, 86)
- `sync_runtime()` for config-to-router synchronization confirmed (line 277)
- `local_first` threshold check confirmed in `select_for_complexity()`

Sequence diagram 1 (Complexity-Aware Routing) is accurate.
Sequence diagram 2 (Cost-Aware Routing) is accurate.
Sequence diagram 3 (Bounded Fallback Execution) is accurate.

#### RT-1: Finding #1 partially resolved -- stream and interview paths now use fallback

The audit document's finding #1 ("Route-family inconsistency") noted that
`agent_message_stream()` and `interview_turn()` use single-provider calls. Both
paths have been **fixed** in v0.8.0:
- Streaming: `agent.rs` lines 2069-2094 uses candidate loop with breaker checks
- Interview: `interview.rs` line 106 uses `infer_content_with_fallback()`

**Status:** Fixed. All four entry paths (E1-E4) now use bounded fallback.

#### RT-2: Stale dataflow diagram -- E2 and E4 paths

The flowchart shows `E2 -> S1` and `E2 -> X2` where X2 is "single provider call
(stream/interview paths)". This dual path is stale: E2 (streaming) now goes through
the same `S1 -> S2 -> ... -> X1` path as E1. Similarly, E4 (interview) now goes
through `select_routed_model` and `infer_content_with_fallback` rather than X2.
The `X2` node should be removed entirely.

**Impact:** Medium -- diagram shows a divergent architecture that no longer exists.

#### RT-3: Findings #2, #3, #4 still valid

Finding #2 (Config-vs-router drift risk): `sync_runtime()` exists but no formal
proof that all mutation paths trigger it. The `config_runtime::apply_runtime_config()`
and `admin` routes both call it, but any new mutation path could miss it.

Finding #3 (Override observability gap): `set_override` is callable via chat command;
no dedicated audit event is emitted when an override is set or cleared.

Finding #4 (Unused `model_overrides` config map): `models.model_overrides` field
exists in `config.rs` line 563 with `ModelOverride` struct at line 706, but NO code
in `router.rs` or `agent.rs` reads this field. The runtime `set_override()` uses a
separate `override_model: Option<String>` field on the router struct. The config-level
`model_overrides` HashMap is dead code.

**Status:** All three still valid.

#### Summary of router audit drift

| Category | Count | Severity | Details |
|---|---|---|---|
| Fixed findings | 1 | -- | Route-family inconsistency resolved (RT-1) |
| Stale diagram path | 1 | Medium | E2/E4 -> X2 single-provider path (RT-2) |
| Still valid findings | 3 | Low-Medium | Sync gap (RT-3), observability (RT-3), dead config (RT-3) |
