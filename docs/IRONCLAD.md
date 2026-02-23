# Ironclad

A single-binary autonomous agent runtime written in Rust, designed for maximum efficiency, minimum operational cost, and self-sustainability.

---

## Architecture

Ironclad compiles to a single static binary. Every subsystem -- LLM routing, agent loop, memory, scheduling, wallet, channels, dashboard -- runs in one OS process on one async runtime (tokio), sharing one SQLite database.

| Metric | Value |
| --- | --- |
| Language | Rust (edition 2024) |
| Workspace crates | 11 |
| Source files | 117 |
| Lines of code | ~32,000 |
| Test count | ~1,271 unit + integration |
| SQLite tables | 28 |
| Architecture docs | 17 (C4 + dataflow + sequence diagrams) |

### Crate Map

```text
ironclad-core        Shared types, config, error definitions, personality system
ironclad-db          SQLite persistence (28 tables, FTS5, WAL mode, BLOB embeddings, ANN index, cache persistence)
ironclad-llm         LLM client pipeline, format translation, routing, caching (persistent), circuit breakers, embedding client
ironclad-agent       ReAct loop, tool system, policy engine, injection defense, hybrid RAG, skills, subagents
ironclad-wallet      Ethereum wallet, x402 payments, treasury policy, yield engine
ironclad-schedule    Cron scheduler, heartbeat daemon, built-in tasks
ironclad-channels    Telegram, WhatsApp, Discord, WebSocket, A2A protocol
ironclad-server      HTTP server (axum), REST API, CLI, dashboard, auth, migration
ironclad-plugin-sdk  Plugin trait, script runner, manifest parser, plugin registry
ironclad-browser     Headless browser automation via CDP
ironclad-tests       Integration test suite
```

---

## Feature Summary

### LLM Client Pipeline

- **Model-agnostic proxy** -- provider configuration is fully externalized in TOML; adding a new provider requires zero code changes
- **Bundled provider configs** for OpenAI, Anthropic, Google, Ollama, OpenRouter (shipped as `bundled_providers.toml`)
- **Format translation** -- typed Rust enums with `From<T>` implementations for 4 API formats (OpenAI Chat, OpenAI Responses, Anthropic Messages, Google GenerativeAI); 12+ translation pairs
- **Circuit breaker** per provider (Closed/Open/HalfOpen states, exponential backoff, rate/credit/timeout trips)
- **In-flight deduplication** -- SHA-256 fingerprinting prevents duplicate concurrent requests
- **Tier-based prompt adaptation** -- T1 (condensed), T2 (preamble + reorder), T3/T4 (passthrough + Anthropic cache_control)
- **Heuristic model router** -- rule-based fallback chain + heuristic complexity classifier for routing
- **3-level semantic cache** -- L1 exact hash, L2 embedding cosine similarity, L3 deterministic tool TTL; backed by SQLite persistence (loaded on boot, flushed every 5 min)
- **Multi-provider embedding client** -- supports OpenAI, Ollama, and Google embedding APIs with automatic format translation; falls back to deterministic char n-gram hashing when no provider is configured
- **Persistent connection pool** -- single `reqwest::Client` with HTTP/2 multiplexing per provider
- **x402 payment protocol** -- automatic payment-gated inference (402 -> sign EIP-3009 -> retry)

### Agent Core

- **ReAct state machine** -- Think -> Act -> Observe -> Persist cycle with idle/loop detection
- **Tool system** -- trait-based plugin architecture with 10 tool categories
- **Policy engine** -- 6 built-in rules (authority, command safety, financial, path protection, rate limit, validation)
- **4-layer prompt injection defense**:
  - L1: regex pattern detection (instruction override, authority claims, encoding evasion, multi-language, financial manipulation) producing a 0.0-1.0 ThreatScore
  - L2: HMAC-tagged trust boundary markers in system prompt (unforgeable by injected content)
  - L3: output validation (authority-based tool call filtering, peer message restrictions)
  - L4: output scanning (NFKC normalization, encoding decode, homoglyph folding, regex pattern detection)
- **Progressive context loading** -- 4 complexity levels (L0 ~2K, L1 ~4K, L2 ~8K, L3 ~16K tokens)
- **Subagent framework** -- spawn child agents with isolated tool registries and policy overrides
- **Human-in-the-loop approvals** -- configurable approval gates for high-risk tool calls

### Memory System

- **5-tier unified memory** in a single SQLite database:
  - Working (session-scoped)
  - Episodic (events with importance scoring)
  - Semantic (categorized facts)
  - Procedural (tool usage patterns and outcomes)
  - Relationship (entity trust scores)
- **Full-text search** via SQLite FTS5
- **Hybrid RAG retrieval** -- combines FTS5 keyword matching with vector cosine similarity (configurable `hybrid_weight` blend); used for pre-inference context retrieval
- **Multi-provider embeddings** -- real embeddings from OpenAI, Ollama, or Google; n-gram fallback when no provider configured
- **Binary BLOB embedding storage** -- `Vec<f32>` stored as native byte arrays (~4x smaller than JSON text); backward-compatible JSON fallback for legacy data
- **HNSW ANN index** -- optional approximate nearest-neighbor index (instant-distance crate) for O(log n) similarity search over large embedding sets; toggled via `memory.ann_index`
- **Content chunking** -- long documents split into overlapping chunks (default 512 tokens, 64-token overlap) for more granular embedding and retrieval
- **Memory budget manager** -- configurable per-tier token allocation with unused rollover
- **Post-turn ingestion** -- background task classifies conversation turns and stores content into appropriate memory tiers, generating and persisting embeddings
- **Background pruning** via heartbeat task

### Scheduling

- **Durable scheduler** -- cron expressions, interval, one-time timestamps; all state in SQLite
- **Lease-based execution** -- prevents double-execution across instances
- **Heartbeat daemon** -- configurable tick interval, builds TickContext (balance, survival tier) per tick
- **7 built-in tasks**: SurvivalCheck, UsdcMonitor, YieldTask, MemoryPrune, CacheEvict, MetricSnapshot, AgentCardRefresh

### Financial

- **Ethereum wallet** -- keypair generation/loading via `alloy-rs`, EIP-191 signing
- **x402 payment protocol** -- EIP-3009 TransferWithAuthorization for automated LLM payments
- **Treasury policy** -- per-payment, hourly, daily, and minimum reserve limits
- **Yield engine** -- deposits idle USDC into Aave/Compound on Base, auto-withdraws when balance drops below threshold
- **Survival tier system** -- high/normal/low_compute/critical/dead states drive model downgrading and distress signals

### Channels

- **Telegram** -- long-poll + webhook, Markdown V2 formatting, 4096-char chunking
- **WhatsApp** -- Cloud API webhook, signature verification
- **Discord** -- gateway WebSocket, slash commands, embed formatting
- **WebSocket** -- direct browser/client connections with ping/pong keepalive
- **A2A (Agent-to-Agent)** -- zero-trust protocol:
  - Mutual authentication via Ethereum signatures (ERC-8004 on-chain identity)
  - ECDH session key derivation with forward secrecy
  - AES-256-GCM encrypted messages
  - Timestamp freshness, message size limits, per-peer rate limiting
  - Trust scoring integrated with relationship memory
  - Opacity principle: no internal state exposed to peers
- **Delivery queue** -- persistent message delivery with retry logic
- **Channel router** -- unified routing across all adapters

### Plugin SDK

- **Plugin trait** -- `name()`, `version()`, `tools()`, `execute()`
- **6 script languages**: `.gosh` (preferred), `.go`, `.sh`, `.py`, `.rb`, `.js`
- **Sandboxed execution** -- configurable timeout, output size cap, interpreter whitelist, environment stripping
- **Plugin manifest** (`plugin.toml`) -- declarative tool registration with risk levels
- **Auto-discovery** -- scans plugin directories, registers tools at boot
- **Hot-reload** -- detects content hash changes and re-registers

### Browser Automation

- **Chrome DevTools Protocol** via WebSocket
- **Action types**: navigate, click, type, screenshot, evaluate JavaScript, wait, scroll, extract
- **Session management** -- start/stop headless Chrome instances
- **REST API integration** -- `/api/browser/*` endpoints for remote control

### Obsidian Integration

- **Bidirectional knowledge store** -- agent reads vault content via `KnowledgeSource` trait, writes documents via `Tool` implementations
- **Full Obsidian support** -- YAML frontmatter parsing, wikilink resolution (case-insensitive), backlink index, inline `#tag` extraction
- **Three agent tools** -- `obsidian_read` (Safe), `obsidian_write` (Caution), `obsidian_search` (Safe)
- **Preferred destination** -- when enabled, the system prompt directs the agent to write persistent documents to the vault
- **Template engine** -- `{{variable}}` substitution with built-in `{{date}}` and `{{time}}`
- **`obsidian://` URI generation** -- clickable links to open notes in Obsidian
- **Auto-detect** -- opt-in scanning of specified paths for `.obsidian` directories
- **File watching** (optional, `notify` crate) -- re-indexes vault on filesystem changes with 500ms debounce
- **Config** -- `[obsidian]` section in `ironclad.toml` with `vault_path`, `default_folder`, `tag_boost`, `ignored_folders`

### Skill System

Two formats, both loaded from a configurable skills directory:

- **Structured skills** (`.toml`) -- declarative tool chains with parameter templates, script paths, and policy overrides
- **Instruction skills** (`.md`) -- YAML frontmatter (triggers, priority) + markdown body injected verbatim into the system prompt
- **Trigger matching** -- keyword, tool name, and regex patterns
- **Safety scanning** on import -- 50+ danger patterns across 5 categories (destructive commands, network access, filesystem access, env exfiltration, obfuscation)

### CLI

24 top-level commands, 15 subcommand groups:

```text
ironclad serve          Boot the runtime (aliases: start, run)
ironclad init           Initialize workspace
ironclad setup          Interactive setup wizard
ironclad check          Validate configuration
ironclad status         Agent status overview
ironclad mechanic       Diagnostics and self-repair
ironclad logs           View logs (--follow, --level)
ironclad dashboard      Open web dashboard
ironclad sessions       List / show / create / export sessions
ironclad memory         List / search memory tiers
ironclad skills         List / show / reload / import / export skills
ironclad schedule       View scheduled tasks
ironclad metrics        Costs / transactions / cache stats
ironclad wallet         Show / address / balance
ironclad config         Show / get / set / unset config keys
ironclad models         List / scan providers
ironclad plugins        List / info / install / uninstall / enable / disable
ironclad agents         List / start / stop agents
ironclad channels       Channel status
ironclad circuit        Circuit breaker status / reset
ironclad migrate        Bidirectional data migration (import / export / 6 data areas)
ironclad daemon         System daemon install / status / uninstall
ironclad security       Security audit
ironclad update         Check for updates (stable/beta/dev)
ironclad version        Version and build info
```

### REST API

48 routes covering all subsystems. Full CRUD for sessions, memory, cron jobs, skills, plugins, agents, and browser sessions. WebSocket push for real-time events.

### Dashboard

Single-page application embedded in the binary (zero external dependencies):

- **9 pages**: Overview, Sessions, Memory, Skills, Scheduler, Metrics, Wallet, Settings, Workspace
- **4 themes**: AI Black & Purple (Default), CRT Orange, CRT Green, Psychedelic Freakout
- **Live sparkline charts** and stacked area charts for multi-provider cost breakdown
- **Retro CRT aesthetic** with scanline effects and monospace typography

### Data Migration

Bidirectional import/export engine covering 6 data areas: configuration, personality, skills, sessions, cron jobs, and channels. Supports migration from external agent platforms.

#### CLI

```text
ironclad migrate import <source-root> [--areas config,skills,...] [--yes]
ironclad migrate export <target-dir>  [--areas config,skills,...]
ironclad skills import <path>         [--no-safety-check] [--accept-warnings]
ironclad skills export <output-dir>   [--ids skill-a,skill-b]
```

#### Data Areas (6)

| Area | Import | Export |
| --- | --- | --- |
| **Configuration** | External agent config -> `ironclad.toml` (agent identity, model, provider, temperature, max_tokens). API keys extracted to env vars for security. | `ironclad.toml` -> portable JSON config with deep-merge to preserve unknown fields. |
| **Personality** | Markdown personality files -> `OS.toml` / `FIRMWARE.toml`. Sections parsed into TOML keys; full original stored as `prompt_text` for round-trip fidelity. | `OS.toml` -> personality markdown, `FIRMWARE.toml` -> behavioral instructions markdown. If `prompt_text` exists, the original markdown is restored byte-for-byte. |
| **Skills** | External skill directories (scripts, configs) copied to `~/.ironclad/skills/` after safety scanning. Critical findings block import; warnings prompt confirmation. | Skills exported to portable directory structure. |
| **Sessions** | JSON/JSONL session files ingested into SQLite `sessions` + `session_messages` tables. | SQLite sessions exported to JSON with full message history per session. |
| **Cron Jobs** | JSON job definitions imported to SQLite `cron_jobs` table with schedule expressions, payloads, and enabled state. | SQLite cron jobs exported to JSON with name, schedule, command, and enabled flag. |
| **Channels** | Channel config blocks -> `channels.toml`. Tokens extracted to env vars. | Channel definitions exported to portable JSON format. |

#### Safety Scanning

All skill imports (both via `ironclad migrate import` and standalone `ironclad skills import`) pass through a safety scanner that checks for 50+ danger patterns across 5 categories:

- **Dangerous Commands** -- `rm -rf /`, fork bombs, pipe-to-shell RCE, dynamic eval
- **Network Access** -- curl, wget, netcat, SSH
- **Filesystem Access** -- writes to `~/.ssh/`, `~/.gnupg/`, access to `ironclad.db` or `wallet.json`
- **Environment Exfiltration** -- reading `$API_KEY`, `$SECRET`, `$PASSWORD`, `process.env`, `os.environ`
- **Obfuscation** -- base64-decode piped to shell

Verdicts: **Clean** (safe to import), **Warnings** (review recommended, user confirms), **Critical** (import blocked unless `--no-safety-check` override).

#### Design Principles

- **Secrets never stored in config files** -- API keys and tokens are translated to environment variable references during import, with the actual values reported as warnings so the operator can set them
- **Round-trip fidelity** -- personality files store the original markdown as `prompt_text`, so an import followed by an export reproduces the original files verbatim
- **Non-destructive export** -- exporting to an existing directory deep-merges into the target config rather than overwriting it, preserving fields Ironclad doesn't know about
- **Selective migration** -- the `--areas` flag allows migrating individual data areas independently (e.g., import only skills and sessions, export only config)

### Developer Tooling

29 `just` recipes: build, test, coverage (80% min / 90% goal), lint, format, watch, database inspection, dependency audit, and all CLI management shortcuts.

---

### Cost Reduction Estimates

| Optimization | Mechanism | Estimated Savings |
| --- | --- | --- |
| Heuristic model routing | Route simple queries to cheap/local models | 60-85% on routed queries |
| Semantic caching | Avoid re-inferring semantically identical prompts | 15-30% cache hit rate |
| Progressive context | Load only what complexity requires (2K vs 16K tokens) | 40-60% input token reduction |
| Connection pooling | Eliminate TCP setup overhead | ~50ms per request |
| Yield engine | Earn interest on idle USDC | 4-8% APY |

---

## Quick Start

```bash
# Install from crates.io
cargo install ironclad-server

# Or install from source
# cargo install --path crates/ironclad-server

# Initialize workspace
ironclad init

# Interactive setup
ironclad setup

# Start the runtime
ironclad serve

# Open dashboard
ironclad dashboard
```

## Developer Quick Start

```bash
# Install dev tools
just install-tools

# Build
just build

# Test
just test

# Coverage report
just coverage

# Watch mode
just watch
```
