use ironclad_core::config::YieldConfig;
use ironclad_core::{IroncladError, Result};
use tracing::info;

#[derive(Debug, Clone)]
pub struct YieldEngine {
    pub enabled: bool,
    pub protocol: String,
    pub chain: String,
    pub min_deposit: f64,
    pub withdrawal_threshold: f64,
}

impl YieldEngine {
    pub fn new(config: &YieldConfig) -> Self {
        Self {
            enabled: config.enabled,
            protocol: config.protocol.clone(),
            chain: config.chain.clone(),
            min_deposit: config.min_deposit,
            withdrawal_threshold: config.withdrawal_threshold,
        }
    }

    /// Excess = balance - minimum_reserve - operational_buffer (10% of minimum_reserve).
    /// Returns 0.0 if negative.
    pub fn calculate_excess(&self, balance: f64, minimum_reserve: f64) -> f64 {
        let operational_buffer = minimum_reserve * 0.1;
        let excess = balance - minimum_reserve - operational_buffer;
        excess.max(0.0)
    }

    pub fn should_deposit(&self, excess: f64) -> bool {
        self.enabled && excess > self.min_deposit
    }

    pub fn should_withdraw(&self, balance: f64) -> bool {
        self.enabled && balance < self.withdrawal_threshold
    }

    pub async fn deposit(&self, amount: f64) -> Result<String> {
        if !self.enabled {
            return Err(IroncladError::Wallet("yield engine is disabled".into()));
        }
        info!(amount, protocol = %self.protocol, "mock deposit to yield protocol");
        let tx_hash = format!(
            "0x{:016x}{:016x}",
            (amount * 1e18) as u64,
            chrono::Utc::now().timestamp() as u64
        );
        Ok(tx_hash)
    }

    pub async fn withdraw(&self, amount: f64) -> Result<String> {
        if !self.enabled {
            return Err(IroncladError::Wallet("yield engine is disabled".into()));
        }
        info!(amount, protocol = %self.protocol, "mock withdrawal from yield protocol");
        let tx_hash = format!(
            "0x{:016x}{:016x}",
            (amount * 1e18) as u64,
            chrono::Utc::now().timestamp() as u64
        );
        Ok(tx_hash)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled_config() -> YieldConfig {
        YieldConfig {
            enabled: true,
            protocol: "aave".into(),
            chain: "base".into(),
            min_deposit: 50.0,
            withdrawal_threshold: 30.0,
        }
    }

    fn disabled_config() -> YieldConfig {
        YieldConfig {
            enabled: false,
            ..enabled_config()
        }
    }

    #[test]
    fn calculate_excess_positive() {
        let engine = YieldEngine::new(&enabled_config());
        // balance=200, reserve=100 → buffer=10 → excess=90
        let excess = engine.calculate_excess(200.0, 100.0);
        assert!((excess - 90.0).abs() < f64::EPSILON);
    }

    #[test]
    fn calculate_excess_zero_when_insufficient() {
        let engine = YieldEngine::new(&enabled_config());
        // balance=105, reserve=100 → buffer=10 → excess would be -5 → 0
        let excess = engine.calculate_excess(105.0, 100.0);
        assert!((excess - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn calculate_excess_exact_boundary() {
        let engine = YieldEngine::new(&enabled_config());
        // balance=110, reserve=100 → buffer=10 → excess=0
        let excess = engine.calculate_excess(110.0, 100.0);
        assert!((excess - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn should_deposit_when_excess_above_min() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(engine.should_deposit(50.01));
        assert!(!engine.should_deposit(50.0));
        assert!(!engine.should_deposit(49.0));
    }

    #[test]
    fn should_deposit_disabled() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(!engine.should_deposit(1000.0));
    }

    #[test]
    fn should_withdraw_when_balance_low() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(engine.should_withdraw(29.0));
        assert!(!engine.should_withdraw(30.0));
        assert!(!engine.should_withdraw(100.0));
    }

    #[test]
    fn should_withdraw_disabled() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(!engine.should_withdraw(0.0));
    }

    #[tokio::test]
    async fn deposit_returns_tx_hash() {
        let engine = YieldEngine::new(&enabled_config());
        let tx = engine.deposit(100.0).await.unwrap();
        assert!(tx.starts_with("0x"));
    }

    #[tokio::test]
    async fn withdraw_returns_tx_hash() {
        let engine = YieldEngine::new(&enabled_config());
        let tx = engine.withdraw(50.0).await.unwrap();
        assert!(tx.starts_with("0x"));
    }

    #[tokio::test]
    async fn deposit_disabled_errors() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(engine.deposit(100.0).await.is_err());
    }

    #[tokio::test]
    async fn withdraw_disabled_errors() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(engine.withdraw(50.0).await.is_err());
    }
}
