pub mod money;
pub mod treasury;
pub mod wallet;
pub mod x402;
pub mod yield_engine;

pub use money::Money;
pub use treasury::TreasuryPolicy;
pub use wallet::Wallet;
pub use x402::{PaymentRequirements, X402Handler};
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
