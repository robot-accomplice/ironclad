//! # ironclad-core
//!
//! Core types, configuration, error handling, and encrypted credential storage
//! for the Ironclad agent runtime. This is the leaf crate in the dependency
//! graph -- every other workspace crate depends on it.
//!
//! ## Key Types
//!
//! - [`IroncladConfig`] -- Central configuration loaded from `ironclad.toml`
//! - [`IroncladError`] / [`Result`] -- Unified error type (13 variants) used across all crates
//! - [`Keystore`] -- Encrypted key-value storage for API keys and secrets
//! - [`SurvivalTier`] -- Financial health tier derived from on-chain balance
//!
//! ## Modules
//!
//! - `config` -- Configuration structs, TOML parsing, tilde expansion, validation
//! - `error` -- `IroncladError` enum and `Result` type alias
//! - `keystore` -- Encrypted JSON file store with machine-key auto-unlock
//! - `personality` -- OS/soul/firmware personality loading from workspace
//! - `style` -- Terminal theme (CRT, typewriter effect, icons)
//! - `types` -- Shared domain enums: `SurvivalTier`, `AgentState`, `ApiFormat`,
//!   `ModelTier`, `PolicyDecision`, `RiskLevel`, `SkillManifest`, etc.

pub mod config;
pub mod error;
pub mod input_capability_scan;
pub mod keystore;
pub mod personality;
pub mod style;
pub mod types;

pub use config::{IroncladConfig, home_dir};
pub use error::{IroncladError, Result};
pub use keystore::Keystore;
pub use types::*;
