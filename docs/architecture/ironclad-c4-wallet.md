# C4 Level 3: Component Diagram -- ironclad-wallet

*Ethereum wallet (key management, 0600 file perms), Treasury (TreasuryPolicy; **Money(i64 cents)** internally, f64 API surface), yield engine (Aave V3 on Base Sepolia), x402.*

---

## Component Diagram

```mermaid
flowchart TB
    subgraph IroncladWallet ["ironclad-wallet"]
        WALLET["wallet.rs<br/>Ethereum Wallet"]
        X402["x402.rs<br/>x402 Payment Protocol"]
        TREASURY["treasury.rs<br/>Treasury Policy"]
        YIELD["yield_engine.rs<br/>DeFi Yield Engine"]
    end

    subgraph MoneyDetail ["money.rs — Peer Module"]
        MONEY["Money(i64 cents)<br/>from_dollars(), dollars(), cents()<br/>Add/Sub impls"]
    end

    subgraph WalletDetail ["wallet.rs"]
        LOAD["load_or_generate():<br/>Load wallet from wallet.path<br/>or generate; set file perms 0o600"]
        SIGN_MSG["sign_message():<br/>EIP-191 personal sign"]
        SIGN_TX["sign_transaction():<br/>EIP-1559 transaction signing"]
        PUB_ADDR["public_address():<br/>Ethereum address derived<br/>from private key"]
        BALANCE["get_usdc_balance():<br/>ERC-20 balanceOf call<br/>via alloy-rs"]
    end

    subgraph X402Detail ["x402.rs"]
        BUILD_HEADER["build_payment_header():<br/>Construct X-Payment header<br/>for x402 protocol"]
        SIGN_AUTH["sign_transfer_with_authorization():<br/>EIP-3009 TransferWithAuthorization<br/>USDC permit + transfer in one sig"]
        X402_FLOW["x402 flow:<br/>1. POST to credits endpoint<br/>2. Receive HTTP 402 + requirements<br/>3. Sign authorization<br/>4. Retry with X-Payment header<br/>5. Credits added"]
    end

    subgraph TreasuryDetail ["treasury.rs"]
        POLICY_STRUCT["TreasuryPolicy from TreasuryConfig<br/>Uses Money(i64) internally;<br/>API takes f64 (dollars)"]
        CHECK_PAYMENT["check_per_payment(amount_f64)<br/>check_hourly_limit(), check_daily_limit()"]
        CHECK_HOURLY["check_hourly_limit():<br/>query transactions table<br/>(1h window aggregate)"]
        CHECK_DAILY["check_daily_limit():<br/>query transactions table<br/>(24h window aggregate)"]
        CHECK_RESERVE["check_minimum_reserve():<br/>ensure balance stays above<br/>minimum_reserve after tx"]
        CHECK_INFERENCE["check_inference_budget():<br/>query inference_costs table<br/>(24h window aggregate)"]
    end

    subgraph YieldDetail ["yield_engine.rs"]
        AAVE_V3["Aave V3 on Base Sepolia<br/>pool_address, usdc_address, a_token_address"]
        CALC_EXCESS["calculate_excess():<br/>balance - minimum_reserve - 10% buffer"]
        SHOULD_DEPOSIT["should_deposit(): excess > min_deposit"]
        SHOULD_WITHDRAW["should_withdraw(): balance < withdrawal_threshold"]
        AAVE_DEPOSIT["deposit(): supply() when chain_rpc_url set;<br/>else mock tx hash"]
        AAVE_WITHDRAW["withdraw(): withdraw() to restore reserve"]
        TRACK_EARNINGS["track_earnings():<br/>periodic aToken balance check,<br/>delta recorded as yield_earned<br/>transaction"]
    end

    WALLET --> X402
    TREASURY --> WALLET
    YIELD --> WALLET
```

## Financial Flow

```mermaid
sequenceDiagram
    participant HB as Heartbeat Task
    participant Yield as yield_engine.rs
    participant Treasury as treasury.rs
    participant Wallet as wallet.rs
    participant Base as Ethereum Base
    participant DB as ironclad-db

    HB->>Wallet: get_usdc_balance()
    Wallet->>Base: ERC-20 balanceOf
    Base-->>Wallet: balance
    
    alt balance > minimum_reserve + min_deposit
        HB->>Yield: should_deposit()
        Yield->>Treasury: check_minimum_reserve()
        Treasury-->>Yield: OK
        Yield->>Wallet: sign_transaction (Aave deposit)
        Wallet->>Base: submit transaction
        Base-->>Wallet: tx_hash
        Yield->>DB: INSERT transactions (yield_deposit)
    else balance < withdrawal_threshold
        HB->>Yield: should_withdraw()
        Yield->>Wallet: sign_transaction (Aave withdraw)
        Wallet->>Base: submit transaction
        Base-->>Wallet: tx_hash
        Yield->>DB: INSERT transactions (yield_withdraw)
    end

    Note over HB,DB: Periodic yield tracking
    HB->>Yield: track_earnings()
    Yield->>Base: check aToken balance
    Yield->>DB: INSERT transactions (yield_earned, delta)
```

## Dependencies

**External crates**: `alloy-rs` (Ethereum client, signers, contracts), `alloy-sol-types` (Solidity ABI encoding)

**Internal crates**: `ironclad-core`, `ironclad-db`

**Depended on by**: `ironclad-schedule`, `ironclad-server`
