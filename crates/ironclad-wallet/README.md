# ironclad-wallet

> **Version 0.5.0**

Ethereum wallet management for the [Ironclad](https://github.com/robot-accomplice/ironclad) agent runtime. Features HD wallet generation, x402 payment protocol (EIP-3009 `transferWithAuthorization`), treasury policy engine with survival-tier-aware spending limits, and DeFi yield optimization on Base (Aave/Compound).

## Key Types

| Type | Module | Description |
|------|--------|-------------|
| `WalletService` | `lib` | Top-level facade composing wallet, treasury, and yield engine |
| `Wallet` | `wallet` | HD wallet with key loading/generation and USDC balance tracking |
| `TreasuryPolicy` | `treasury` | Spending limits, per-payment caps, reserve thresholds |
| `YieldEngine` | `yield_engine` | DeFi yield optimization (Aave/Compound deposit/withdraw) |
| `X402Handler` | `x402` | x402 payment protocol handler (EIP-3009) |
| `Money` | `money` | USDC amount type with formatting and arithmetic |
| `TokenBalance` | `wallet` | On-chain token balance snapshot |

## Usage

```toml
[dependencies]
ironclad-wallet = "0.5"
```

```rust
use ironclad_wallet::WalletService;
use ironclad_core::IroncladConfig;

let config = IroncladConfig::from_file("ironclad.toml")?;
let service = WalletService::new(&config).await?;
println!("Wallet address: {}", service.wallet.address());
```

## Documentation

API docs are available on [docs.rs](https://docs.rs/ironclad-wallet).

## License

Licensed under Apache-2.0. See [LICENSE](https://github.com/robot-accomplice/ironclad/blob/main/LICENSE) for details.
