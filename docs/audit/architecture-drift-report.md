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
