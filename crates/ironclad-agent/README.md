# ironclad-agent

> **Version 0.5.0**

Agent core for the [Ironclad](https://github.com/robot-accomplice/ironclad) runtime. Implements the ReAct reasoning loop as a typed state machine, with policy enforcement, 4-layer prompt injection defense, HMAC trust boundaries, 5-tier memory with hybrid RAG retrieval, conversation analysis, proactive recommendations, multi-agent orchestration, WASM plugin execution, and MCP integration.

## Key Types & Traits

| Type | Module | Description |
|------|--------|-------------|
| `agent_loop` | `loop.rs` | ReAct state machine (Think → Act → Observe → Persist) |
| `ToolRegistry` | `tools` | Trait-based tool system with 10 categories |
| `PolicyEngine` | `policy` | Rule-based policy evaluation (Authority, Safety, Financial, Path, Rate) |
| `MemoryRetriever` | `retrieval` | Hybrid RAG pipeline (FTS5 + vector cosine) |
| `ConversationAnalyzer` | `analyzer` | Topic extraction, sentiment, complexity scoring |
| `RecommendationEngine` | `recommendations` | Proactive suggestion generation |
| `ObsidianVault` | `obsidian` | Vault scanning, wikilink resolution, template engine |
| `SkillLoader` / `SkillRegistry` | `skills` | Dual-format skill system (structured + instruction) |
| `ScriptRunner` | `script_runner` | Sandboxed script execution with timeout and allowlists |
| `SubagentRegistry` | `subagents` | Multi-agent registry and concurrency management |

## Modules

| Module | Description |
|--------|-------------|
| `context` | Progressive context loading and compression |
| `injection` | 4-layer injection defense (input scan, HMAC boundaries, output validation, adaptive) |
| `prompt` | System prompt builder with HMAC trust markers |
| `memory` | Memory budget manager and turn ingestion |
| `knowledge` | KnowledgeSource trait and aggregation |
| `discovery` | Capability discovery (tools, skills, plugins) |
| `digest` | Turn digest and history summarization |
| `device` | Device context and hardware info |
| `workspace` | Workspace state manager (file tree, git, metadata) |
| `orchestration` | Multi-agent task decomposition |
| `governor` | Rate governor and concurrency limits |
| `typestate` | Compile-time valid state machine transitions |
| `speculative` | Speculative parallel branch evaluation |
| `manifest` | Agent capability declarations |
| `services` | Service locator and dependency wiring |
| `mcp` | Model Context Protocol client |
| `wasm` | WASM plugin runtime (wasmtime) |
| `interview` | Personality interview flow |
| `approvals` | Gated tool approval flows |

## Usage

```toml
[dependencies]
ironclad-agent = "0.5"
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-agent).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
