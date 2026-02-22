pub mod backend;
pub mod checkpoint;
pub mod cron;
pub mod embeddings;
pub mod hippocampus;
pub mod memory;
pub mod metrics;
pub mod policy;
pub mod schema;
pub mod sessions;
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
