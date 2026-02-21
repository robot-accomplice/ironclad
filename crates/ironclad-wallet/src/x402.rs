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
        let body = serde_json::json!({"amount": 1.0, "recipient": "0x123"});
        assert!(X402Handler::parse_payment_requirements(&body).is_err());
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
