# Architecture Drift Report — v0.8.0

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Diagrams audited against v0.8.0 code. Diagrams were last updated at v0.5.0-v0.6.0.

## Summary

| File | Diagrams | Structural | Relationship | Behavioral | Naming | Status |
|------|----------|-----------|-------------|-----------|--------|--------|
| `ironclad-c4-system-context.md` | 1 (C4Context) | 7 missing nodes, 1 stale node | 2 relationship-label gaps | 0 | 1 vague label | Drifted |
| `ironclad-c4-container.md` | 1 (C4Container) | 0 | 2 spurious arrows, 1 missing arrow, 1 missing table dep, 8 missing `core` arrows, 6 missing `server` arrows | 0 | 0 | Drifted |

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
