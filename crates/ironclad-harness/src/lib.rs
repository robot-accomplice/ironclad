//! # ironclad-harness
//!
//! Parallel smoke/UAT testing harness for Ironclad.
//!
//! Provides [`sandbox::SandboxedServer`] — an isolated server instance with its own
//! port, database, config, and optional mocked LLM — that can run in parallel
//! with other sandboxes for conflict-free full-stack testing.
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use ironclad_harness::sandbox::{SandboxedServer, SandboxMode};
//!
//! #[tokio::test]
//! async fn health_check() {
//!     let server = SandboxedServer::spawn(SandboxMode::InProcess).await.unwrap();
//!     let resp = server.client().get_ok("/api/health").await.unwrap();
//!     assert!(resp["status"].as_str().is_some());
//! }
//! ```
//!
//! ## Architecture
//!
//! Each `SandboxedServer::spawn()` call:
//! 1. Atomically allocates a unique port (`port_pool`)
//! 2. Generates an isolated TOML config in a temp directory (`config_gen`)
//! 3. Boots the server in-process via `bootstrap()` or as a child process (`sandbox`)
//! 4. Provides a typed HTTP client for assertions (`client`)

pub mod client;
pub mod config_gen;
pub mod golden;
pub mod mock_llm;
pub mod port_pool;
pub mod sandbox;
