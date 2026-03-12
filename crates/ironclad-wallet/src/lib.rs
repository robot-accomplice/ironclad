//! # ironclad-wallet
//!
//! Ethereum wallet management for the Ironclad agent runtime. Provides HD
//! wallet generation, on-chain balance tracking, x402 payment protocol
//! (EIP-3009), treasury policy enforcement, and DeFi yield optimization.
//!
//! ## Key Types
//!
//! - [`WalletService`] -- Top-level facade composing wallet, treasury, and yield engine
//! - [`Wallet`] -- HD wallet with key loading/generation
//! - [`TreasuryPolicy`] -- Spending limits and survival-tier-aware caps
//! - [`YieldEngine`] -- DeFi yield optimization (Aave/Compound on Base)
//! - [`X402Handler`] -- x402 payment protocol handler
//! - [`Money`] -- USDC amount type with formatting
//!
//! ## Modules
//!
//! - `wallet` -- Wallet loading, generation, address, balance
//! - `treasury` -- Treasury policy engine with per-payment caps and reserves
//! - `yield_engine` -- DeFi protocol integration for idle capital
//! - `x402` -- EIP-3009 `transferWithAuthorization` payment flow
//! - `money` -- USDC amount type and arithmetic

pub mod evm_submit;
pub mod money;
pub mod treasury;
pub mod wallet;
pub mod x402;
pub mod yield_engine;

pub use evm_submit::{
    EvmContractCall, get_evm_transaction_receipt_status, submit_evm_contract_call,
};
pub use money::Money;
pub use treasury::TreasuryPolicy;
pub use wallet::{TokenBalance, Wallet};
pub use x402::{PaymentRequirements, WalletPaymentHandler, X402Handler};
pub use yield_engine::YieldEngine;

use ironclad_core::Result;
use ironclad_core::config::IroncladConfig;

pub struct WalletService {
    pub wallet: Wallet,
    pub treasury: TreasuryPolicy,
    pub yield_engine: YieldEngine,
}

impl WalletService {
    pub async fn new(config: &IroncladConfig) -> Result<Self> {
        let wallet = Wallet::load_or_generate(&config.wallet).await?;
        let treasury = TreasuryPolicy::new(&config.treasury);
        let yield_engine = YieldEngine::new(&config.r#yield);

        Ok(Self {
            wallet,
            treasury,
            yield_engine,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL_TOML: &str = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
"#;

    #[tokio::test]
    async fn wallet_service_new_with_temp_wallet_path() {
        let dir = tempfile::tempdir().expect("temp dir");
        let wallet_path = dir.path().join("wallet.json");

        let mut config = IroncladConfig::from_str(MINIMAL_TOML).expect("parse config");
        config.wallet.path = wallet_path;

        let service = WalletService::new(&config)
            .await
            .expect("WalletService::new");
        assert!(!service.wallet.address().is_empty());
        assert_eq!(service.wallet.chain_id(), 8453);
    }

    #[tokio::test]
    async fn wallet_service_new_uses_treasury_and_yield_from_config() {
        let dir = tempfile::tempdir().expect("temp dir");
        let mut config = IroncladConfig::from_str(MINIMAL_TOML).expect("parse config");
        config.wallet.path = dir.path().join("wallet.json");

        let service = WalletService::new(&config)
            .await
            .expect("WalletService::new");
        let _ = &service.treasury;
        let _ = &service.yield_engine;
    }
}
