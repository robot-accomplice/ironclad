use ironclad_core::error::{IroncladError, Result};

/// Extension trait for converting any Display error into IroncladError::Database.
pub trait DbResultExt<T> {
    fn db_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> DbResultExt<T> for std::result::Result<T, E> {
    fn db_err(self) -> Result<T> {
        self.map_err(|e| IroncladError::Database(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_result_ext_converts_error() {
        let err: std::result::Result<(), &str> = Err("test error");
        let converted = err.db_err();
        assert!(converted.is_err());
        match converted.unwrap_err() {
            IroncladError::Database(msg) => assert_eq!(msg, "test error"),
            other => panic!("Expected Database, got {:?}", other),
        }
    }
}
