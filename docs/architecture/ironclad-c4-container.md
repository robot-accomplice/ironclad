# C4 Level 2: Container Diagram — Ironclad Platform

*All containers run within a single Rust binary (logical separation only). Crate list and dependencies match the workspace.*

---

## Container Diagram

```mermaid
C4Container
    title Ironclad Container Diagram (Single Binary)

    Person(creator, "Creator")

    System_Boundary(ironclad, "Ironclad Binary (single OS process)") {

        Container(server, "ironclad-server", "Rust / axum", "HTTP entry: REST API,<br/>dashboard, WebSocket, bootstrap")

        Container(channels, "ironclad-channels", "Rust", "Telegram, WhatsApp, WebSocket,<br/>Agent-to-Agent (A2A)")

        Container(agent, "ironclad-agent", "Rust", "ReAct loop, tools, policy engine (6 rules),<br/>injection defense (4 layers), prompt builder,<br/>context assembly, hybrid RAG retrieval,<br/>content chunking, skills (TOML + MD), script runner")

        Container(llm, "ironclad-llm", "Rust / reqwest", "LLM client: provider registry,<br/>format translation, heuristic model router,<br/>semantic cache (HashMap + SQLite persist),<br/>circuit breaker, tier adaptation,<br/>multi-provider embedding client")

        Container(schedule, "ironclad-schedule", "Rust / tokio", "Heartbeat daemon,<br/>cron worker (DB-backed, leased)")

        Container(wallet, "ironclad-wallet", "Rust / alloy", "Ethereum wallet, treasury,<br/>yield (e.g. Aave V3 on Base)")

        Container(db, "ironclad-db", "Rust / rusqlite", "SQLite: sessions, messages, turns,<br/>5-tier memory, memory_fts (FTS5),<br/>embeddings (BLOB), ANN index (HNSW),<br/>semantic cache persistence,<br/>cron jobs/runs, transactions, costs, skills, identity")

        Container(core, "ironclad-core", "Rust", "Config, types, errors, personality")

        Container(pluginSdk, "ironclad-plugin-sdk", "Rust", "Plugin registry, tool discovery")

        Container(browser, "ironclad-browser", "Rust", "Browser automation (CDP/WebSocket)")
    }

    System_Ext(llmProviders, "LLM Providers", "Anthropic, OpenAI, Ollama, Groq, etc.")
    System_Ext(baseChain, "Base / Base Sepolia", "USDC, Aave V3")
    System_Ext(chatChannels, "Chat Channels", "Telegram, WhatsApp")
    System_Ext(peerAgents, "Peer Agents", "A2A")

    Rel(creator, server, "Dashboard / WebSocket / REST")
    Rel(creator, channels, "Telegram / WhatsApp / Discord / Signal / Email / Voice")
    Rel(server, agent, "In-process")
    Rel(server, db, "In-process: sessions, metrics, config")
    Rel(server, llm, "In-process: inference_costs, cache flush")
    Rel(server, wallet, "In-process: balance, address")
    Rel(server, schedule, "In-process: bootstrap scheduler")
    Rel(server, channels, "In-process: bootstrap adapters")
    Rel(server, core, "In-process: config, types")
    Rel(server, pluginSdk, "In-process")
    Rel(server, browser, "In-process")
    Rel(channels, db, "In-process: delivery_queue")
    Rel(agent, llm, "In-process: inference")
    Rel(agent, db, "In-process: sessions, memory, tools")
    Rel(llm, llmProviders, "HTTPS / HTTP")
    Rel(schedule, agent, "In-process: cron payloads")
    Rel(schedule, db, "In-process: cron_jobs, cron_runs")
    Rel(schedule, wallet, "In-process: heartbeat")
    Rel(wallet, baseChain, "JSON-RPC (alloy)")
    Rel(wallet, db, "In-process: transactions")
    Rel(channels, peerAgents, "HTTPS (A2A)")
    Rel(channels, chatChannels, "HTTPS")
```

## Crates (Workspace Members)

| Crate | Role | Depends On |
|-------|------|------------|
| `ironclad-core` | Config, types, errors, personality | — |
| `ironclad-db` | SQLite schema, migrations, sessions, memory, FTS, embeddings (BLOB), ANN index, cache persistence, cron, skills, metrics | `ironclad-core` |
| `ironclad-llm` | LLM client, heuristic router, semantic cache (persistent), circuit breaker, embedding client | `ironclad-core` |
| `ironclad-agent` | Agent loop, tools, policy (6 rules), injection defense, hybrid RAG retrieval, chunking, skills | `ironclad-core`, `ironclad-db`, `ironclad-llm` |
| `ironclad-wallet` | Wallet, treasury, yield (Base, Aave V3) | `ironclad-core`, `ironclad-db` |
| `ironclad-schedule` | Heartbeat daemon, cron worker | `ironclad-core`, `ironclad-db`, `ironclad-agent`, `ironclad-wallet` |
| `ironclad-channels` | Telegram, WhatsApp, Discord, Signal, Email, Voice, WebSocket, A2A | `ironclad-core`, `ironclad-db` |
| `ironclad-plugin-sdk` | Plugin registry | `ironclad-core` |
| `ironclad-browser` | Browser automation | `ironclad-core` |
| `ironclad-server` | HTTP server, API, dashboard, CLI, bootstrap | All of the above (except tests) |
| `ironclad-tests` | Integration tests | Multiple crates |

**Note**: All containers depend on `ironclad-core` for shared types, config, and errors; `Rel` arrows to `core` are omitted from the diagram for visual clarity. The diagram includes `Rel(schedule, wallet, "In-process: heartbeat")`: ironclad-schedule uses ironclad-wallet for tick context (USDC balance, survival tier).

## Implementation Notes

- **Routing**: Heuristic classifier in `ironclad-llm/src/router.rs` (weighted message length, tool calls, depth). No ONNX or ML models. Runtime config accepts `"primary"` and `"metascore"`; legacy values must be migrated by update/mechanic before startup.
- **Cache**: `SemanticCache` in `ironclad-llm/src/cache.rs` (HashMap, L1 exact / L2 semantic cosine / L3 tool TTL). Persisted to SQLite via `ironclad-db/src/cache.rs` — loaded on boot, flushed every 5 minutes.
- **Policy rules**: Six rules in `ironclad-agent/src/policy.rs`: AuthorityRule, CommandSafetyRule, FinancialRule, PathProtectionRule, RateLimitRule, ValidationRule. Server bootstrap wires AuthorityRule and CommandSafetyRule by default.
- **FTS**: `memory_fts` FTS5 virtual table with columns `content`, `category`, `source_table`, `source_id`. Synced via trigger for episodic; working and semantic inserts in `ironclad-db/src/memory.rs`.

## Communication

All inter-container communication is **in-process** on the tokio runtime. No IPC. Single SQLite connection (WAL) shared via `ironclad-db::Database`.
