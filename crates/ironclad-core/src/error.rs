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

impl From<toml::ser::Error> for IroncladError {
    fn from(e: toml::ser::Error) -> Self {
        Self::Config(format!("TOML serialization error: {e}"))
    }
}

impl From<serde_json::Error> for IroncladError {
    fn from(e: serde_json::Error) -> Self {
        Self::Config(format!("JSON parse error: {e}"))
    }
}

impl IroncladError {
    /// Returns `true` when the error indicates a credit, billing, or
    /// quota-exhaustion problem that won't resolve by retrying quickly.
    pub fn is_credit_error(&self) -> bool {
        let msg = match self {
            Self::Llm(m) | Self::Network(m) => m,
            _ => return false,
        };
        let lower = msg.to_ascii_lowercase();
        lower.contains("402 payment required")
            || (lower.contains("credit") && lower.contains("rate_limit"))
            || (lower.contains("credit") && lower.contains("circuit breaker"))
            || lower.contains("billing")
            || lower.contains("insufficient_quota")
            || lower.contains("exceeded your current quota")
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

    #[test]
    fn is_credit_error_detects_proxy_circuit_breaker() {
        let err = IroncladError::Llm(
            r#"provider returned 429 Too Many Requests: {"error": {"message": "Rate limited — proxy circuit breaker for anthropic (credit)", "type": "rate_limit_error"}}"#.into(),
        );
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_detects_402() {
        let err = IroncladError::Llm(
            "provider returned 402 Payment Required: insufficient credits".into(),
        );
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_detects_billing() {
        let err = IroncladError::Llm(
            r#"provider returned 403: {"error": {"message": "Your billing account is inactive"}}"#
                .into(),
        );
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_detects_quota_exhaustion() {
        let err = IroncladError::Llm(
            r#"provider returned 429: {"error": {"message": "You exceeded your current quota"}}"#
                .into(),
        );
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_detects_insufficient_quota() {
        let err = IroncladError::Llm(
            r#"provider returned 429: {"error": {"type": "insufficient_quota"}}"#.into(),
        );
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_false_for_transient_rate_limit() {
        let err = IroncladError::Llm(
            "provider returned 429 Too Many Requests: rate limited, try again".into(),
        );
        assert!(!err.is_credit_error());
    }

    #[test]
    fn is_credit_error_false_for_non_llm_variants() {
        let err = IroncladError::Config("credit billing".into());
        assert!(!err.is_credit_error());
    }

    #[test]
    fn is_credit_error_works_on_network_variant() {
        let err =
            IroncladError::Network("provider returned 402 Payment Required: no credits".into());
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_false_for_other_variants() {
        // These variants short-circuit in the match arm returning false
        assert!(!IroncladError::Database("credit billing".into()).is_credit_error());
        assert!(!IroncladError::Channel("credit rate_limit".into()).is_credit_error());
        assert!(!IroncladError::Wallet("billing issue".into()).is_credit_error());
        assert!(!IroncladError::Injection("402 Payment Required".into()).is_credit_error());
        assert!(!IroncladError::Schedule("billing".into()).is_credit_error());
        assert!(!IroncladError::A2a("billing".into()).is_credit_error());
        assert!(!IroncladError::Skill("billing".into()).is_credit_error());
        assert!(!IroncladError::Keystore("billing".into()).is_credit_error());
        assert!(
            !IroncladError::Policy {
                rule: "credit".into(),
                reason: "billing".into()
            }
            .is_credit_error()
        );
        assert!(
            !IroncladError::Tool {
                tool: "credit".into(),
                message: "billing".into()
            }
            .is_credit_error()
        );
        let io_err = std::io::Error::new(std::io::ErrorKind::Other, "billing");
        assert!(!IroncladError::Io(io_err).is_credit_error());
    }

    #[test]
    fn is_credit_error_network_billing() {
        let err = IroncladError::Network("Your billing account is inactive".into());
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_credit_rate_limit_combo() {
        let err = IroncladError::Llm("credit exhausted, rate_limit triggered".into());
        assert!(err.is_credit_error());
    }

    #[test]
    fn is_credit_error_credit_circuit_breaker_combo() {
        let err = IroncladError::Llm("credit tripped circuit breaker".into());
        assert!(err.is_credit_error());
    }

    #[test]
    fn toml_ser_error_conversion() {
        // Force a TOML serialization error using a custom serializer that always fails
        fn force_toml_ser_error<S: serde::Serializer>(
            _v: &str,
            _s: S,
        ) -> std::result::Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("forced error"))
        }
        #[derive(serde::Serialize)]
        struct Bad {
            #[serde(serialize_with = "force_toml_ser_error")]
            field: String,
        }
        let result = toml::to_string(&Bad {
            field: "x".into(),
        });
        let err: IroncladError = result.unwrap_err().into();
        assert!(matches!(err, IroncladError::Config(_)));
        assert!(err.to_string().contains("TOML serialization error"));
    }
}
