# Ironclad

A single-binary autonomous agent runtime written in Rust, designed for maximum efficiency, minimum operational cost, and self-sustainability.

---

## Architecture

Ironclad compiles to a single static binary that replaces the multi-process, multi-language stack of its predecessor (OpenClaw). Every subsystem -- LLM routing, agent loop, memory, scheduling, wallet, channels, dashboard -- runs in one OS process on one async runtime (tokio), sharing one SQLite database.

| Metric | Value |
| --- | --- |
| Language | Rust (edition 2024) |
| Workspace crates | 11 |
| Source files | 108 |
| Lines of code | ~32,000 |
| Test count | ~766 unit + integration |
| SQLite tables | 28 |
| Architecture docs | 17 (C4 + dataflow + sequence diagrams) |

### Crate Map

```text
ironclad-core        Shared types, config, error definitions, personality system
ironclad-db          SQLite persistence (28 tables, FTS5, WAL mode)
ironclad-llm         LLM client pipeline, format translation, routing, caching, circuit breakers
ironclad-agent       ReAct loop, tool system, policy engine, injection defense, skills, subagents
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
- **3-level semantic cache** -- L1 exact hash, L2 embedding cosine similarity, L3 deterministic tool TTL
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
- **Memory budget manager** -- configurable per-tier token allocation with unused rollover
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
ironclad migrate        Bidirectional OpenClaw migration (import / export / 6 data areas)
ironclad daemon         System daemon install / status / uninstall
ironclad security       Security audit
ironclad update         Check for updates (stable/beta/dev)
ironclad version        Version and build info
```

### REST API

41 routes covering all subsystems. Full CRUD for sessions, memory, cron jobs, skills, plugins, agents, and browser sessions. WebSocket push for real-time events.

### Dashboard

Single-page application embedded in the binary (zero external dependencies):

- **9 pages**: Overview, Sessions, Memory, Skills, Scheduler, Metrics, Wallet, Settings, Workspace
- **4 themes**: AI Black & Purple (Default), CRT Orange, CRT Green, Psychedelic Freakout
- **Live sparkline charts** and stacked area charts for multi-provider cost breakdown
- **Retro CRT aesthetic** with scanline effects and monospace typography

### Migration (OpenClaw Interoperability)

Full bidirectional import/export between OpenClaw and Ironclad, enabling zero-downtime migration in either direction or running both systems side by side during a transition.

#### CLI

```text
ironclad migrate import <openclaw-root> [--areas config,skills,...] [--yes]
ironclad migrate export <target-dir>    [--areas config,skills,...]
ironclad skills import <path>           [--no-safety-check] [--accept-warnings]
ironclad skills export <output-dir>     [--ids skill-a,skill-b]
```

#### Data Areas (6)

| Area | Import (OpenClaw -> Ironclad) | Export (Ironclad -> OpenClaw) |
| --- | --- | --- |
| **Configuration** | `openclaw.json` -> `ironclad.toml` (agent identity, model, provider, temperature, max_tokens). API keys extracted to env vars for security. | `ironclad.toml` -> `openclaw.json` with deep-merge to preserve unknown fields in an existing `openclaw.json`. |
| **Personality** | `SOUL.md` -> `OS.toml`, `AGENTS.md` -> `FIRMWARE.toml`. Markdown sections parsed into TOML keys; full original stored as `prompt_text` for round-trip fidelity. | `OS.toml` -> `SOUL.md`, `FIRMWARE.toml` -> `AGENTS.md`. If `prompt_text` exists, the original markdown is restored byte-for-byte. |
| **Skills** | OpenClaw skill directories (scripts, configs) copied to Ironclad `~/.ironclad/skills/` after safety scanning. Critical findings block import; warnings prompt confirmation. | Ironclad skills exported to OpenClaw `workspace/skills/` directory. |
| **Sessions** | `sessions.json` (array format) and `agents/<agent>/sessions/*.jsonl` (line-delimited messages) ingested into SQLite `sessions` + `session_messages` tables. | SQLite sessions exported to `sessions.json` with full message history per session. |
| **Cron Jobs** | `jobs.json` and inline `cron` array from `openclaw.json` imported to SQLite `cron_jobs` table with schedule expressions, payloads, and enabled state. | SQLite cron jobs exported to `jobs.json` with name, schedule, command, and enabled flag. |
| **Channels** | Telegram and WhatsApp config from `openclaw.json` channels block -> `channels.toml`. Tokens extracted to env vars. | `channels.toml` / `ironclad.toml` channel definitions -> `openclaw.json` channels block. Merges into existing config. |

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
- **Round-trip fidelity** -- personality files store the original markdown as `prompt_text`, so an import followed by an export reproduces the original `SOUL.md` / `AGENTS.md` verbatim
- **Non-destructive export** -- exporting to an existing OpenClaw directory deep-merges into `openclaw.json` rather than overwriting it, preserving fields Ironclad doesn't know about
- **Selective migration** -- the `--areas` flag allows migrating individual data areas independently (e.g., import only skills and sessions, export only config)

### Developer Tooling

29 `just` recipes: build, test, coverage (80% min / 90% goal), lint, format, watch, database inspection, dependency audit, and all CLI management shortcuts.

---

## Comparison with OpenClaw

| Dimension | OpenClaw | Ironclad |
| --- | --- | --- |
| **Architecture** | 3 separate processes (Node.js gateway, Python proxy, TypeScript automaton) | Single Rust binary |
| **Languages** | Node.js + Python + TypeScript + Go | Rust (one language, one toolchain) |
| **Memory usage** | ~500 MB (3 processes) | ~50 MB (1 process) |
| **Proxy latency** | ~50ms (Python aiohttp, new connection per request) | ~2ms (in-process, persistent connection pool) |
| **Cold start** | ~3s (Node.js) + ~2s (Python) | ~50ms |
| **Binary / deploy size** | ~200 MB (node_modules + pip packages) | ~15 MB static binary |
| **Supply chain** | 500+ npm + pip packages | ~50 auditable crates |
| **Type safety** | Runtime errors (untyped dicts in proxy) | Compile-time guarantees (strongly typed enums) |
| **Database** | 5 storage layers (JSONL, SQLite, JSON files, Markdown) | 1 unified SQLite database (28 tables, WAL mode) |
| **Sessions** | Append-only JSONL files | SQLite `sessions` + `session_messages` tables |
| **Memory** | Split across Markdown, JSON, and SQLite (automaton) with incompatible schemas | Unified 5-tier memory in single SQLite DB |
| **Full-text search** | None | SQLite FTS5 |
| **Model routing** | Rule-based fallback chain only | Heuristic complexity classifier + rule-based fallback |
| **Semantic caching** | None | 3-level cache (exact hash, embedding similarity, tool TTL) |
| **Prompt compression** | None (17 KB system prompt sent every request) | Progressive context loading (4 levels, 2K-16K tokens) |
| **Token estimation** | `len(payload) // 4` (20-40% error) | Config-driven per-model cost rates |
| **Connection pooling** | New `ClientSession` per request (Python) | Persistent `reqwest::Client` with HTTP/2 |
| **Provider config** | Hardcoded provider knowledge in code | Fully externalized TOML config; adding providers requires zero code changes |
| **Format translation** | Untyped dict-based translation in Python | Strongly typed Rust enums with `From<T>` traits (12+ pairs) |
| **Injection defense** | 8 regex checks in automaton only | 4-layer defense: regex + HMAC boundaries + output validation + output scanning |
| **Agent-to-agent** | No mutual authentication | Zero-trust: ECDSA mutual auth, ECDH key exchange, AES-256-GCM encryption, trust scoring |
| **Financial** | x402 topup only; USDC sits idle | x402 + yield engine (Aave/Compound on Base); 4-8% APY on idle funds |
| **Treasury controls** | Runtime policy checks | Compile-time limit types + runtime policy engine |
| **Cron / scheduling** | JSON file + separate heartbeat daemon | Unified SQLite-backed scheduler with lease-based execution |
| **Dashboard** | Next.js app (separate process, read-only) | Embedded SPA in binary (read + write, 41 API routes) |
| **Dashboard actions** | Observatory only (cannot restart, modify, or control) | Full control: restart agents, modify cron, reset breakers, toggle skills, manage plugins |
| **Plugin system** | OpenClaw skills (Markdown only) | Dual-format skills (structured TOML + instruction Markdown) + plugin SDK (6 scripting languages) |
| **Preferred script language** | None specified | `gosh` (Go-based cross-platform shell) |
| **Browser automation** | None | CDP-based headless Chrome (navigate, click, type, screenshot, evaluate) |
| **Channel support** | Telegram, WhatsApp | Telegram, WhatsApp, Discord, WebSocket, A2A |
| **CLI** | `openclaw` with limited subcommands | 24 top-level commands, 15 subcommand groups, interactive wizard, self-repair |
| **Testing** | Automaton: 897 tests; Proxy: 0; Gateway: unknown; Dashboard: 0 | ~766 tests across all crates + dedicated integration test suite |
| **Test coverage** | Unknown / inconsistent | 80% minimum enforced, 90% target |
| **Proxy authentication** | None (open localhost) | Token-based API authentication |
| **Self-modification** | Agent can edit its own code (guarded only by in-process policy) | Policy engine + path protection + authority rules (creator-only for self-mod) |
| **Migration** | N/A | Bidirectional import/export with safety scanning |

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
# Install
cargo install --path crates/ironclad-server

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
