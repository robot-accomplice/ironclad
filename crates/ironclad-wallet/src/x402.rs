use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::wallet::Wallet;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaymentRequirements {
    pub amount: f64,
    pub recipient: String,
    pub chain_id: u64,
}

#[derive(Debug, Clone)]
pub struct X402Handler;

impl X402Handler {
    pub fn new() -> Self {
        Self
    }

    pub fn build_payment_header(amount: f64, recipient: &str, authorization: &str) -> String {
        format!("x402 amount={amount} recipient={recipient} auth={authorization}")
    }

    pub async fn handle_402(response_body: &serde_json::Value, wallet: &Wallet) -> Result<String> {
        let requirements = Self::parse_payment_requirements(response_body)?;
        debug!(
            amount = requirements.amount,
            recipient = %requirements.recipient,
            "handling 402 payment"
        );

        let auth_message = format!(
            "pay:{}:{}:{}",
            requirements.amount, requirements.recipient, requirements.chain_id
        );
        let authorization = wallet.sign_message(&auth_message).await?;

        Ok(Self::build_payment_header(
            requirements.amount,
            &requirements.recipient,
            &authorization,
        ))
    }

    pub fn parse_payment_requirements(body: &serde_json::Value) -> Result<PaymentRequirements> {
        let amount = body.get("amount").and_then(|v| v.as_f64()).ok_or_else(|| {
            IroncladError::Wallet("missing or invalid 'amount' in payment requirements".into())
        })?;

        let recipient = body
            .get("recipient")
            .and_then(|v| v.as_str())
            .ok_or_else(|| {
                IroncladError::Wallet(
                    "missing or invalid 'recipient' in payment requirements".into(),
                )
            })?
            .to_string();

        if !is_valid_eth_address(&recipient) {
            return Err(IroncladError::Wallet(format!(
                "invalid recipient address format: must start with '0x' followed by 40 hex characters, got '{recipient}'"
            )));
        }

        let chain_id = body
            .get("chain_id")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| {
                IroncladError::Wallet(
                    "missing or invalid 'chain_id' in payment requirements".into(),
                )
            })?;

        Ok(PaymentRequirements {
            amount,
            recipient,
            chain_id,
        })
    }
}

impl Default for X402Handler {
    fn default() -> Self {
        Self::new()
    }
}

/// Validates that a string is a well-formed Ethereum address: starts with "0x"
/// followed by exactly 40 hexadecimal characters.
fn is_valid_eth_address(addr: &str) -> bool {
    addr.len() == 42 && addr.starts_with("0x") && addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclad_core::config::WalletConfig;
    use tempfile::TempDir;

    fn sample_body() -> serde_json::Value {
        serde_json::json!({
            "amount": 0.05,
            "recipient": "0xabcdef1234567890abcdef1234567890abcdef12",
            "chain_id": 8453
        })
    }

    #[test]
    fn build_payment_header_format() {
        let header = X402Handler::build_payment_header(0.05, "0xrecipient", "0xsignature");
        assert!(header.contains("amount=0.05"));
        assert!(header.contains("recipient=0xrecipient"));
        assert!(header.contains("auth=0xsignature"));
    }

    #[test]
    fn parse_payment_requirements_valid() {
        let body = sample_body();
        let req = X402Handler::parse_payment_requirements(&body).unwrap();
        assert!((req.amount - 0.05).abs() < f64::EPSILON);
        assert_eq!(req.recipient, "0xabcdef1234567890abcdef1234567890abcdef12");
        assert_eq!(req.chain_id, 8453);
    }

    #[test]
    fn parse_payment_requirements_missing_amount() {
        let body = serde_json::json!({"recipient": "0x123", "chain_id": 1});
        assert!(X402Handler::parse_payment_requirements(&body).is_err());
    }

    #[test]
    fn parse_payment_requirements_missing_recipient() {
        let body = serde_json::json!({"amount": 1.0, "chain_id": 1});
        assert!(X402Handler::parse_payment_requirements(&body).is_err());
    }

    #[test]
    fn parse_payment_requirements_missing_chain_id() {
        let body = serde_json::json!({"amount": 1.0, "recipient": "0xabcdef1234567890abcdef1234567890abcdef12"});
        assert!(X402Handler::parse_payment_requirements(&body).is_err());
    }

    #[test]
    fn parse_payment_requirements_invalid_recipient_too_short() {
        let body = serde_json::json!({"amount": 1.0, "recipient": "0x123", "chain_id": 1});
        let err = X402Handler::parse_payment_requirements(&body).unwrap_err();
        assert!(err.to_string().contains("invalid recipient address format"));
    }

    #[test]
    fn parse_payment_requirements_invalid_recipient_no_prefix() {
        let body = serde_json::json!({"amount": 1.0, "recipient": "abcdef1234567890abcdef1234567890abcdef12ab", "chain_id": 1});
        let err = X402Handler::parse_payment_requirements(&body).unwrap_err();
        assert!(err.to_string().contains("invalid recipient address format"));
    }

    #[test]
    fn parse_payment_requirements_invalid_recipient_non_hex() {
        let body = serde_json::json!({"amount": 1.0, "recipient": "0xZZZZZZ1234567890abcdef1234567890abcdef12", "chain_id": 1});
        let err = X402Handler::parse_payment_requirements(&body).unwrap_err();
        assert!(err.to_string().contains("invalid recipient address format"));
    }

    #[tokio::test]
    async fn handle_402_flow() {
        let dir = TempDir::new().unwrap();
        let config = WalletConfig {
            path: dir.path().join("wallet.json"),
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".into(),
        };
        let wallet = Wallet::load_or_generate(&config).await.unwrap();
        let body = sample_body();

        let header = X402Handler::handle_402(&body, &wallet).await.unwrap();
        assert!(header.starts_with("x402 "));
        assert!(header.contains("amount=0.05"));
        assert!(header.contains("auth=0x"));
    }
}
