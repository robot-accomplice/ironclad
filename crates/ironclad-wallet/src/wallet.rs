use std::path::PathBuf;

use ironclad_core::config::WalletConfig;
use ironclad_core::{IroncladError, Result};
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use tracing::{debug, info};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalletFile {
    address: String,
    chain_id: u64,
    private_key_hex: String,
}

#[derive(Debug, Clone)]
pub struct Wallet {
    address: String,
    private_key: Vec<u8>,
    private_key_path: PathBuf,
    chain_id: u64,
    rpc_url: String,
}

fn eth_address_from_public_key(signing_key: &SigningKey) -> String {
    let verify_key = signing_key.verifying_key();
    let encoded = verify_key.to_encoded_point(false);
    let public_bytes = &encoded.as_bytes()[1..];
    let hash = Keccak256::digest(public_bytes);
    let addr_bytes = &hash[12..];
    format!("0x{}", hex::encode(addr_bytes))
}

impl Wallet {
    pub async fn load_or_generate(config: &WalletConfig) -> Result<Self> {
        let wallet_path = &config.path;

        if let Some(parent) = wallet_path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                IroncladError::Wallet(format!("failed to create wallet directory: {e}"))
            })?;
        }

        if wallet_path.exists() {
            debug!(?wallet_path, "loading existing wallet");
            let contents = tokio::fs::read_to_string(wallet_path)
                .await
                .map_err(|e| IroncladError::Wallet(format!("failed to read wallet file: {e}")))?;

            match serde_json::from_str::<WalletFile>(&contents) {
                Ok(wallet_file) => {
                    let key_bytes = hex::decode(&wallet_file.private_key_hex).map_err(|e| {
                        IroncladError::Wallet(format!("invalid private key hex: {e}"))
                    })?;
                    let signing_key = SigningKey::from_slice(&key_bytes).map_err(|e| {
                        IroncladError::Wallet(format!("invalid private key: {e}"))
                    })?;

                    let derived_addr = eth_address_from_public_key(&signing_key);
                    if derived_addr != wallet_file.address {
                        return Err(IroncladError::Wallet(
                            "wallet file address does not match derived address".into(),
                        ));
                    }

                    return Ok(Self {
                        address: wallet_file.address,
                        private_key: key_bytes,
                        private_key_path: wallet_path.clone(),
                        chain_id: config.chain_id,
                        rpc_url: config.rpc_url.clone(),
                    });
                }
                Err(_) => {
                    info!(?wallet_path, "legacy wallet file detected, regenerating with real keypair");
                }
            }
        }

        {
            info!(?wallet_path, "generating new wallet keypair");
            let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
            let key_bytes = signing_key.to_bytes().to_vec();
            let address = eth_address_from_public_key(&signing_key);

            let wallet_file = WalletFile {
                address: address.clone(),
                chain_id: config.chain_id,
                private_key_hex: hex::encode(&key_bytes),
            };
            let contents = serde_json::to_string_pretty(&wallet_file)
                .map_err(|e| IroncladError::Wallet(format!("failed to serialize wallet: {e}")))?;
            tokio::fs::write(wallet_path, contents)
                .await
                .map_err(|e| IroncladError::Wallet(format!("failed to write wallet file: {e}")))?;

            Ok(Self {
                address,
                private_key: key_bytes,
                private_key_path: wallet_path.clone(),
                chain_id: config.chain_id,
                rpc_url: config.rpc_url.clone(),
            })
        }
    }

    pub fn address(&self) -> &str {
        &self.address
    }

    pub fn chain_id(&self) -> u64 {
        self.chain_id
    }

    pub fn private_key_path(&self) -> &std::path::Path {
        &self.private_key_path
    }

    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    pub async fn sign_message(&self, message: &str) -> Result<String> {
        let signing_key = SigningKey::from_slice(&self.private_key).map_err(|e| {
            IroncladError::Wallet(format!("failed to load signing key: {e}"))
        })?;

        let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
        let mut data = Vec::with_capacity(prefix.len() + message.len());
        data.extend_from_slice(prefix.as_bytes());
        data.extend_from_slice(message.as_bytes());
        let digest = Keccak256::digest(&data);

        let (sig, _recid): (k256::ecdsa::Signature, k256::ecdsa::RecoveryId) = signing_key
            .sign_prehash_recoverable(&digest)
            .map_err(|e| IroncladError::Wallet(format!("signing failed: {e}")))?;
        Ok(format!("0x{}", hex::encode(sig.to_bytes())))
    }

    /// Query the USDC ERC-20 balance via eth_call to the configured RPC endpoint.
    /// Returns the balance in USDC (6 decimals, converted to f64).
    pub async fn get_usdc_balance(&self) -> Result<f64> {
        const USDC_BASE: &str = "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913";
        const BALANCE_OF_SELECTOR: &str = "70a08231";

        let padded_addr = format!("{:0>64}", &self.address[2..]);
        let call_data = format!("0x{BALANCE_OF_SELECTOR}{padded_addr}");

        let rpc_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{
                "to": USDC_BASE,
                "data": call_data,
            }, "latest"],
            "id": 1,
        });

        let client = reqwest::Client::new();
        let resp = client
            .post(&self.rpc_url)
            .json(&rpc_body)
            .send()
            .await
            .map_err(|e| IroncladError::Wallet(format!("RPC request failed: {e}")))?;

        let body: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| IroncladError::Wallet(format!("RPC response parse failed: {e}")))?;

        if let Some(err) = body.get("error") {
            return Err(IroncladError::Wallet(format!("RPC error: {err}")));
        }

        let result_hex = body["result"]
            .as_str()
            .ok_or_else(|| IroncladError::Wallet("missing result in RPC response".into()))?;

        let hex_str = result_hex.trim_start_matches("0x");
        let raw_balance = u128::from_str_radix(hex_str, 16)
            .map_err(|e| IroncladError::Wallet(format!("failed to parse balance hex: {e}")))?;

        Ok(raw_balance as f64 / 1_000_000.0)
    }

    pub fn test_mock() -> Self {
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();
        let address = eth_address_from_public_key(&signing_key);
        Self {
            address,
            private_key: key_bytes,
            private_key_path: PathBuf::from("/dev/null"),
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(dir: &TempDir) -> WalletConfig {
        WalletConfig {
            path: dir.path().join("wallet.json"),
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".into(),
        }
    }

    #[tokio::test]
    async fn load_or_generate_creates_file() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        assert!(!config.path.exists());
        let wallet = Wallet::load_or_generate(&config).await.unwrap();
        assert!(config.path.exists());
        assert!(wallet.address().starts_with("0x"));
        assert_eq!(wallet.address().len(), 42);
    }

    #[tokio::test]
    async fn load_or_generate_is_stable() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        let w1 = Wallet::load_or_generate(&config).await.unwrap();
        let w2 = Wallet::load_or_generate(&config).await.unwrap();
        assert_eq!(w1.address(), w2.address());
    }

    #[tokio::test]
    async fn address_and_chain_id() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let wallet = Wallet::load_or_generate(&config).await.unwrap();

        assert!(!wallet.address().is_empty());
        assert_eq!(wallet.address().len(), 42);
        assert_eq!(wallet.chain_id(), 8453);
    }

    #[tokio::test]
    async fn sign_message_produces_valid_ecdsa_signature() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let wallet = Wallet::load_or_generate(&config).await.unwrap();

        let sig1 = wallet.sign_message("hello").await.unwrap();
        let sig2 = wallet.sign_message("hello").await.unwrap();
        assert_eq!(sig1, sig2);
        assert!(sig1.starts_with("0x"));
        assert_eq!(sig1.len(), 2 + 128);

        let sig3 = wallet.sign_message("different").await.unwrap();
        assert_ne!(sig1, sig3);
    }

    #[tokio::test]
    async fn test_mock_creates_valid_wallet() {
        let wallet = Wallet::test_mock();
        assert!(wallet.address().starts_with("0x"));
        assert_eq!(wallet.address().len(), 42);
        assert_eq!(wallet.chain_id(), 8453);

        let sig = wallet.sign_message("test").await.unwrap();
        assert!(sig.starts_with("0x"));
    }
}
