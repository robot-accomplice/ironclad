use thiserror::Error;

#[derive(Debug, Error)]
pub enum IroncladError {
    #[error("config error: {0}")]
    Config(String),

    #[error("channel error: {0}")]
    Channel(String),

    #[error("database error: {0}")]
    Database(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("network error: {0}")]
    Network(String),

    #[error("policy violation: {rule} -- {reason}")]
    Policy { rule: String, reason: String },

    #[error("tool error: {tool} -- {message}")]
    Tool { tool: String, message: String },

    #[error("wallet error: {0}")]
    Wallet(String),

    #[error("injection detected: {0}")]
    Injection(String),

    #[error("schedule error: {0}")]
    Schedule(String),

    #[error("A2A error: {0}")]
    A2a(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("skill error: {0}")]
    Skill(String),

    #[error("keystore error: {0}")]
    Keystore(String),
}

impl From<toml::de::Error> for IroncladError {
    fn from(e: toml::de::Error) -> Self {
        Self::Config(e.to_string())
    }
}

impl From<serde_json::Error> for IroncladError {
    fn from(e: serde_json::Error) -> Self {
        Self::Config(format!("JSON parse error: {e}"))
    }
}

pub type Result<T> = std::result::Result<T, IroncladError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_variants() {
        let cases: Vec<(IroncladError, &str)> = vec![
            (
                IroncladError::Config("bad toml".into()),
                "config error: bad toml",
            ),
            (
                IroncladError::Channel("serialize failed".into()),
                "channel error: serialize failed",
            ),
            (
                IroncladError::Database("locked".into()),
                "database error: locked",
            ),
            (IroncladError::Llm("timeout".into()), "LLM error: timeout"),
            (
                IroncladError::Network("refused".into()),
                "network error: refused",
            ),
            (
                IroncladError::Policy {
                    rule: "financial".into(),
                    reason: "over limit".into(),
                },
                "policy violation: financial -- over limit",
            ),
            (
                IroncladError::Tool {
                    tool: "git".into(),
                    message: "not found".into(),
                },
                "tool error: git -- not found",
            ),
            (
                IroncladError::Wallet("no key".into()),
                "wallet error: no key",
            ),
            (
                IroncladError::Injection("override attempt".into()),
                "injection detected: override attempt",
            ),
            (
                IroncladError::Schedule("missed".into()),
                "schedule error: missed",
            ),
            (
                IroncladError::A2a("handshake failed".into()),
                "A2A error: handshake failed",
            ),
            (
                IroncladError::Skill("parse error".into()),
                "skill error: parse error",
            ),
            (
                IroncladError::Keystore("locked".into()),
                "keystore error: locked",
            ),
        ];

        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    #[test]
    fn io_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "missing");
        let err: IroncladError = io_err.into();
        assert!(matches!(err, IroncladError::Io(_)));
        assert!(err.to_string().contains("missing"));
    }

    #[test]
    fn toml_error_conversion() {
        let bad_toml = "[[invalid";
        let result: std::result::Result<toml::Value, _> = toml::from_str(bad_toml);
        let err: IroncladError = result.unwrap_err().into();
        assert!(matches!(err, IroncladError::Config(_)));
    }

    #[test]
    fn json_error_conversion() {
        let bad_json = "{invalid}";
        let result: std::result::Result<serde_json::Value, _> = serde_json::from_str(bad_json);
        let err: IroncladError = result.unwrap_err().into();
        assert!(matches!(err, IroncladError::Config(_)));
    }

    #[test]
    fn result_type_alias() {
        let ok: Result<i32> = Ok(42);
        assert!(matches!(ok, Ok(42)));

        let err: Result<i32> = Err(IroncladError::Config("test".into()));
        assert!(err.is_err());
    }
}
