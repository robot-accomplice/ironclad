//! # ironclad-cli
//!
//! CLI command handlers and migration engine for the Ironclad agent runtime.
//! This crate contains all `ironclad <subcommand>` implementations (status,
//! sessions, memory, wallet, update, admin, defrag, etc.) as well as the
//! Legacy ↔ Ironclad migration and skill import/export engine.

pub mod cli;
pub mod migrate;
pub mod state_hygiene;
