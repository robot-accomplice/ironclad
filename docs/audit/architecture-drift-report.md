# Architecture Drift Report — v0.8.0

<!-- last_updated: 2026-02-26, version: 0.8.0 -->

Diagrams audited against v0.8.0 code. Diagrams were last updated at v0.5.0-v0.6.0.

## Summary

| File | Diagrams | Structural | Relationship | Behavioral | Naming | Status |
|------|----------|-----------|-------------|-----------|--------|--------|
| `ironclad-c4-system-context.md` | 1 (C4Context) | 7 missing nodes, 1 stale node | 2 relationship-label gaps | 0 | 1 vague label | Drifted |

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
