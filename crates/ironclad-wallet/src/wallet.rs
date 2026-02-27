use std::fmt;
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

fn derive_key(passphrase: &str, salt: &[u8]) -> Result<[u8; 32]> {
    use argon2::Argon2;
    let params = argon2::Params::new(65536, 3, 1, Some(32))
        .map_err(|e| IroncladError::Wallet(format!("invalid Argon2 params: {e}")))?;
    let argon2 = Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, params);
    let mut key = [0u8; 32];
    argon2
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|e| IroncladError::Wallet(format!("Argon2id key derivation failed: {e}")))?;
    Ok(key)
}

fn derive_key_legacy_hkdf(passphrase: &str, salt: &[u8]) -> [u8; 32] {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), passphrase.as_bytes());
    let mut key = [0u8; 32];
    hkdf.expand(b"ironclad-wallet-encryption", &mut key)
        .expect("HKDF-SHA256 expand to 32 bytes cannot fail per RFC 5869");
    key
}

fn encrypt_wallet_data(data: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    use rand::RngCore;
    let mut salt = [0u8; ENCRYPTION_SALT_LEN];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; NONCE_LEN];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(passphrase, &salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key)
        .map_err(|e| IroncladError::Wallet(format!("AES-256-GCM key init failed: {e}")))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, data)
        .map_err(|e| IroncladError::Wallet(format!("AES-GCM encryption failed: {e}")))?;

    // Format: salt (16) || nonce (12) || ciphertext
    let mut result = Vec::with_capacity(ENCRYPTION_SALT_LEN + NONCE_LEN + ciphertext.len());
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

fn decrypt_wallet_data(encrypted: &[u8], passphrase: &str) -> Result<Vec<u8>> {
    if encrypted.len() < ENCRYPTION_SALT_LEN + NONCE_LEN + 16 {
        return Err(IroncladError::Wallet("encrypted data too short".into()));
    }
    let salt = &encrypted[..ENCRYPTION_SALT_LEN];
    let nonce_bytes = &encrypted[ENCRYPTION_SALT_LEN..ENCRYPTION_SALT_LEN + NONCE_LEN];
    let ciphertext = &encrypted[ENCRYPTION_SALT_LEN + NONCE_LEN..];

    // Try Argon2id-derived key first
    let key = derive_key(passphrase, salt)?;
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

#[derive(Clone, Serialize, Deserialize)]
struct WalletFile {
    address: String,
    chain_id: u64,
    private_key_hex: String,
}

impl fmt::Debug for WalletFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WalletFile")
            .field("address", &self.address)
            .field("chain_id", &self.chain_id)
            .field("private_key_hex", &"[REDACTED]")
            .finish()
    }
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
    http: reqwest::Client,
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
                serde_json::from_str::<WalletFile>(&s).ok().inspect(|_wf| {
                    warn!("SECURITY: wallet file loaded as plaintext JSON without encryption. Re-encrypt with a passphrase for production use.");
                })
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
                    http: reqwest::Client::new(),
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
                    encrypt_wallet_data(&json_bytes, &passphrase)?
                }
                _ => {
                    let machine_pass = Self::machine_passphrase();
                    warn!(
                        "IRONCLAD_WALLET_PASSPHRASE not set; encrypting with machine-derived key"
                    );
                    encrypt_wallet_data(&json_bytes, &machine_pass)?
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
                http: reqwest::Client::new(),
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

        let resp = self
            .http
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

        // NOTE: u128-to-f64 cast loses precision for balances above ~2^53 base units
        // (~9.007 ETH at 18 decimals). A future refactor should use the Money type for
        // lossless arithmetic. For typical agent balances this is acceptable.
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

        let resp = self
            .http
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

        // NOTE: u128-to-f64 cast loses precision for balances above ~2^53 base units.
        // A future refactor should use the Money type for lossless arithmetic.
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
            http: reqwest::Client::new(),
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
        let encrypted = encrypt_wallet_data(data, "correct-password").unwrap();
        let result = decrypt_wallet_data(&encrypted, "wrong-password");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("wrong passphrase"));
    }

    #[test]
    fn encrypt_decrypt_roundtrip_unit() {
        let original = b"hello wallet world";
        let encrypted = encrypt_wallet_data(original, "my-pass").unwrap();
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

    /// Helper: build a wallet with a custom chain_id for testing chain-specific logic.
    async fn wallet_with_chain(chain_id: u64) -> Wallet {
        let dir = TempDir::new().unwrap();
        let config = WalletConfig {
            path: dir.path().join("wallet.json"),
            chain_id,
            rpc_url: "https://rpc.example.com".into(),
        };
        // We need `dir` to live long enough, so leak a reference (test only).
        // dir dropped here but wallet already loaded key
        Wallet::load_or_generate(&config).await.unwrap()
    }

    // --- network_name coverage ---

    #[tokio::test]
    async fn network_name_ethereum_mainnet() {
        let w = wallet_with_chain(1).await;
        assert_eq!(w.network_name(), "Ethereum Mainnet");
    }

    #[tokio::test]
    async fn network_name_optimism() {
        let w = wallet_with_chain(10).await;
        assert_eq!(w.network_name(), "Optimism");
    }

    #[tokio::test]
    async fn network_name_polygon() {
        let w = wallet_with_chain(137).await;
        assert_eq!(w.network_name(), "Polygon");
    }

    #[tokio::test]
    async fn network_name_base() {
        let w = wallet_with_chain(8453).await;
        assert_eq!(w.network_name(), "Base");
    }

    #[tokio::test]
    async fn network_name_arbitrum() {
        let w = wallet_with_chain(42161).await;
        assert_eq!(w.network_name(), "Arbitrum One");
    }

    #[tokio::test]
    async fn network_name_base_sepolia() {
        let w = wallet_with_chain(84532).await;
        assert_eq!(w.network_name(), "Base Sepolia");
    }

    #[tokio::test]
    async fn network_name_sepolia() {
        let w = wallet_with_chain(11155111).await;
        assert_eq!(w.network_name(), "Sepolia");
    }

    #[tokio::test]
    async fn network_name_unknown() {
        let w = wallet_with_chain(99999).await;
        assert_eq!(w.network_name(), "Unknown Network");
    }

    // --- native_symbol / native_name coverage ---

    #[tokio::test]
    async fn native_symbol_polygon_is_matic() {
        let w = wallet_with_chain(137).await;
        assert_eq!(w.native_symbol(), "MATIC");
        assert_eq!(w.native_name(), "Polygon MATIC");
    }

    #[tokio::test]
    async fn native_symbol_default_is_eth() {
        let w = wallet_with_chain(1).await;
        assert_eq!(w.native_symbol(), "ETH");
        assert_eq!(w.native_name(), "Ether");
    }

    #[tokio::test]
    async fn native_symbol_base_is_eth() {
        let w = wallet_with_chain(8453).await;
        assert_eq!(w.native_symbol(), "ETH");
        assert_eq!(w.native_name(), "Ether");
    }

    // --- usdc_address_for_chain coverage ---

    #[test]
    fn usdc_address_ethereum() {
        assert_eq!(
            Wallet::usdc_address_for_chain(1),
            "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48"
        );
    }

    #[test]
    fn usdc_address_optimism() {
        assert_eq!(
            Wallet::usdc_address_for_chain(10),
            "0x0b2C639c533813f4Aa9D7837CAf62653d097Ff85"
        );
    }

    #[test]
    fn usdc_address_polygon() {
        assert_eq!(
            Wallet::usdc_address_for_chain(137),
            "0x3c499c542cEF5E3811e1192ce70d8cC03d5c3359"
        );
    }

    #[test]
    fn usdc_address_base() {
        assert_eq!(
            Wallet::usdc_address_for_chain(8453),
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        );
    }

    #[test]
    fn usdc_address_arbitrum() {
        assert_eq!(
            Wallet::usdc_address_for_chain(42161),
            "0xaf88d065e77c8cC2239327C5EDb3A432268e5831"
        );
    }

    #[test]
    fn usdc_address_base_sepolia() {
        assert_eq!(
            Wallet::usdc_address_for_chain(84532),
            "0x036CbD53842c5426634e7929541eC2318f3dCF7e"
        );
    }

    #[test]
    fn usdc_address_fallback() {
        // Unknown chain falls back to Base USDC address
        assert_eq!(
            Wallet::usdc_address_for_chain(99999),
            "0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913"
        );
    }

    // --- known_tokens coverage ---

    #[tokio::test]
    async fn known_tokens_base_has_five_tokens() {
        let w = wallet_with_chain(8453).await;
        let tokens = w.known_tokens();
        assert_eq!(tokens.len(), 5);
        let symbols: Vec<&str> = tokens.iter().map(|t| t.symbol).collect();
        assert!(symbols.contains(&"USDC"));
        assert!(symbols.contains(&"USDT"));
        assert!(symbols.contains(&"DAI"));
        assert!(symbols.contains(&"WETH"));
        assert!(symbols.contains(&"cbBTC"));
    }

    #[tokio::test]
    async fn known_tokens_ethereum_has_five_tokens() {
        let w = wallet_with_chain(1).await;
        let tokens = w.known_tokens();
        assert_eq!(tokens.len(), 5);
        let symbols: Vec<&str> = tokens.iter().map(|t| t.symbol).collect();
        assert!(symbols.contains(&"USDC"));
        assert!(symbols.contains(&"USDT"));
        assert!(symbols.contains(&"DAI"));
        assert!(symbols.contains(&"WETH"));
        assert!(symbols.contains(&"WBTC"));
    }

    #[tokio::test]
    async fn known_tokens_arbitrum_has_three_tokens() {
        let w = wallet_with_chain(42161).await;
        let tokens = w.known_tokens();
        assert_eq!(tokens.len(), 3);
        let symbols: Vec<&str> = tokens.iter().map(|t| t.symbol).collect();
        assert!(symbols.contains(&"USDC"));
        assert!(symbols.contains(&"USDT"));
        assert!(symbols.contains(&"WETH"));
    }

    #[tokio::test]
    async fn known_tokens_unknown_chain_fallback_usdc_only() {
        let w = wallet_with_chain(99999).await;
        let tokens = w.known_tokens();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].symbol, "USDC");
        assert_eq!(tokens[0].decimals, 6);
    }

    #[tokio::test]
    async fn known_tokens_base_decimals_correct() {
        let w = wallet_with_chain(8453).await;
        let tokens = w.known_tokens();
        for token in &tokens {
            match token.symbol {
                "USDC" | "USDT" => assert_eq!(token.decimals, 6),
                "DAI" | "WETH" => assert_eq!(token.decimals, 18),
                "cbBTC" => assert_eq!(token.decimals, 8),
                _ => panic!("unexpected token: {}", token.symbol),
            }
        }
    }

    #[tokio::test]
    async fn known_tokens_ethereum_decimals_correct() {
        let w = wallet_with_chain(1).await;
        let tokens = w.known_tokens();
        for token in &tokens {
            match token.symbol {
                "USDC" | "USDT" => assert_eq!(token.decimals, 6),
                "DAI" | "WETH" => assert_eq!(token.decimals, 18),
                "WBTC" => assert_eq!(token.decimals, 8),
                _ => panic!("unexpected token: {}", token.symbol),
            }
        }
    }

    // --- machine_passphrase coverage ---

    #[test]
    fn machine_passphrase_is_deterministic() {
        let p1 = Wallet::machine_passphrase();
        let p2 = Wallet::machine_passphrase();
        assert_eq!(p1, p2);
    }

    #[test]
    fn machine_passphrase_is_hex_string() {
        let p = Wallet::machine_passphrase();
        // Keccak256 produces 32 bytes = 64 hex chars
        assert_eq!(p.len(), 64);
        assert!(p.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // --- WalletFile debug redaction ---

    #[test]
    fn wallet_file_debug_redacts_private_key() {
        let wf = WalletFile {
            address: "0xtest".into(),
            chain_id: 1,
            private_key_hex: "deadbeef".into(),
        };
        let debug_str = format!("{:?}", wf);
        assert!(debug_str.contains("REDACTED"));
        assert!(!debug_str.contains("deadbeef"));
        assert!(debug_str.contains("0xtest"));
    }

    // --- eth_address_from_public_key ---

    #[test]
    fn eth_address_from_public_key_produces_valid_address() {
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let addr = eth_address_from_public_key(&signing_key);
        assert!(addr.starts_with("0x"));
        assert_eq!(addr.len(), 42);
        // All chars after 0x should be hex
        assert!(addr[2..].chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn eth_address_from_public_key_is_deterministic() {
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let addr1 = eth_address_from_public_key(&signing_key);
        let addr2 = eth_address_from_public_key(&signing_key);
        assert_eq!(addr1, addr2);
    }

    // --- derive_key_legacy_hkdf ---

    #[test]
    fn derive_key_legacy_hkdf_is_deterministic() {
        let salt = [0u8; 16];
        let key1 = derive_key_legacy_hkdf("passphrase", &salt);
        let key2 = derive_key_legacy_hkdf("passphrase", &salt);
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_key_legacy_hkdf_different_passphrase_different_key() {
        let salt = [0u8; 16];
        let key1 = derive_key_legacy_hkdf("passphrase1", &salt);
        let key2 = derive_key_legacy_hkdf("passphrase2", &salt);
        assert_ne!(key1, key2);
    }

    #[test]
    fn derive_key_legacy_hkdf_different_salt_different_key() {
        let salt1 = [0u8; 16];
        let salt2 = [1u8; 16];
        let key1 = derive_key_legacy_hkdf("passphrase", &salt1);
        let key2 = derive_key_legacy_hkdf("passphrase", &salt2);
        assert_ne!(key1, key2);
    }

    // --- derive_key (argon2id) ---

    #[test]
    fn derive_key_argon2id_is_deterministic() {
        let salt = [42u8; 16];
        let key1 = derive_key("pass", &salt).unwrap();
        let key2 = derive_key("pass", &salt).unwrap();
        assert_eq!(key1, key2);
    }

    #[test]
    fn derive_key_argon2id_different_passphrase() {
        let salt = [42u8; 16];
        let key1 = derive_key("pass1", &salt).unwrap();
        let key2 = derive_key("pass2", &salt).unwrap();
        assert_ne!(key1, key2);
    }

    // --- TokenBalance struct ---

    #[test]
    fn token_balance_serialization_roundtrip() {
        let tb = TokenBalance {
            symbol: "USDC".into(),
            name: "USD Coin".into(),
            balance: 100.5,
            contract: Some("0xabc".into()),
            decimals: 6,
            is_native: false,
        };
        let json = serde_json::to_string(&tb).unwrap();
        let deserialized: TokenBalance = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.symbol, "USDC");
        assert_eq!(deserialized.name, "USD Coin");
        assert!((deserialized.balance - 100.5).abs() < f64::EPSILON);
        assert_eq!(deserialized.contract, Some("0xabc".into()));
        assert_eq!(deserialized.decimals, 6);
        assert!(!deserialized.is_native);
    }

    #[test]
    fn token_balance_native_no_contract() {
        let tb = TokenBalance {
            symbol: "ETH".into(),
            name: "Ether".into(),
            balance: 1.5,
            contract: None,
            decimals: 18,
            is_native: true,
        };
        let json = serde_json::to_string(&tb).unwrap();
        let deserialized: TokenBalance = serde_json::from_str(&json).unwrap();
        assert!(deserialized.is_native);
        assert!(deserialized.contract.is_none());
    }

    #[test]
    fn token_balance_debug_format() {
        let tb = TokenBalance {
            symbol: "USDC".into(),
            name: "USD Coin".into(),
            balance: 42.0,
            contract: Some("0xabc".into()),
            decimals: 6,
            is_native: false,
        };
        let debug_str = format!("{:?}", tb);
        assert!(debug_str.contains("USDC"));
        assert!(debug_str.contains("USD Coin"));
    }

    // --- WalletFile serialization ---

    #[test]
    fn wallet_file_serde_roundtrip() {
        let wf = WalletFile {
            address: "0x1234".into(),
            chain_id: 8453,
            private_key_hex: "abcdef".into(),
        };
        let json = serde_json::to_string(&wf).unwrap();
        let wf2: WalletFile = serde_json::from_str(&json).unwrap();
        assert_eq!(wf2.address, "0x1234");
        assert_eq!(wf2.chain_id, 8453);
        assert_eq!(wf2.private_key_hex, "abcdef");
    }

    // --- load_or_generate with plaintext JSON file (legacy) ---

    #[tokio::test]
    async fn load_or_generate_reads_plaintext_json_wallet() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.json");

        // Generate a real keypair
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();
        let address = eth_address_from_public_key(&signing_key);

        let wallet_file = WalletFile {
            address: address.clone(),
            chain_id: 8453,
            private_key_hex: hex::encode(&key_bytes),
        };
        let json = serde_json::to_string_pretty(&wallet_file).unwrap();
        std::fs::write(&wallet_path, &json).unwrap();

        let config = WalletConfig {
            path: wallet_path,
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".into(),
        };

        let loaded = Wallet::load_or_generate(&config).await.unwrap();
        assert_eq!(loaded.address(), address);
    }

    // --- load_or_generate with address mismatch ---

    #[tokio::test]
    async fn load_or_generate_rejects_address_mismatch() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.json");

        // Write a wallet file with mismatched address
        let signing_key = SigningKey::random(&mut k256::elliptic_curve::rand_core::OsRng);
        let key_bytes = signing_key.to_bytes().to_vec();

        let wallet_file = WalletFile {
            address: "0x0000000000000000000000000000000000000bad".into(), // wrong address
            chain_id: 8453,
            private_key_hex: hex::encode(&key_bytes),
        };
        let json = serde_json::to_string_pretty(&wallet_file).unwrap();
        std::fs::write(&wallet_path, &json).unwrap();

        let config = WalletConfig {
            path: wallet_path,
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".into(),
        };

        let result = Wallet::load_or_generate(&config).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("does not match"));
    }

    // --- encrypted data format validation ---

    #[test]
    fn encrypt_wallet_data_produces_salt_nonce_ciphertext() {
        let data = b"test data";
        let encrypted = encrypt_wallet_data(data, "password").unwrap();
        // Should have: 16 bytes salt + 12 bytes nonce + ciphertext (at least 16 bytes GCM tag)
        assert!(encrypted.len() >= ENCRYPTION_SALT_LEN + NONCE_LEN + 16);
    }

    #[test]
    fn encrypt_wallet_data_different_each_time() {
        let data = b"test data";
        let e1 = encrypt_wallet_data(data, "password").unwrap();
        let e2 = encrypt_wallet_data(data, "password").unwrap();
        // Random salt/nonce means different ciphertexts
        assert_ne!(e1, e2);
    }

    // --- RPC method tests using a local mock HTTP server ---

    /// Start a simple mock JSON-RPC server that returns configurable responses.
    /// Returns the (address, join_handle).
    async fn start_mock_rpc_server(
        response_body: serde_json::Value,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let url = format!("http://127.0.0.1:{}", addr.port());

        let handle = tokio::spawn(async move {
            // Accept multiple connections to handle multiple RPC calls
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let resp = response_body.clone();
                tokio::spawn(async move {
                    let mut buf = vec![0u8; 4096];
                    let _ = stream.read(&mut buf).await;
                    let body = serde_json::to_string(&resp).unwrap();
                    let http_resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(http_resp.as_bytes()).await;
                    let _ = stream.shutdown().await;
                });
            }
        });

        (url, handle)
    }

    /// Helper: create a wallet with a given RPC URL.
    async fn wallet_with_rpc(rpc_url: &str) -> Wallet {
        let dir = TempDir::new().unwrap();
        let config = WalletConfig {
            path: dir.path().join("wallet.json"),
            chain_id: 8453,
            rpc_url: rpc_url.to_string(),
        };
        Wallet::load_or_generate(&config).await.unwrap()
    }

    #[tokio::test]
    async fn get_native_balance_parses_rpc_response() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0xDE0B6B3A7640000" // 1 ETH in wei (10^18)
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet.get_native_balance().await.unwrap();
        // 0xDE0B6B3A7640000 = 10^18 wei = 1.0 ETH
        assert!((balance - 1.0).abs() < 1e-10);
        handle.abort();
    }

    #[tokio::test]
    async fn get_native_balance_zero() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x0"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet.get_native_balance().await.unwrap();
        assert!((balance - 0.0).abs() < f64::EPSILON);
        handle.abort();
    }

    #[tokio::test]
    async fn get_native_balance_rpc_error() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32000, "message": "execution reverted"}
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet.get_native_balance().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RPC error"));
        handle.abort();
    }

    #[tokio::test]
    async fn get_native_balance_missing_result() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet.get_native_balance().await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing result"));
        handle.abort();
    }

    #[tokio::test]
    async fn get_native_balance_invalid_hex() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0xZZZZZZZZ"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet.get_native_balance().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to parse balance hex")
        );
        handle.abort();
    }

    #[tokio::test]
    async fn get_native_balance_connection_refused() {
        let wallet = wallet_with_rpc("http://127.0.0.1:1").await;
        let result = wallet.get_native_balance().await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RPC request failed")
        );
    }

    #[tokio::test]
    async fn get_erc20_balance_parses_rpc_response() {
        // balanceOf returns a uint256 in the result field
        // 1,000,000 raw = 1.0 USDC (6 decimals)
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x00000000000000000000000000000000000000000000000000000000000F4240"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await
            .unwrap();
        // 0xF4240 = 1,000,000 raw units / 10^6 = 1.0 USDC
        assert!((balance - 1.0).abs() < 1e-10);
        handle.abort();
    }

    #[tokio::test]
    async fn get_erc20_balance_zero() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x0000000000000000000000000000000000000000000000000000000000000000"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await
            .unwrap();
        assert!((balance - 0.0).abs() < f64::EPSILON);
        handle.abort();
    }

    #[tokio::test]
    async fn get_erc20_balance_rpc_error() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {"code": -32000, "message": "execution reverted"}
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("RPC error"));
        handle.abort();
    }

    #[tokio::test]
    async fn get_erc20_balance_missing_result() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("missing result"));
        handle.abort();
    }

    #[tokio::test]
    async fn get_erc20_balance_invalid_hex() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "not-valid-hex"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let result = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await;
        assert!(result.is_err());
        handle.abort();
    }

    #[tokio::test]
    async fn get_erc20_balance_connection_refused() {
        let wallet = wallet_with_rpc("http://127.0.0.1:1").await;
        let result = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 6)
            .await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("RPC request failed")
        );
    }

    #[tokio::test]
    async fn get_erc20_balance_18_decimals() {
        // 1 * 10^18 = 1.0 token with 18 decimals
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x0000000000000000000000000000000000000000000000000DE0B6B3A7640000"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet
            .get_erc20_balance("0x833589fCD6eDb6E08f4c7C32D4f71b54bdA02913", 18)
            .await
            .unwrap();
        assert!((balance - 1.0).abs() < 1e-10);
        handle.abort();
    }

    #[tokio::test]
    async fn get_usdc_balance_delegates_to_erc20() {
        // 2,000,000 raw = 2.0 USDC (6 decimals)
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x00000000000000000000000000000000000000000000000000000000001E8480"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balance = wallet.get_usdc_balance().await.unwrap();
        // 0x1E8480 = 2,000,000 raw / 10^6 = 2.0 USDC
        assert!((balance - 2.0).abs() < 1e-10);
        handle.abort();
    }

    #[tokio::test]
    async fn get_all_balances_returns_native_and_tokens() {
        // This mock returns a valid balance for all RPC calls.
        // get_all_balances calls get_native_balance() then get_erc20_balance() per known token.
        // For chain 8453, that's 1 native + 5 tokens = 6 calls.
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x0000000000000000000000000000000000000000000000000000000000000000"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balances = wallet.get_all_balances().await;
        // Should have at least 1 entry (native balance)
        assert!(!balances.is_empty());
        // First should be native
        assert!(balances[0].is_native);
        assert_eq!(balances[0].symbol, "ETH");
        assert_eq!(balances[0].decimals, 18);
        // Should also have token balances (chain 8453 has 5 tokens)
        assert_eq!(balances.len(), 6, "expected 1 native + 5 tokens for Base");
        handle.abort();
    }

    #[tokio::test]
    async fn get_all_balances_handles_native_failure_gracefully() {
        // If the RPC is unreachable, get_all_balances should still return an
        // empty vec (or partial results) without panicking.
        let wallet = wallet_with_rpc("http://127.0.0.1:1").await;
        let balances = wallet.get_all_balances().await;
        // With RPC unreachable, both native and token calls fail, so balances is empty
        assert!(balances.is_empty());
    }

    #[tokio::test]
    async fn get_all_balances_token_entries_have_contracts() {
        let rpc_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": "0x00000000000000000000000000000000000000000000000000000000000F4240"
        });
        let (url, handle) = start_mock_rpc_server(rpc_response).await;
        let wallet = wallet_with_rpc(&url).await;
        let balances = wallet.get_all_balances().await;
        for tb in &balances {
            if !tb.is_native {
                assert!(
                    tb.contract.is_some(),
                    "ERC-20 token {} should have contract address",
                    tb.symbol
                );
            } else {
                assert!(
                    tb.contract.is_none(),
                    "native token should not have contract"
                );
            }
        }
        handle.abort();
    }

    // --- wallet.rs load_or_generate: legacy/corrupt wallet file triggers regeneration ---

    #[tokio::test]
    async fn load_or_generate_regenerates_on_corrupt_file() {
        let dir = TempDir::new().unwrap();
        let wallet_path = dir.path().join("wallet.json");

        // Write random binary data that can't be parsed as JSON or decrypted
        let corrupt_data: Vec<u8> = (0..100).map(|i| (i * 37 + 13) as u8).collect();
        std::fs::write(&wallet_path, &corrupt_data).unwrap();

        let config = WalletConfig {
            path: wallet_path.clone(),
            chain_id: 8453,
            rpc_url: "https://mainnet.base.org".into(),
        };

        // Should regenerate a new wallet instead of failing
        let wallet = Wallet::load_or_generate(&config).await.unwrap();
        assert!(wallet.address().starts_with("0x"));
        assert_eq!(wallet.address().len(), 42);
    }

    // --- known_tokens contract addresses are valid hex ---

    #[tokio::test]
    async fn known_tokens_have_valid_contract_addresses() {
        for chain_id in [1u64, 8453, 42161, 99999] {
            let w = wallet_with_chain(chain_id).await;
            for token in w.known_tokens() {
                assert!(
                    token.contract.starts_with("0x"),
                    "token {} on chain {} has invalid contract prefix",
                    token.symbol,
                    chain_id
                );
                assert_eq!(
                    token.contract.len(),
                    42,
                    "token {} on chain {} has wrong address length",
                    token.symbol,
                    chain_id
                );
            }
        }
    }

    // --- legacy HKDF fallback decryption path ---

    #[test]
    fn decrypt_wallet_data_legacy_hkdf_fallback() {
        // Manually encrypt with legacy HKDF-derived key to exercise the fallback decryption path
        use aes_gcm::aead::Aead;
        use rand::RngCore;

        let data = b"legacy encrypted wallet data";
        let passphrase = "legacy-pass";

        let mut salt = [0u8; ENCRYPTION_SALT_LEN];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);

        // Encrypt with legacy HKDF key
        let legacy_key = derive_key_legacy_hkdf(passphrase, &salt);
        let cipher = Aes256Gcm::new_from_slice(&legacy_key).unwrap();
        let nonce = Nonce::from_slice(&nonce_bytes);
        let ciphertext = cipher.encrypt(nonce, data.as_ref()).unwrap();

        // Build the encrypted blob: salt || nonce || ciphertext
        let mut encrypted = Vec::with_capacity(ENCRYPTION_SALT_LEN + NONCE_LEN + ciphertext.len());
        encrypted.extend_from_slice(&salt);
        encrypted.extend_from_slice(&nonce_bytes);
        encrypted.extend_from_slice(&ciphertext);

        // decrypt_wallet_data should fall back to legacy HKDF and succeed
        let decrypted = decrypt_wallet_data(&encrypted, passphrase).unwrap();
        assert_eq!(&decrypted, data);
    }

    // --- known_tokens names are non-empty ---

    #[tokio::test]
    async fn known_tokens_have_nonempty_names() {
        for chain_id in [1u64, 8453, 42161, 137] {
            let w = wallet_with_chain(chain_id).await;
            for token in w.known_tokens() {
                assert!(
                    !token.name.is_empty(),
                    "token {} on chain {} has empty name",
                    token.symbol,
                    chain_id
                );
                assert!(
                    !token.symbol.is_empty(),
                    "chain {} has token with empty symbol",
                    chain_id
                );
            }
        }
    }
}
