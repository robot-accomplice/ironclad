//! # ironclad-db
//!
//! SQLite persistence layer for the Ironclad agent runtime. All state --
//! sessions, memories, tool calls, policy decisions, cron jobs, embeddings,
//! skills, and semantic cache -- lives in a single SQLite database with WAL
//! mode enabled.
//!
//! ## Key Types
//!
//! - [`Database`] -- Thread-safe handle wrapping `Arc<Mutex<Connection>>`
//!
//! ## Modules
//!
//! - `schema` -- Table definitions, `initialize_db()`, migration runner
//! - `sessions` -- Session CRUD, message append/list, turn persistence
//! - `memory` -- 5-tier memory CRUD (working, episodic, semantic, procedural, relationship) + FTS5
//! - `embeddings` -- BLOB embedding storage / lookup with JSON fallback
//! - `ann` -- HNSW approximate nearest-neighbor index (instant-distance)
//! - `hippocampus` -- Long-term memory consolidation and decay
//! - `checkpoint` -- Session checkpoint / restore via `context_snapshots` table
//! - `efficiency` -- Efficiency metrics tracking and queries
//! - `agents` -- Sub-agent registry and enabled-agent CRUD
//! - `backend` -- Storage backend abstraction trait
//! - `cache` -- Semantic cache persistence (loaded on boot, flushed every 5 min)
//! - `cron` -- Cron job state, lease acquisition, run history
//! - `skills` -- Skill definition CRUD and trigger lookup
//! - `tools` -- Tool call records
//! - `policy` -- Policy decision records
//! - `metrics` -- Inference cost tracking, proxy snapshots, transactions, turn feedback
//! - `routing_dataset` -- Historical routing decision + cost outcome JOIN for ML training
//! - `shadow_routing` -- Counterfactual ML predictions stored alongside production decisions

pub mod abuse;
pub mod agents;
pub mod ann;
pub mod approvals;
pub mod backend;
pub mod cache;
pub mod checkpoint;
pub mod cron;
pub mod delivery_queue;
pub mod efficiency;
pub mod embeddings;
pub mod hippocampus;
pub mod memory;
pub mod metrics;
pub mod model_selection;
pub mod policy;
pub mod routing_dataset;
pub mod schema;
pub mod sessions;
pub mod shadow_routing;
pub mod skills;
pub mod tools;

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

use ironclad_core::{IroncladError, Result};

#[derive(Clone)]
pub struct Database {
    conn: Arc<Mutex<Connection>>,
}

impl Database {
    /// Opens a new database at the given path (or in-memory if `":memory:"`).
    ///
    /// # Examples
    ///
    /// ```
    /// use ironclad_db::Database;
    ///
    /// let db = Database::new(":memory:").unwrap();
    /// // database is now ready for use
    /// ```
    pub fn new(path: &str) -> Result<Self> {
        let conn = if path == ":memory:" {
            Connection::open_in_memory()
        } else {
            Connection::open(path)
        }
        .map_err(|e| IroncladError::Database(e.to_string()))?;

        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")
            .map_err(|e| IroncladError::Database(e.to_string()))?;

        let db = Self {
            conn: Arc::new(Mutex::new(conn)),
        };
        schema::initialize_db(&db)?;
        Ok(db)
    }

    pub fn conn(&self) -> std::sync::MutexGuard<'_, Connection> {
        self.conn.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl std::fmt::Debug for Database {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Database").finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn database_debug_impl() {
        let db = Database::new(":memory:").expect("in-memory db");
        let s = format!("{:?}", db);
        assert_eq!(s, "Database");
    }

    #[test]
    fn database_new_in_memory() {
        let db = Database::new(":memory:").expect("in-memory db");
        let _guard = db.conn();
    }

    #[test]
    fn database_new_invalid_path_returns_error() {
        let result = Database::new("/");
        assert!(result.is_err(), "opening \"/\" as database should fail");
    }
}
