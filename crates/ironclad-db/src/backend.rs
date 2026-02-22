use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::debug;

/// Abstract storage backend trait.
/// SQLite is the default; PostgreSQL is available as an opt-in alternative.
pub trait StorageBackend: Send + Sync + std::fmt::Debug {
    /// Execute a query that returns rows.
    fn query(&self, sql: &str, params: &[QueryParam]) -> Result<Vec<Row>, StorageError>;

    /// Execute a statement that modifies data (INSERT, UPDATE, DELETE).
    fn execute(&self, sql: &str, params: &[QueryParam]) -> Result<u64, StorageError>;

    /// Begin a transaction.
    fn begin_transaction(&self) -> Result<(), StorageError>;

    /// Commit the current transaction.
    fn commit(&self) -> Result<(), StorageError>;

    /// Rollback the current transaction.
    fn rollback(&self) -> Result<(), StorageError>;

    /// Get the backend type name.
    fn backend_type(&self) -> &str;

    /// Check if the backend is healthy/connected.
    fn is_healthy(&self) -> bool;
}

/// A generic query parameter.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum QueryParam {
    Text(String),
    Integer(i64),
    Real(f64),
    Blob(Vec<u8>),
    Null,
}

impl From<&str> for QueryParam {
    fn from(s: &str) -> Self {
        QueryParam::Text(s.to_string())
    }
}

impl From<String> for QueryParam {
    fn from(s: String) -> Self {
        QueryParam::Text(s)
    }
}

impl From<i64> for QueryParam {
    fn from(v: i64) -> Self {
        QueryParam::Integer(v)
    }
}

impl From<f64> for QueryParam {
    fn from(v: f64) -> Self {
        QueryParam::Real(v)
    }
}

/// A generic row from a query result.
#[derive(Debug, Clone)]
pub struct Row {
    pub columns: HashMap<String, ColumnValue>,
}

impl Row {
    pub fn new() -> Self {
        Self {
            columns: HashMap::new(),
        }
    }

    pub fn get_text(&self, col: &str) -> Option<&str> {
        match self.columns.get(col) {
            Some(ColumnValue::Text(s)) => Some(s),
            _ => None,
        }
    }

    pub fn get_integer(&self, col: &str) -> Option<i64> {
        match self.columns.get(col) {
            Some(ColumnValue::Integer(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_real(&self, col: &str) -> Option<f64> {
        match self.columns.get(col) {
            Some(ColumnValue::Real(v)) => Some(*v),
            _ => None,
        }
    }

    pub fn get_blob(&self, col: &str) -> Option<&[u8]> {
        match self.columns.get(col) {
            Some(ColumnValue::Blob(b)) => Some(b),
            _ => None,
        }
    }

    pub fn is_null(&self, col: &str) -> bool {
        matches!(self.columns.get(col), Some(ColumnValue::Null) | None)
    }
}

impl Default for Row {
    fn default() -> Self {
        Self::new()
    }
}

/// A column value in a row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ColumnValue {
    Text(String),
    Integer(i64),
    Real(f64),
    Blob(Vec<u8>),
    Null,
}

/// Storage error type.
#[derive(Debug, Clone)]
pub struct StorageError {
    pub message: String,
    pub kind: StorageErrorKind,
}

impl StorageError {
    pub fn new(kind: StorageErrorKind, message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind,
        }
    }
}

impl std::fmt::Display for StorageError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.kind, self.message)
    }
}

impl std::error::Error for StorageError {}

/// Categories of storage errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageErrorKind {
    ConnectionFailed,
    QueryFailed,
    TransactionFailed,
    ConstraintViolation,
    NotFound,
    Internal,
}

impl std::fmt::Display for StorageErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StorageErrorKind::ConnectionFailed => write!(f, "connection_failed"),
            StorageErrorKind::QueryFailed => write!(f, "query_failed"),
            StorageErrorKind::TransactionFailed => write!(f, "transaction_failed"),
            StorageErrorKind::ConstraintViolation => write!(f, "constraint_violation"),
            StorageErrorKind::NotFound => write!(f, "not_found"),
            StorageErrorKind::Internal => write!(f, "internal"),
        }
    }
}

/// In-memory storage backend for testing.
#[derive(Debug)]
pub struct InMemoryBackend {
    _tables: std::sync::Mutex<HashMap<String, Vec<Row>>>,
    in_transaction: std::sync::atomic::AtomicBool,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self {
            _tables: std::sync::Mutex::new(HashMap::new()),
            in_transaction: std::sync::atomic::AtomicBool::new(false),
        }
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl StorageBackend for InMemoryBackend {
    fn query(&self, sql: &str, _params: &[QueryParam]) -> Result<Vec<Row>, StorageError> {
        debug!(sql, "in-memory query");
        Ok(Vec::new())
    }

    fn execute(&self, sql: &str, _params: &[QueryParam]) -> Result<u64, StorageError> {
        debug!(sql, "in-memory execute");
        Ok(0)
    }

    fn begin_transaction(&self) -> Result<(), StorageError> {
        self.in_transaction
            .store(true, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn commit(&self) -> Result<(), StorageError> {
        self.in_transaction
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn rollback(&self) -> Result<(), StorageError> {
        self.in_transaction
            .store(false, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    fn backend_type(&self) -> &str {
        "in-memory"
    }

    fn is_healthy(&self) -> bool {
        true
    }
}

/// Backend configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackendConfig {
    #[serde(default = "default_backend")]
    pub backend: String,
    #[serde(default)]
    pub postgres_url: Option<String>,
}

fn default_backend() -> String {
    "sqlite".to_string()
}

impl Default for BackendConfig {
    fn default() -> Self {
        Self {
            backend: default_backend(),
            postgres_url: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_backend_type() {
        let backend = InMemoryBackend::new();
        assert_eq!(backend.backend_type(), "in-memory");
        assert!(backend.is_healthy());
    }

    #[test]
    fn in_memory_query() {
        let backend = InMemoryBackend::new();
        let rows = backend.query("SELECT * FROM test", &[]).unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn in_memory_execute() {
        let backend = InMemoryBackend::new();
        let affected = backend
            .execute(
                "INSERT INTO test VALUES (?)",
                &[QueryParam::Text("hello".into())],
            )
            .unwrap();
        assert_eq!(affected, 0);
    }

    #[test]
    fn in_memory_transaction() {
        let backend = InMemoryBackend::new();
        backend.begin_transaction().unwrap();
        backend.commit().unwrap();
        backend.begin_transaction().unwrap();
        backend.rollback().unwrap();
    }

    #[test]
    fn row_accessors() {
        let mut row = Row::new();
        row.columns
            .insert("name".into(), ColumnValue::Text("Alice".into()));
        row.columns.insert("age".into(), ColumnValue::Integer(30));
        row.columns.insert("score".into(), ColumnValue::Real(9.5));
        row.columns
            .insert("data".into(), ColumnValue::Blob(vec![1, 2, 3]));
        row.columns.insert("empty".into(), ColumnValue::Null);

        assert_eq!(row.get_text("name"), Some("Alice"));
        assert_eq!(row.get_integer("age"), Some(30));
        assert_eq!(row.get_real("score"), Some(9.5));
        assert_eq!(row.get_blob("data"), Some([1, 2, 3].as_slice()));
        assert!(row.is_null("empty"));
        assert!(row.is_null("nonexistent"));
    }

    #[test]
    fn row_missing_column() {
        let row = Row::new();
        assert!(row.get_text("missing").is_none());
        assert!(row.get_integer("missing").is_none());
    }

    #[test]
    fn query_param_from_conversions() {
        let p1 = QueryParam::from("hello");
        assert!(matches!(p1, QueryParam::Text(_)));

        let p2 = QueryParam::from(42_i64);
        assert!(matches!(p2, QueryParam::Integer(42)));

        let p3 = QueryParam::from(2.72_f64);
        assert!(matches!(p3, QueryParam::Real(_)));
    }

    #[test]
    fn storage_error_display() {
        let err = StorageError::new(StorageErrorKind::QueryFailed, "bad SQL");
        assert!(err.to_string().contains("query_failed"));
        assert!(err.to_string().contains("bad SQL"));
    }

    #[test]
    fn storage_error_kind_display() {
        assert_eq!(
            format!("{}", StorageErrorKind::ConnectionFailed),
            "connection_failed"
        );
        assert_eq!(format!("{}", StorageErrorKind::NotFound), "not_found");
    }

    #[test]
    fn backend_config_defaults() {
        let config = BackendConfig::default();
        assert_eq!(config.backend, "sqlite");
        assert!(config.postgres_url.is_none());
    }

    #[test]
    fn query_param_serde() {
        let params = vec![
            QueryParam::Text("hello".into()),
            QueryParam::Integer(42),
            QueryParam::Real(2.72),
            QueryParam::Null,
        ];
        for p in &params {
            let json = serde_json::to_string(p).unwrap();
            let back: QueryParam = serde_json::from_str(&json).unwrap();
            assert!(matches!(
                (&p, &back),
                (QueryParam::Text(_), QueryParam::Text(_))
                    | (QueryParam::Integer(_), QueryParam::Integer(_))
                    | (QueryParam::Real(_), QueryParam::Real(_))
                    | (QueryParam::Null, QueryParam::Null)
            ));
        }
    }
}
