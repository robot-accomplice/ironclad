#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreasuryConfig {
    #[serde(default = "default_per_payment_cap")]
    pub per_payment_cap: f64,
    #[serde(default = "default_hourly_limit")]
    pub hourly_transfer_limit: f64,
    #[serde(default = "default_daily_limit")]
    pub daily_transfer_limit: f64,
    #[serde(default = "default_min_reserve")]
    pub minimum_reserve: f64,
    #[serde(default = "default_inference_budget")]
    pub daily_inference_budget: f64,
    #[serde(default)]
    pub revenue_swap: RevenueSwapConfig,
}

impl Default for TreasuryConfig {
    fn default() -> Self {
        Self {
            per_payment_cap: default_per_payment_cap(),
            hourly_transfer_limit: default_hourly_limit(),
            daily_transfer_limit: default_daily_limit(),
            minimum_reserve: default_min_reserve(),
            daily_inference_budget: default_inference_budget(),
            revenue_swap: RevenueSwapConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfitTaxConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_profit_tax_rate")]
    pub rate: f64,
    #[serde(default)]
    pub destination_wallet: Option<String>,
}

impl Default for ProfitTaxConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            rate: default_profit_tax_rate(),
            destination_wallet: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SelfFundingConfig {
    #[serde(default)]
    pub tax: ProfitTaxConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueSwapChainConfig {
    pub chain: String,
    pub target_contract_address: String,
    #[serde(default)]
    pub swap_contract_address: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RevenueSwapConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_revenue_swap_target_symbol")]
    pub target_symbol: String,
    #[serde(default = "default_revenue_swap_default_chain")]
    pub default_chain: String,
    #[serde(default = "default_revenue_swap_chains")]
    pub chains: Vec<RevenueSwapChainConfig>,
}

impl Default for RevenueSwapConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            target_symbol: default_revenue_swap_target_symbol(),
            default_chain: default_revenue_swap_default_chain(),
            chains: default_revenue_swap_chains(),
        }
    }
}

fn default_revenue_swap_target_symbol() -> String {
    "PALM_USD".into()
}

fn default_revenue_swap_default_chain() -> String {
    "ETH".into()
}

fn default_revenue_swap_chains() -> Vec<RevenueSwapChainConfig> {
    vec![
        RevenueSwapChainConfig {
            chain: "ETH".into(),
            target_contract_address: "0xfaf0cee6b20e2aaa4b80748a6af4cd89609a3d78".into(),
            swap_contract_address: None,
        },
        RevenueSwapChainConfig {
            chain: "SOLANA".into(),
            target_contract_address: "9muem3X58Ztm2nimxEftLH4X9qSx6tJRd7njsdZuY1rQ".into(),
            swap_contract_address: None,
        },
        RevenueSwapChainConfig {
            chain: "BSC".into(),
            target_contract_address: "0xFAF0cEe6B20e2Aaa4B80748a6AF4CD89609a3d78".into(),
            swap_contract_address: None,
        },
    ]
}

fn default_profit_tax_rate() -> f64 {
    0.0
}

fn default_per_payment_cap() -> f64 {
    100.0
}
fn default_hourly_limit() -> f64 {
    500.0
}
fn default_daily_limit() -> f64 {
    2000.0
}
fn default_min_reserve() -> f64 {
    5.0
}
fn default_inference_budget() -> f64 {
    50.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YieldConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_yield_protocol")]
    pub protocol: String,
    #[serde(default = "default_yield_chain")]
    pub chain: String,
    #[serde(default = "default_min_deposit")]
    pub min_deposit: f64,
    #[serde(default = "default_withdrawal_threshold")]
    pub withdrawal_threshold: f64,
    /// RPC URL for yield chain (e.g. Base Sepolia). If unset, deposit/withdraw use mock behavior.
    #[serde(default)]
    pub chain_rpc_url: Option<String>,
    /// Aave V3 Pool address. Default: Base Sepolia.
    #[serde(default = "default_yield_pool_address")]
    pub pool_address: String,
    /// Underlying asset (e.g. USDC) address for supply/withdraw. Default: Base Sepolia USDC.
    #[serde(default = "default_yield_usdc_address")]
    pub usdc_address: String,
    /// aToken address for balance checks (e.g. aBase Sepolia USDC).
    /// When `None`, falls back to the Base Sepolia aUSDC default.
    #[serde(default)]
    pub atoken_address: Option<String>,
}

impl Default for YieldConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            protocol: default_yield_protocol(),
            chain: default_yield_chain(),
            min_deposit: default_min_deposit(),
            withdrawal_threshold: default_withdrawal_threshold(),
            chain_rpc_url: None,
            pool_address: default_yield_pool_address(),
            usdc_address: default_yield_usdc_address(),
            atoken_address: None,
        }
    }
}

fn default_yield_protocol() -> String {
    "aave".into()
}
fn default_yield_chain() -> String {
    "base".into()
}
fn default_min_deposit() -> f64 {
    50.0
}
fn default_withdrawal_threshold() -> f64 {
    30.0
}

/// Aave V3 Pool on Base Sepolia
fn default_yield_pool_address() -> String {
    "0x07eA79F68B2B3df564D0A34F8e19D9B1e339814b".into()
}
/// USDC on Base Sepolia
fn default_yield_usdc_address() -> String {
    "0x036CbD53842c5426634e7929541eC2318f3dCF7e".into()
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WalletConfig {
    #[serde(default = "default_wallet_path")]
    pub path: PathBuf,
    #[serde(default = "default_chain_id")]
    pub chain_id: u64,
    #[serde(default = "default_rpc_url")]
    pub rpc_url: String,
}

impl Default for WalletConfig {
    fn default() -> Self {
        Self {
            path: default_wallet_path(),
            chain_id: default_chain_id(),
            rpc_url: default_rpc_url(),
        }
    }
}

fn default_wallet_path() -> PathBuf {
    dirs_next().join("wallet.json")
}

fn default_chain_id() -> u64 {
    8453
}

fn default_rpc_url() -> String {
    "https://mainnet.base.org".into()
}
