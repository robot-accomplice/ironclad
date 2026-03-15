//! # ironclad-api
//!
//! HTTP routes, WebSocket push, authentication middleware, rate limiting,
//! dashboard serving, config runtime, cron runtime, and abuse protection
//! for the Ironclad agent runtime.
//!
//! ## Key Types
//!
//! - [`AppState`] -- Shared application state passed to all route handlers
//! - [`PersonalityState`] -- Loaded personality files (OS, firmware, identity)
//! - [`EventBus`] -- Tokio broadcast channel for WebSocket event push
//!
//! ## Modules
//!
//! - `api` -- REST API mount point, `build_router()`, route modules
//! - `auth` -- API key authentication middleware layer
//! - `rate_limit` -- Global + per-IP rate limiting (sliding window)
//! - `dashboard` -- Embedded SPA serving (compile-time or filesystem)
//! - `ws` -- WebSocket upgrade and event broadcasting
//! - `config_runtime` -- Runtime config parsing, hot-reload, and apply
//! - `cron_runtime` -- Background cron task execution
//! - `abuse` -- Abuse detection and protection

pub mod abuse;
pub mod api;
pub mod auth;
pub mod config_runtime;
pub mod cron_runtime;
pub mod dashboard;
pub mod rate_limit;
pub mod ws;
pub mod ws_ticket;

pub use api::{AppState, PersonalityState, build_mcp_router, build_public_router, build_router};
pub use dashboard::{build_dashboard_html, dashboard_handler};
pub use ws::{EventBus, ws_route};
pub use ws_ticket::TicketStore;
