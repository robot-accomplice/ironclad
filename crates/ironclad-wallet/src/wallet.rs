use std::path::PathBuf;

use aes_gcm::aead::Aead;
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use hkdf::Hkdf;
use ironclad_core::config::WalletConfig;
use ironclad_core::{IroncladError, Result};
use k256::ecdsa::SigningKey;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use sha3::{Digest, Keccak256};
use tracing::{debug, info, warn};
use zeroize::Zeroizing;

const ENCRYPTION_SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12;

fn derive_key(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    use argon2::Argon2;
    let params = argon2::Params::new(65536, 3, 1, Some(32)).expect("valid Argon2 params");
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .expect("Argon2id hash with valid params cannot fail");
    key
}

fn derive_key_legacy_hkdf(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), passphrase.as_bytes());
    let mut key = [0u8; 32];
    hkdf.expand(b"ironclad-wallet-encryption", &mut key)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail per RFC 5869");
    key
}

fn encrypt_wallet_data(data: &[u8], passphrase: &str) -> Vec<u8> {
    use rand::RngCore;
    let mut salt = [0u8; ENCRYPTION_SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt);
    let cipher = Aes256Gcm::new_from_slice(&key).expect("AES-256-GCM key is 32 bytes");
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, data)
        .expect("AES-GCM encryption failed");

    // Format: salt (16) || nonce (12) || ciphertext
    let mut result = Vec::with_capacity(ENCRYPTION_SALT_LEN + NONCE_LEN + ciphertext.len());
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    result
}

fn decrypt_wallet_data(encrypted: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if encrypted.len() < ENCRYPTION_SALT_LEN + NONCE_LEN + 16 {
        return Err(IroncladError::Wallet("encrypted data too short".into()));
    }
    let salt = &encrypted[..ENCRYPTION_SALT_LEN];
    let nonce_bytes = &encrypted[ENCRYPTION_SALT_LEN..ENCRYPTION_SALT_LEN + NONCE_LEN];
    let ciphertext = &encrypted[ENCRYPTION_SALT_LEN + NONCE_LEN..];

    // Try Argon2id-derived key first
    let key = derive_key(passphrase, salt);
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| IroncladError::Wallet(format!("cipher init failed: {e}")))?;
    let nonce = Nonce::from_slice(nonce_bytes);
    if let Ok(plaintext) = cipher.decrypt(nonce, ciphertext) {
        return Ok(plaintext);
    }

    // Fallback: try legacy HKDF-derived key
    let legacy_key = derive_key_legacy_hkdf(passphrase, salt);
    let legacy_cipher = Aes256Gcm::new_from_slice(&legacy_key)
        .map_err(|e| IroncladError::Wallet(format!("cipher init failed: {e}")))?;
    match legacy_cipher.decrypt(nonce, ciphertext) {
        Ok(plaintext) => {
            tracing::warn!(
                "wallet decrypted with legacy HKDF; re-encrypt with Argon2id by re-saving"
            );
            Ok(plaintext)
        }
        Err(_) => Err(IroncladError::Wallet(
            "wallet decryption failed (wrong passphrase?)".into(),
        )),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct WalletFile {
    address: String,
    chain_id: u64,
    private_key_hex: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBalance {
    pub symbol: String,
    pub name: String,
    pub balance: f64,
    pub contract: Option<String>,
    pub decimals: u32,
    pub is_native: bool,
}

struct KnownToken {
    symbol: &'static str,
    name: &'static str,
    contract: &'static str,
    decimals: u8,
}

#[derive(Debug, Clone)]
pub struct Wallet {
    address: String,
    private_key: Zeroizing<Vec<u8>>,
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
            let raw = tokio::fs::read(wallet_path)
                .await
                .map_err(|e| IroncladError::Wallet(format!("failed to read wallet file: {e}")))?;

            let wallet_file = if let Ok(s) = String::from_utf8(raw.clone()) {
                serde_json::from_str::<WalletFile>(&s).ok()
            } else {
                None
            }
            .or_else(|| {
                std::env::var("IRONCLAD_WALLET_PASSPHRASE")
                    .ok()
                    .filter(|p| !p.is_empty())
                    .and_then(|passphrase| {
                        decrypt_wallet_data(&raw, &passphrase)
                            .ok()
                            .and_then(|decrypted| {
                                serde_json::from_slice::<WalletFile>(&decrypted).ok()
                            })
                    })
            })
            .or_else(|| {
                let machine_pass = Self::machine_passphrase();
                decrypt_wallet_data(&raw, &machine_pass)
                    .ok()
                    .and_then(|decrypted| serde_json::from_slice::<WalletFile>(&decrypted).ok())
            });

            if let Some(wallet_file) = wallet_file {
                let key_bytes = hex::decode(&wallet_file.private_key_hex)
                    .map_err(|e| IroncladError::Wallet(format!("invalid private key hex: {e}")))?;
                let signing_key = SigningKey::from_slice(&key_bytes)
                    .map_err(|e| IroncladError::Wallet(format!("invalid private key: {e}")))?;

                let derived_addr = eth_address_from_public_key(&signing_key);
                if derived_addr != wallet_file.address {
                    return Err(IroncladError::Wallet(
                        "wallet file address does not match derived address".into(),
                    ));
                }

                return Ok(Self {
                    address: wallet_file.address,
                    private_key: Zeroizing::new(key_bytes),
                    private_key_path: wallet_path.clone(),
                    chain_id: config.chain_id,
                    rpc_url: config.rpc_url.clone(),
                });
            }

            info!(
                ?wallet_path,
                "legacy wallet file detected, regenerating with real keypair"
            );
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
            let json_bytes = serde_json::to_string_pretty(&wallet_file)
                .map_err(|e| IroncladError::Wallet(format!("failed to serialize wallet: {e}")))?
                .into_bytes();

            let to_write: Vec<u8> = match std::env::var("IRONCLAD_WALLET_PASSPHRASE") {
                Ok(passphrase) if !passphrase.is_empty() => {
                    encrypt_wallet_data(&json_bytes, &passphrase)
                }
                _ => {
                    let machine_pass = Self::machine_passphrase();
                    warn!(
                        "IRONCLAD_WALLET_PASSPHRASE not set; encrypting with machine-derived key"
                    );
                    encrypt_wallet_data(&json_bytes, &machine_pass)
                }
            };
            tokio::fs::write(wallet_path, to_write)
                .await
                .map_err(|e| IroncladError::Wallet(format!("failed to write wallet file: {e}")))?;

            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o600);
                std::fs::set_permissions(wallet_path, perms).map_err(|e| {
                    IroncladError::Wallet(format!("failed to set wallet permissions: {e}"))
                })?;
            }

            Ok(Self {
                address,
                private_key: Zeroizing::new(key_bytes),
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
        let signing_key = SigningKey::from_slice(&self.private_key)
            .map_err(|e| IroncladError::Wallet(format!("failed to load signing key: {e}")))?;

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
        self.get_erc20_balance(Self::usdc_address_for_chain(self.chain_id), 6)
            .await
    }

    /// Query the native gas token balance (ETH on Ethereum/Base/etc).
    pub async fn get_native_balance(&self) -> Result<f64> {
        let rpc_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_getBalance",
            "params": [&self.address, "latest"],
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

        let hex = body["result"]
            .as_str()
            .ok_or_else(|| IroncladError::Wallet("missing result in RPC response".into()))?
            .trim_start_matches("0x");

        let raw = u128::from_str_radix(hex, 16)
            .map_err(|e| IroncladError::Wallet(format!("failed to parse balance hex: {e}")))?;

        // 18 decimals for native ETH
        Ok(raw as f64 / 1e18)
    }

    /// Generic ERC-20 balance query. `decimals` is the token's decimal count.
    pub async fn get_erc20_balance(&self, contract: &str, decimals: u8) -> Result<f64> {
        const BALANCE_OF_SELECTOR: &str = "70a08231";

        let padded_addr = format!("{:0>64}", &self.address[2..]);
        let call_data = format!("0x{BALANCE_OF_SELECTOR}{padded_addr}");

        let rpc_body = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [{
                "to": contract,
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

        let hex = body["result"]
            .as_str()
            .ok_or_else(|| IroncladError::Wallet("missing result in RPC response".into()))?
            .trim_start_matches("0x");

        let raw = u128::from_str_radix(hex, 16)
            .map_err(|e| IroncladError::Wallet(format!("failed to parse balance hex: {e}")))?;

        let divisor = 10f64.powi(decimals as i32);
        Ok(raw as f64 / divisor)
    }

    /// Fetch all relevant token balances for the current chain.
    /// Returns (symbol, balance, contract_or_"native") tuples.
    pub async fn get_all_balances(&self) -> Vec<TokenBalance> {
        let mut balances = Vec::new();

        let native = self.native_symbol();
        match self.get_native_balance().await {
            Ok(b) => balances.push(TokenBalance {
                symbol: native.to_string(),
                name: self.native_name().to_string(),
                balance: b,
                contract: None,
                decimals: 18,
                is_native: true,
            }),
            Err(e) => tracing::warn!(error = %e, "failed to fetch native balance"),
        }

        for token in self.known_tokens() {
            match self.get_erc20_balance(token.contract, token.decimals).await {
                Ok(b) => balances.push(TokenBalance {
                    symbol: token.symbol.to_string(),
                    name: token.name.to_string(),
                    balance: b,
                    contract: Some(token.contract.to_string()),
                    decimals: token.decimals as u32,
                    is_native: false,
                }),
                Err(e) => tracing::warn!(
                    error = %e, token = token.symbol,
                    "failed to fetch token balance"
                ),
            }
        }

        balances
    }

    pub fn network_name(&self) -> &'static str {
        match self.chain_id {
            1 => "Ethereum Mainnet",
            10 => "Optimism",
            137 => "Polygon",
            8453 => "Base",
            42161 => "Arbitrum One",
            84532 => "Base Sepolia",
            11155111 => "Sepolia",
            _ => "Unknown Network",
        }
    }

    fn native_symbol(&self) -> &'static str {
        match self.chain_id {
            137 => "MATIC",
            _ => "ETH",
        }
    }

    fn native_name(&self) -> &'static str {
        match self.chain_id {
            137 => "Polygon MATIC",
            _ => "Ether",
        }
    }

    fn usdc_address_for_chain(chain_id: u64) -> &'static str {
        match chain_id {
            1 => "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48", // Ethereum
            10 => "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85", // Optimism
            137 => "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359", // Polygon
            8453 => "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", // Base
            42161 => "0xaf88d065e77c8cC2239327C5EDb3A432268e5831", // Arbitrum
            84532 => "0x036CbD53842c5426634e7929541eC2318f3dCF7e", // Base Sepolia
            _ => "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", // fallback: Base
        }
    }

    fn known_tokens(&self) -> Vec<KnownToken> {
        match self.chain_id {
            8453 => vec![
                KnownToken {
                    symbol: "USDC",
                    name: "USD Coin",
                    contract: "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "USDT",
                    name: "Tether USD",
                    contract: "0xfde4C96c8593536E31F229EA8f37b2ADa2699bb2",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "DAI",
                    name: "Dai Stablecoin",
                    contract: "0x50c5725949A6F0c72E6C4a641F24049A917DB0Cb",
                    decimals: 18,
                },
                KnownToken {
                    symbol: "WETH",
                    name: "Wrapped Ether",
                    contract: "0x4200000000000000000000000000000000000006",
                    decimals: 18,
                },
                KnownToken {
                    symbol: "cbBTC",
                    name: "Coinbase Wrapped BTC",
                    contract: "0xcbB7C0000aB88B473b1f5aFd9ef808440eed33Bf",
                    decimals: 8,
                },
            ],
            1 => vec![
                KnownToken {
                    symbol: "USDC",
                    name: "USD Coin",
                    contract: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "USDT",
                    name: "Tether USD",
                    contract: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "DAI",
                    name: "Dai Stablecoin",
                    contract: "0x6B175474E89094C44Da98b954EedeAC495271d0F",
                    decimals: 18,
                },
                KnownToken {
                    symbol: "WETH",
                    name: "Wrapped Ether",
                    contract: "0xC02aaA39b223FE8D0A0e5C4F27eAD9083C756Cc2",
                    decimals: 18,
                },
                KnownToken {
                    symbol: "WBTC",
                    name: "Wrapped Bitcoin",
                    contract: "0x2260FAC5E5542a773Aa44fBCfeDf7C193bc2C599",
                    decimals: 8,
                },
            ],
            42161 => vec![
                KnownToken {
                    symbol: "USDC",
                    name: "USD Coin",
                    contract: "0xaf88d065e77c8cC2239327C5EDb3A432268e5831",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "USDT",
                    name: "Tether USD",
                    contract: "0xFd086bC7CD5C481DCC9C85ebE478A1C0b69FCbb9",
                    decimals: 6,
                },
                KnownToken {
                    symbol: "WETH",
                    name: "Wrapped Ether",
                    contract: "0x82aF49447D8a07e3bd95BD0d56f35241523fBab1",
                    decimals: 18,
                },
            ],
            _ => vec![KnownToken {
                symbol: "USDC",
                name: "USD Coin",
                contract: Self::usdc_address_for_chain(self.chain_id),
                decimals: 6,
            }],
        }
    }

    /// Derives a deterministic machine-local passphrase from hostname and username.
    /// This is a fallback when IRONCLAD_WALLET_PASSPHRASE is not configured.
    /// Not as secure as a user-supplied passphrase, but prevents plaintext key storage.
    fn machine_passphrase() -> String {
        use sha3::Digest as _;
        let hostname = hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown-host".to_string());
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_else(|_| "unknown-user".to_string());
        let input = format!("ironclad-wallet-machine-key::{hostname}::{user}");
        let hash = Keccak256::digest(input.as_bytes());
        hex::encode(hash)
    }

    pub fn test_mock() -> Self {
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();
        let address = eth_address_from_public_key(&signing_key);
        Self {
            address,
            private_key: Zeroizing::new(key_bytes),
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

    #[cfg(unix)]
    #[tokio::test]
    async fn load_or_generate_sets_restrictive_permissions_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let _wallet = Wallet::load_or_generate(&config).await.unwrap();
        let meta = std::fs::metadata(&config.path).unwrap();
        let mode = meta.permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "wallet file should be 0o600 (owner read/write only)"
        );
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

    #[tokio::test]
    async fn wallet_encryption_roundtrip() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);

        unsafe {
            std::env::set_var("IRONCLAD_WALLET_PASSPHRASE", "test-passphrase");
        }
        let wallet1 = Wallet::load_or_generate(&config).await.unwrap();
        let addr1 = wallet1.address().to_string();

        let raw = tokio::fs::read(&config.path).await.unwrap();
        assert!(
            serde_json::from_slice::<serde_json::Value>(&raw).is_err(),
            "wallet file should not be plaintext JSON when passphrase is set"
        );

        let wallet2 = Wallet::load_or_generate(&config).await.unwrap();
        assert_eq!(wallet2.address(), addr1);

        unsafe {
            std::env::remove_var("IRONCLAD_WALLET_PASSPHRASE");
        }
    }

    #[test]
    fn decrypt_wallet_data_too_short() {
        let result = decrypt_wallet_data(&[0u8; 10], "password");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("too short"));
    }

    #[test]
    fn decrypt_wallet_data_wrong_passphrase() {
        let data = b"some wallet data to encrypt";
        let encrypted = encrypt_wallet_data(data, "correct-password");
        let result = decrypt_wallet_data(&encrypted, "wrong-password");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong passphrase"));
    }

    #[test]
    fn encrypt_decrypt_roundtrip_unit() {
        let original = b"hello wallet world";
        let encrypted = encrypt_wallet_data(original, "my-pass");
        let decrypted = decrypt_wallet_data(&encrypted, "my-pass").unwrap();
        assert_eq!(&decrypted, original);
    }

    #[tokio::test]
    async fn private_key_path_returns_wallet_path() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let wallet = Wallet::load_or_generate(&config).await.unwrap();
        assert_eq!(wallet.private_key_path(), config.path);
    }

    #[tokio::test]
    async fn rpc_url_returns_configured_url() {
        let dir = TempDir::new().unwrap();
        let config = test_config(&dir);
        let wallet = Wallet::load_or_generate(&config).await.unwrap();
        assert_eq!(wallet.rpc_url(), "https://mainnet.base.org");
    }
}
