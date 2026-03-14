use ironclad_core::error::{IroncladError, Result};

pub trait NetworkResultExt<T> {
    fn net_err(self) -> Result<T>;
}

impl<T, E: std::fmt::Display> NetworkResultExt<T> for std::result::Result<T, E> {
    fn net_err(self) -> Result<T> {
        self.map_err(|e| IroncladError::Network(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn net_result_ext_converts_error() {
        let err: std::result::Result<(), &str> = Err("connection refused");
        let converted = err.net_err();
        assert!(converted.is_err());
        match converted.unwrap_err() {
            IroncladError::Network(msg) => assert_eq!(msg, "connection refused"),
            other => panic!("Expected Network, got {:?}", other),
        }
    }
}
