//! Yield engine: Aave V3 supply/withdraw and aToken balance.
//! When `chain_rpc_url` is set, uses real Aave Pool on Base Sepolia; otherwise mock behavior for tests.

use alloy::primitives::{Address, U256};
use alloy::sol;
use ironclad_core::config::YieldConfig;
use ironclad_core::{IroncladError, Result};
use tracing::info;

sol! {
    #[sol(rpc)]
    interface IPool {
        function supply(address asset, uint256 amount, address onBehalfOf, uint16 referralCode) external;
        function withdraw(address asset, uint256 amount, address to) external returns (uint256);
    }
    #[sol(rpc)]
    interface IERC20 {
        function approve(address spender, uint256 amount) external returns (bool);
        function balanceOf(address account) external view returns (uint256);
    }
}

const USDC_SCALE: f64 = 1_000_000.0;

#[derive(Debug, Clone)]
pub struct YieldEngine {
    enabled: bool,
    protocol: String,
    chain: String,
    min_deposit: f64,
    withdrawal_threshold: f64,
    chain_rpc_url: Option<String>,
    pool_address: String,
    usdc_address: String,
    a_token_address: String,
}

impl YieldEngine {
    pub fn new(config: &YieldConfig) -> Self {
        Self {
            enabled: config.enabled,
            protocol: config.protocol.clone(),
            chain: config.chain.clone(),
            min_deposit: config.min_deposit,
            withdrawal_threshold: config.withdrawal_threshold,
            chain_rpc_url: config.chain_rpc_url.clone(),
            pool_address: config.pool_address.clone(),
            usdc_address: config.usdc_address.clone(),
            a_token_address: config
                .atoken_address
                .clone()
                .unwrap_or_else(|| "0x4d8e6968b67a2a216b2e5928b793663415377c2e".into()),
        }
    }

    pub fn enabled(&self) -> bool {
        self.enabled
    }

    pub fn protocol(&self) -> &str {
        &self.protocol
    }

    pub fn chain(&self) -> &str {
        &self.chain
    }

    pub fn min_deposit(&self) -> f64 {
        self.min_deposit
    }

    pub fn withdrawal_threshold(&self) -> f64 {
        self.withdrawal_threshold
    }

    pub fn chain_rpc_url(&self) -> Option<&str> {
        self.chain_rpc_url.as_deref()
    }

    pub fn pool_address(&self) -> &str {
        &self.pool_address
    }

    pub fn usdc_address(&self) -> &str {
        &self.usdc_address
    }

    pub fn a_token_address(&self) -> &str {
        &self.a_token_address
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

    /// Deposit USDC to Aave. When `chain_rpc_url` is set, `agent_address` and `private_key` must be
    /// provided to perform a real tx; otherwise mock tx hash is returned.
    pub async fn deposit(
        &self,
        amount: f64,
        agent_address: Option<&str>,
        private_key: Option<&[u8]>,
    ) -> Result<String> {
        if amount <= 0.0 {
            return Err(IroncladError::Wallet(format!(
                "deposit amount must be positive, got {amount}"
            )));
        }
        if !self.enabled {
            return Err(IroncladError::Wallet("yield engine is disabled".into()));
        }
        if self.chain_rpc_url.is_none() {
            info!(amount, protocol = %self.protocol, "mock deposit to yield protocol");
            return Ok(mock_tx_hash(amount));
        }
        let (agent_addr, key) = match (agent_address, private_key) {
            (Some(a), Some(k)) => (a, k),
            _ => {
                return Err(IroncladError::Wallet(
                    "chain_rpc_url is set but agent_address or private_key missing for deposit"
                        .into(),
                ));
            }
        };
        real_deposit(self, amount, agent_addr, key).await
    }

    /// Withdraw USDC from Aave. When `chain_rpc_url` is set, `agent_address` and `private_key` must be
    /// provided; otherwise mock tx hash is returned.
    pub async fn withdraw(
        &self,
        amount: f64,
        agent_address: Option<&str>,
        private_key: Option<&[u8]>,
    ) -> Result<String> {
        if amount <= 0.0 {
            return Err(IroncladError::Wallet(format!(
                "withdrawal amount must be positive, got {amount}"
            )));
        }
        if !self.enabled {
            return Err(IroncladError::Wallet("yield engine is disabled".into()));
        }
        if self.chain_rpc_url.is_none() {
            info!(amount, protocol = %self.protocol, "mock withdrawal from yield protocol");
            return Ok(mock_tx_hash(amount));
        }
        let (agent_addr, key) = match (agent_address, private_key) {
            (Some(a), Some(k)) => (a, k),
            _ => {
                return Err(IroncladError::Wallet(
                    "chain_rpc_url is set but agent_address or private_key missing for withdraw"
                        .into(),
                ));
            }
        };
        real_withdraw(self, amount, agent_addr, key).await
    }

    /// Returns aToken balance in USDC units (6 decimals). When no RPC configured, returns 0.0.
    pub async fn get_a_token_balance(&self, agent_address: &str) -> Result<f64> {
        let Some(rpc_url) = &self.chain_rpc_url else {
            return Ok(0.0);
        };
        real_a_token_balance(rpc_url, &self.a_token_address, agent_address).await
    }

    /// Builds the Aave Pool supply call params (for tests that verify construction without RPC).
    pub fn build_supply_call_params(
        &self,
        amount: f64,
        on_behalf_of: &str,
    ) -> Result<(Address, U256, Address, u16)> {
        let asset = parse_address(&self.usdc_address)?;
        let amount_raw = amount_to_raw(amount);
        let on_behalf = parse_address(on_behalf_of)?;
        Ok((asset, amount_raw, on_behalf, 0u16))
    }

    /// Builds the Aave Pool withdraw call params (for tests).
    pub fn build_withdraw_call_params(
        &self,
        amount: f64,
        to: &str,
    ) -> Result<(Address, U256, Address)> {
        let asset = parse_address(&self.usdc_address)?;
        let amount_raw = amount_to_raw(amount);
        let to_addr = parse_address(to)?;
        Ok((asset, amount_raw, to_addr))
    }
}

fn mock_tx_hash(amount: f64) -> String {
    // Use USDC scale (1e6) instead of 1e18 to avoid u64 saturation for amounts > ~18.4
    let scaled = (amount * USDC_SCALE).round().max(0.0);
    let amount_bits = if scaled > u64::MAX as f64 {
        u64::MAX
    } else {
        scaled as u64
    };
    format!(
        "0x{:016x}{:016x}",
        amount_bits,
        chrono::Utc::now().timestamp() as u64
    )
}

fn amount_to_raw(amount_usdc: f64) -> U256 {
    let scaled = (amount_usdc * USDC_SCALE).round().max(0.0);
    // Clamp to u64::MAX to avoid silent saturation on extreme values
    let clamped = if scaled > u64::MAX as f64 {
        u64::MAX
    } else {
        scaled as u64
    };
    U256::from(clamped)
}

fn parse_address(s: &str) -> Result<Address> {
    let s = s.trim_start_matches("0x");
    let bytes =
        hex::decode(s).map_err(|e| IroncladError::Wallet(format!("invalid address hex: {e}")))?;
    if bytes.len() != 20 {
        return Err(IroncladError::Wallet("address must be 20 bytes".into()));
    }
    let mut arr = [0u8; 20];
    arr.copy_from_slice(&bytes);
    Ok(Address::from(arr))
}

async fn real_deposit(
    engine: &YieldEngine,
    amount: f64,
    agent_address: &str,
    private_key: &[u8],
) -> Result<String> {
    use alloy::network::EthereumWallet;
    use alloy::providers::ProviderBuilder;
    use alloy::signers::local::PrivateKeySigner;

    let rpc_url = engine
        .chain_rpc_url
        .as_ref()
        .ok_or_else(|| IroncladError::Wallet("missing chain_rpc_url".into()))?;
    let pool_addr = parse_address(&engine.pool_address)?;
    let usdc_addr = parse_address(&engine.usdc_address)?;
    let on_behalf = parse_address(agent_address)?;
    let amount_raw = amount_to_raw(amount);

    let key_bytes: &[u8; 32] = private_key
        .try_into()
        .map_err(|_| IroncladError::Wallet("invalid private key length".into()))?;
    let signer: PrivateKeySigner = PrivateKeySigner::from_bytes(key_bytes.into())
        .map_err(|e| IroncladError::Wallet(format!("invalid private key: {e}")))?;
    let wallet = EthereumWallet::from(signer);
    // with_recommended_fillers() adds ChainIdFiller, NonceFiller, and GasFiller
    // for proper gas estimation and nonce management on-chain.
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            rpc_url
                .parse()
                .map_err(|e| IroncladError::Wallet(format!("invalid RPC URL: {e}")))?,
        );

    let pool = IPool::new(pool_addr, &provider);
    let erc20 = IERC20::new(usdc_addr, &provider);

    erc20
        .approve(pool_addr, amount_raw)
        .send()
        .await
        .map_err(|e| IroncladError::Wallet(format!("approve failed: {e}")))?
        .watch()
        .await
        .map_err(|e| IroncladError::Wallet(format!("approve receipt: {e}")))?;

    let tx_hash = pool
        .supply(usdc_addr, amount_raw, on_behalf, 0u16)
        .send()
        .await
        .map_err(|e| IroncladError::Wallet(format!("supply failed: {e}")))?
        .watch()
        .await
        .map_err(|e| IroncladError::Wallet(format!("supply receipt: {e}")))?;
    Ok(format!("{:?}", tx_hash))
}

async fn real_withdraw(
    engine: &YieldEngine,
    amount: f64,
    agent_address: &str,
    private_key: &[u8],
) -> Result<String> {
    use alloy::network::EthereumWallet;
    use alloy::providers::ProviderBuilder;
    use alloy::signers::local::PrivateKeySigner;

    let rpc_url = engine
        .chain_rpc_url
        .as_ref()
        .ok_or_else(|| IroncladError::Wallet("missing chain_rpc_url".into()))?;
    let pool_addr = parse_address(&engine.pool_address)?;
    let usdc_addr = parse_address(&engine.usdc_address)?;
    let to_addr = parse_address(agent_address)?;
    let amount_raw = amount_to_raw(amount);

    let key_bytes: &[u8; 32] = private_key
        .try_into()
        .map_err(|_| IroncladError::Wallet("invalid private key length".into()))?;
    let signer: PrivateKeySigner = PrivateKeySigner::from_bytes(key_bytes.into())
        .map_err(|e| IroncladError::Wallet(format!("invalid private key: {e}")))?;
    let wallet = EthereumWallet::from(signer);
    let provider = ProviderBuilder::new()
        .with_recommended_fillers()
        .wallet(wallet)
        .on_http(
            rpc_url
                .parse()
                .map_err(|e| IroncladError::Wallet(format!("invalid RPC URL: {e}")))?,
        );

    let pool = IPool::new(pool_addr, &provider);
    let tx_hash = pool
        .withdraw(usdc_addr, amount_raw, to_addr)
        .send()
        .await
        .map_err(|e| IroncladError::Wallet(format!("withdraw failed: {e}")))?
        .watch()
        .await
        .map_err(|e| IroncladError::Wallet(format!("withdraw receipt: {e}")))?;
    Ok(format!("{:?}", tx_hash))
}

async fn real_a_token_balance(rpc_url: &str, a_token_address: &str, account: &str) -> Result<f64> {
    use alloy::providers::ProviderBuilder;

    let provider = ProviderBuilder::new().on_http(
        rpc_url
            .parse()
            .map_err(|e| IroncladError::Wallet(format!("invalid RPC URL: {e}")))?,
    );
    let atoken = parse_address(a_token_address)?;
    let account_addr = parse_address(account)?;
    let contract = IERC20::new(atoken, &provider);
    let balance = contract
        .balanceOf(account_addr)
        .call()
        .await
        .map_err(|e| IroncladError::Wallet(format!("balanceOf failed: {e}")))?;
    let raw: u128 = balance
        ._0
        .try_into()
        .map_err(|_| IroncladError::Wallet("aToken balance exceeds u128::MAX".into()))?;
    Ok(raw as f64 / USDC_SCALE)
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
            chain_rpc_url: None,
            pool_address: "0x07eA79F68B2B3df564D0A34F8e19D9B1e339814b".into(),
            usdc_address: "0x036CbD53842c5426634e7929541eC2318f3dCF7e".into(),
            atoken_address: Some("0x4d8e6968b67a2a216b2e5928b793663415377c2e".into()),
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
    async fn deposit_returns_tx_hash_mock() {
        let engine = YieldEngine::new(&enabled_config());
        let tx = engine.deposit(100.0, None, None).await.unwrap();
        assert!(tx.starts_with("0x"));
    }

    #[tokio::test]
    async fn withdraw_returns_tx_hash_mock() {
        let engine = YieldEngine::new(&enabled_config());
        let tx = engine.withdraw(50.0, None, None).await.unwrap();
        assert!(tx.starts_with("0x"));
    }

    #[tokio::test]
    async fn deposit_disabled_errors() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(engine.deposit(100.0, None, None).await.is_err());
    }

    #[tokio::test]
    async fn withdraw_disabled_errors() {
        let engine = YieldEngine::new(&disabled_config());
        assert!(engine.withdraw(50.0, None, None).await.is_err());
    }

    #[tokio::test]
    async fn zero_amount_deposit_rejected() {
        let engine = YieldEngine::new(&enabled_config());
        let err = engine.deposit(0.0, None, None).await.unwrap_err();
        assert!(err.to_string().contains("positive"));
    }

    #[tokio::test]
    async fn zero_amount_withdraw_rejected() {
        let engine = YieldEngine::new(&enabled_config());
        let err = engine.withdraw(0.0, None, None).await.unwrap_err();
        assert!(err.to_string().contains("positive"));
    }

    #[tokio::test]
    async fn negative_amount_deposit_rejected() {
        let engine = YieldEngine::new(&enabled_config());
        let err = engine.deposit(-10.0, None, None).await.unwrap_err();
        assert!(err.to_string().contains("positive"));
    }

    #[tokio::test]
    async fn negative_amount_withdraw_rejected() {
        let engine = YieldEngine::new(&enabled_config());
        let err = engine.withdraw(-5.0, None, None).await.unwrap_err();
        assert!(err.to_string().contains("positive"));
    }

    #[test]
    fn build_supply_call_params_constructs_valid_aave_call() {
        let engine = YieldEngine::new(&enabled_config());
        let (asset, amount, on_behalf_of, referral_code) = engine
            .build_supply_call_params(100.5, "0x0000000000000000000000000000000000000001")
            .unwrap();
        assert_eq!(referral_code, 0);
        assert_eq!(amount, amount_to_raw(100.5));
        assert_eq!(asset, parse_address(engine.usdc_address()).unwrap());
        assert_eq!(
            on_behalf_of,
            parse_address("0x0000000000000000000000000000000000000001").unwrap()
        );
    }

    #[test]
    fn build_withdraw_call_params_constructs_valid_aave_call() {
        let engine = YieldEngine::new(&enabled_config());
        let (asset, amount, to) = engine
            .build_withdraw_call_params(50.25, "0x0000000000000000000000000000000000000002")
            .unwrap();
        assert_eq!(amount, amount_to_raw(50.25));
        assert_eq!(asset, parse_address(engine.usdc_address()).unwrap());
        assert_eq!(
            to,
            parse_address("0x0000000000000000000000000000000000000002").unwrap()
        );
    }

    #[tokio::test]
    async fn get_a_token_balance_no_rpc_returns_zero() {
        let engine = YieldEngine::new(&enabled_config());
        let bal = engine
            .get_a_token_balance("0x0000000000000000000000000000000000000001")
            .await
            .unwrap();
        assert!((bal - 0.0).abs() < f64::EPSILON);
    }

    fn rpc_config() -> YieldConfig {
        YieldConfig {
            chain_rpc_url: Some("http://localhost:8545".into()),
            ..enabled_config()
        }
    }

    #[tokio::test]
    async fn deposit_rpc_set_missing_agent_address() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine.deposit(10.0, None, Some(&[0u8; 32])).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[tokio::test]
    async fn deposit_rpc_set_missing_private_key() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine
            .deposit(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn withdraw_rpc_set_missing_agent_address() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine.withdraw(10.0, None, Some(&[0u8; 32])).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing"));
    }

    #[tokio::test]
    async fn withdraw_rpc_set_missing_private_key() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine
            .withdraw(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                None,
            )
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn parse_address_invalid_hex() {
        assert!(parse_address("0xZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZZ").is_err());
    }

    #[test]
    fn parse_address_wrong_length() {
        assert!(parse_address("0xdead").is_err());
    }

    #[test]
    fn amount_to_raw_zero() {
        assert_eq!(amount_to_raw(0.0), U256::from(0u64));
    }

    #[test]
    fn amount_to_raw_fractional() {
        assert_eq!(amount_to_raw(1.5), U256::from(1_500_000u64));
    }

    #[test]
    fn amount_to_raw_large_value() {
        assert_eq!(amount_to_raw(1_000_000.0), U256::from(1_000_000_000_000u64));
    }

    #[test]
    fn build_supply_call_params_invalid_address() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(
            engine
                .build_supply_call_params(10.0, "not-an-address")
                .is_err()
        );
    }

    #[test]
    fn build_withdraw_call_params_invalid_address() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(
            engine
                .build_withdraw_call_params(10.0, "not-an-address")
                .is_err()
        );
    }

    #[test]
    fn mock_tx_hash_format() {
        let hash = mock_tx_hash(100.0);
        assert!(hash.starts_with("0x"));
        assert!(hash.len() > 2);
    }

    #[test]
    fn mock_tx_hash_different_amounts_differ() {
        let h1 = mock_tx_hash(1.0);
        let h2 = mock_tx_hash(2.0);
        assert_ne!(h1, h2);
    }

    #[test]
    fn new_defaults_atoken_address_when_none() {
        let mut cfg = enabled_config();
        cfg.atoken_address = None;
        let engine = YieldEngine::new(&cfg);
        assert!(!engine.a_token_address().is_empty());
    }

    #[test]
    fn amount_to_raw_negative_clamps_to_zero() {
        assert_eq!(amount_to_raw(-100.0), U256::from(0u64));
    }

    #[test]
    fn parse_address_valid_with_prefix() {
        let addr = parse_address("0x0000000000000000000000000000000000000001");
        assert!(addr.is_ok());
    }

    #[test]
    fn parse_address_empty() {
        assert!(parse_address("").is_err());
    }

    #[test]
    fn parse_address_too_long() {
        assert!(parse_address("0x00000000000000000000000000000000000000000001").is_err());
    }

    #[test]
    fn calculate_excess_zero_balance_zero_reserve() {
        let engine = YieldEngine::new(&enabled_config());
        let excess = engine.calculate_excess(0.0, 0.0);
        assert!((excess - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn should_deposit_zero_excess() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(!engine.should_deposit(0.0));
    }

    #[test]
    fn should_withdraw_just_below_threshold() {
        let engine = YieldEngine::new(&enabled_config());
        assert!(engine.should_withdraw(29.99));
    }

    #[test]
    fn new_engine_copies_all_config_fields() {
        let cfg = enabled_config();
        let engine = YieldEngine::new(&cfg);
        assert!(engine.enabled());
        assert_eq!(engine.protocol(), "aave");
        assert_eq!(engine.chain(), "base");
        assert!((engine.min_deposit() - 50.0).abs() < f64::EPSILON);
        assert!((engine.withdrawal_threshold() - 30.0).abs() < f64::EPSILON);
        assert!(engine.chain_rpc_url().is_none());
        assert!(!engine.pool_address().is_empty());
        assert!(!engine.usdc_address().is_empty());
        assert!(!engine.a_token_address().is_empty());
    }

    #[test]
    fn amount_to_raw_rounding() {
        let raw = amount_to_raw(0.000001);
        assert_eq!(raw, U256::from(1u64));
    }

    #[tokio::test]
    async fn deposit_rpc_set_both_missing() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine.deposit(10.0, None, None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn withdraw_rpc_set_both_missing() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine.withdraw(10.0, None, None).await;
        assert!(result.is_err());
    }

    #[test]
    fn build_supply_call_params_zero_amount() {
        let engine = YieldEngine::new(&enabled_config());
        let (_, amount, _, _) = engine
            .build_supply_call_params(0.0, "0x0000000000000000000000000000000000000001")
            .unwrap();
        assert_eq!(amount, U256::from(0u64));
    }

    #[test]
    fn build_withdraw_call_params_zero_amount() {
        let engine = YieldEngine::new(&enabled_config());
        let (_, amount, _) = engine
            .build_withdraw_call_params(0.0, "0x0000000000000000000000000000000000000001")
            .unwrap();
        assert_eq!(amount, U256::from(0u64));
    }

    #[test]
    fn calculate_excess_fractional() {
        let engine = YieldEngine::new(&enabled_config());
        let excess = engine.calculate_excess(150.5, 100.0);
        let expected = 150.5 - 100.0 - 10.0;
        assert!((excess - expected).abs() < 1e-10);
    }

    // --- real_deposit / real_withdraw error paths ---

    #[tokio::test]
    async fn real_deposit_invalid_private_key_length() {
        let engine = YieldEngine::new(&rpc_config());
        // 16-byte key is too short (must be 32)
        let short_key = [0u8; 16];
        let result = engine
            .deposit(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&short_key),
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid private key")
        );
    }

    #[tokio::test]
    async fn real_withdraw_invalid_private_key_length() {
        let engine = YieldEngine::new(&rpc_config());
        let short_key = [0u8; 16];
        let result = engine
            .withdraw(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&short_key),
            )
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("invalid private key")
        );
    }

    fn rpc_config_with_invalid_pool() -> YieldConfig {
        YieldConfig {
            chain_rpc_url: Some("http://localhost:8545".into()),
            pool_address: "not-valid".into(),
            ..enabled_config()
        }
    }

    #[tokio::test]
    async fn real_deposit_invalid_pool_address() {
        let engine = YieldEngine::new(&rpc_config_with_invalid_pool());
        let key = [1u8; 32];
        let result = engine
            .deposit(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key),
            )
            .await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("invalid address") || err_msg.contains("invalid private key"),
            "unexpected error: {err_msg}"
        );
    }

    #[tokio::test]
    async fn real_withdraw_invalid_pool_address() {
        let engine = YieldEngine::new(&rpc_config_with_invalid_pool());
        let key = [1u8; 32];
        let result = engine
            .withdraw(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key),
            )
            .await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("invalid address") || err_msg.contains("invalid private key"),
            "unexpected error: {err_msg}"
        );
    }

    fn rpc_config_with_invalid_usdc() -> YieldConfig {
        YieldConfig {
            chain_rpc_url: Some("http://localhost:8545".into()),
            usdc_address: "0xZZZZ".into(),
            ..enabled_config()
        }
    }

    #[tokio::test]
    async fn real_deposit_invalid_usdc_address() {
        let engine = YieldEngine::new(&rpc_config_with_invalid_usdc());
        let key = [1u8; 32];
        let result = engine
            .deposit(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn real_withdraw_invalid_usdc_address() {
        let engine = YieldEngine::new(&rpc_config_with_invalid_usdc());
        let key = [1u8; 32];
        let result = engine
            .withdraw(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key),
            )
            .await;
        assert!(result.is_err());
    }

    fn rpc_config_with_bad_rpc_url() -> YieldConfig {
        YieldConfig {
            chain_rpc_url: Some("not a url at all".into()),
            ..enabled_config()
        }
    }

    #[tokio::test]
    async fn real_deposit_invalid_rpc_url() {
        let engine = YieldEngine::new(&rpc_config_with_bad_rpc_url());
        // Use a valid secp256k1 private key
        let signing_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes();
        let result = engine
            .deposit(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key_bytes),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid RPC URL"));
    }

    #[tokio::test]
    async fn real_withdraw_invalid_rpc_url() {
        let engine = YieldEngine::new(&rpc_config_with_bad_rpc_url());
        let signing_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes();
        let result = engine
            .withdraw(
                10.0,
                Some("0x0000000000000000000000000000000000000001"),
                Some(&key_bytes),
            )
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid RPC URL"));
    }

    // --- real_deposit with invalid agent_address ---

    #[tokio::test]
    async fn real_deposit_invalid_agent_address() {
        let engine = YieldEngine::new(&rpc_config());
        let signing_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes();
        let result = engine
            .deposit(10.0, Some("not-an-address"), Some(&key_bytes))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn real_withdraw_invalid_agent_address() {
        let engine = YieldEngine::new(&rpc_config());
        let signing_key =
            k256::ecdsa::SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes();
        let result = engine
            .withdraw(10.0, Some("not-an-address"), Some(&key_bytes))
            .await;
        assert!(result.is_err());
    }

    // --- get_a_token_balance with RPC set but invalid address ---

    #[tokio::test]
    async fn get_a_token_balance_invalid_rpc_url() {
        let mut cfg = rpc_config();
        cfg.chain_rpc_url = Some("not a url".into());
        let engine = YieldEngine::new(&cfg);
        let result = engine
            .get_a_token_balance("0x0000000000000000000000000000000000000001")
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("invalid RPC URL"));
    }

    #[tokio::test]
    async fn get_a_token_balance_invalid_atoken_address() {
        let mut cfg = rpc_config();
        cfg.atoken_address = Some("invalid".into());
        let engine = YieldEngine::new(&cfg);
        let result = engine
            .get_a_token_balance("0x0000000000000000000000000000000000000001")
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn get_a_token_balance_invalid_account_address() {
        let engine = YieldEngine::new(&rpc_config());
        let result = engine.get_a_token_balance("not-an-address").await;
        assert!(result.is_err());
    }

    // --- parse_address with no 0x prefix but valid hex ---

    #[test]
    fn parse_address_valid_without_prefix() {
        let addr = parse_address("0000000000000000000000000000000000000001");
        assert!(addr.is_ok());
    }

    // --- amount_to_raw edge cases ---

    #[test]
    fn amount_to_raw_tiny_fraction() {
        // 0.000001 USDC = 1 raw unit
        assert_eq!(amount_to_raw(0.000001), U256::from(1u64));
    }

    #[test]
    fn amount_to_raw_exact_one() {
        assert_eq!(amount_to_raw(1.0), U256::from(1_000_000u64));
    }

    // --- YieldEngine Debug ---

    #[test]
    fn yield_engine_debug_includes_fields() {
        let engine = YieldEngine::new(&enabled_config());
        let debug_str = format!("{:?}", engine);
        assert!(debug_str.contains("enabled"));
        assert!(debug_str.contains("aave"));
        assert!(debug_str.contains("base"));
    }

    // --- YieldEngine Clone ---

    #[test]
    fn yield_engine_clone_preserves_fields() {
        let engine = YieldEngine::new(&enabled_config());
        let cloned = engine.clone();
        assert_eq!(engine.enabled(), cloned.enabled());
        assert_eq!(engine.protocol(), cloned.protocol());
        assert_eq!(engine.chain(), cloned.chain());
        assert!((engine.min_deposit() - cloned.min_deposit()).abs() < f64::EPSILON);
        assert!(
            (engine.withdrawal_threshold() - cloned.withdrawal_threshold()).abs() < f64::EPSILON
        );
    }
}
