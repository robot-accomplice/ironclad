//! # ironclad-agent
//!
//! Agent core for the Ironclad runtime. The central module is `agent_loop`
//! (mapped from `loop.rs`), which implements a ReAct reasoning loop as a typed
//! state machine: Think → Act → Observe → Persist, with idle/loop detection
//! and financial guards.
//!
//! ## Key Types
//!
//! - `agent_loop` -- ReAct state machine with typed transitions via `typestate`
//! - [`tools::ToolRegistry`] -- Trait-based tool system (10 categories)
//! - [`policy::PolicyEngine`] -- Rule-based policy evaluation
//! - [`retrieval::MemoryRetriever`] -- Hybrid RAG pipeline (FTS5 + vector cosine)
//! - [`analyzer::ContextAnalyzer`] -- Topic extraction, sentiment, complexity
//! - [`recommendations::RecommendationEngine`] -- Proactive suggestions
//! - [`obsidian::ObsidianVault`] -- Obsidian vault integration
//! - [`skills::SkillLoader`] / [`skills::SkillRegistry`] -- Dual-format skill system
//!
//! ## Modules
//!
//! - `context` -- Progressive context loading (4 levels) and compression
//! - `injection` -- 4-layer prompt injection defense
//! - `prompt` -- System prompt builder with HMAC trust boundaries
//! - `memory` -- Memory budget manager and turn ingestion
//! - `retrieval` -- Hybrid search RAG pipeline with content chunking
//! - `knowledge` -- KnowledgeSource trait and aggregation
//! - `discovery` -- Capability discovery across tools, skills, plugins
//! - `digest` -- Turn digest and history summarization
//! - `device` -- Device context and hardware info
//! - `workspace` -- Workspace state (file tree, git status, project metadata)
//! - `orchestration` -- Multi-agent task decomposition
//! - `governor` -- Rate governor and concurrency limits
//! - `typestate` -- Compile-time valid state transitions
//! - `speculative` -- Parallel branch evaluation with best-result selection
//! - `manifest` -- Agent capability declarations
//! - `services` -- Service locator and dependency wiring
//! - `mcp` -- Model Context Protocol client integration
//! - `wasm` -- WASM plugin runtime
//! - `obsidian` / `obsidian_tools` -- Vault integration and read/write/search tools
//! - `skills` / `script_runner` -- Skill loading, execution, sandboxed scripts
//! - `analyzer` / `recommendations` -- Conversation analysis and proactive suggestions

#[path = "loop.rs"]
pub mod agent_loop;
pub mod analyzer;
pub mod approvals;
pub mod context;
pub mod device;
pub mod digest;
pub mod discovery;
pub mod governor;
pub mod ingest;
pub mod injection;
pub mod interview;
pub mod knowledge;
pub mod manifest;
pub mod mcp;
pub mod mcp_handler;
pub mod memory;
pub mod obsidian;
pub mod obsidian_tools;
pub mod orchestration;
pub mod policy;
pub mod prompt;
pub mod recommendations;
pub mod retrieval;
pub mod script_runner;
pub mod services;
pub mod skills;
pub mod speculative;
pub mod subagents;
pub mod tools;
pub mod typestate;
pub mod wasm;
pub mod workspace;
