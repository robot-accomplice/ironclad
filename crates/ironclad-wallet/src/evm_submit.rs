use alloy::network::{Ethereum, EthereumWallet, Network, TransactionBuilder};
use alloy::primitives::{Address, Bytes, U256};
use alloy::providers::{Provider, ProviderBuilder};
use alloy::signers::local::PrivateKeySigner;
use ironclad_core::{IroncladError, Result};

use crate::wallet::Wallet;

#[derive(Debug, Clone)]
pub struct EvmContractCall {
    pub to: String,
    pub data_hex: String,
    pub value_wei: Option<String>,
    pub gas_limit: Option<u64>,
    pub max_fee_per_gas_wei: Option<String>,
    pub max_priority_fee_per_gas_wei: Option<String>,
}

pub async fn submit_evm_contract_call(wallet: &Wallet, call: &EvmContractCall) -> Result<String> {
    let to = parse_address(&call.to)?;
    let input = parse_calldata(&call.data_hex)?;
    let value = parse_optional_u256(call.value_wei.as_deref())?;
    let max_fee = parse_optional_u128(call.max_fee_per_gas_wei.as_deref())?;
    let max_priority_fee = parse_optional_u128(call.max_priority_fee_per_gas_wei.as_deref())?;

    let key_bytes: &[u8; 32] = wallet
        .private_key_bytes()
        .try_into()
        .map_err(|_| IroncladError::Wallet("invalid private key length".into()))?;
    let signer = PrivateKeySigner::from_bytes(key_bytes.into())
        .map_err(|e| IroncladError::Wallet(format!("invalid private key: {e}")))?;
    let provider = ProviderBuilder::new()
        .wallet(EthereumWallet::from(signer))
        .on_http(
            wallet
                .rpc_url()
                .parse()
                .map_err(|e| IroncladError::Wallet(format!("invalid RPC URL: {e}")))?,
        );

    let mut tx = <Ethereum as Network>::TransactionRequest::default()
        .with_to(to)
        .with_input(input)
        .with_chain_id(wallet.chain_id());
    if let Some(value) = value {
        tx = tx.with_value(value);
    }
    if let Some(gas_limit) = call.gas_limit {
        tx = tx.with_gas_limit(gas_limit);
    }
    if let Some(max_fee) = max_fee {
        tx = tx.with_max_fee_per_gas(max_fee);
    }
    if let Some(max_priority_fee) = max_priority_fee {
        tx = tx.with_max_priority_fee_per_gas(max_priority_fee);
    }

    let pending = provider
        .send_transaction(tx)
        .await
        .map_err(|e| IroncladError::Wallet(format!("contract call submission failed: {e}")))?;
    Ok(format!("{:#x}", pending.tx_hash()))
}

pub async fn get_evm_transaction_receipt_status(
    wallet: &Wallet,
    tx_hash: &str,
) -> Result<Option<bool>> {
    let tx_hash = tx_hash.trim();
    if tx_hash.is_empty() {
        return Err(IroncladError::Wallet("tx_hash must be non-empty".into()));
    }
    let rpc_body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "eth_getTransactionReceipt",
        "params": [tx_hash],
        "id": 1,
    });
    let resp = reqwest::Client::new()
        .post(wallet.rpc_url())
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
    parse_receipt_status_response(&body)
}

fn parse_address(s: &str) -> Result<Address> {
    s.parse::<Address>()
        .map_err(|e| IroncladError::Wallet(format!("invalid destination address: {e}")))
}

fn parse_calldata(s: &str) -> Result<Bytes> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(IroncladError::Wallet("calldata must be non-empty".into()));
    }
    let hex_body = trimmed.strip_prefix("0x").unwrap_or(trimmed);
    let bytes = hex::decode(hex_body)
        .map_err(|e| IroncladError::Wallet(format!("invalid calldata hex: {e}")))?;
    Ok(Bytes::from(bytes))
}

fn parse_optional_u256(value: Option<&str>) -> Result<Option<U256>> {
    let Some(value) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    parse_u256(value).map(Some)
}

fn parse_optional_u128(value: Option<&str>) -> Result<Option<u128>> {
    let Some(value) = value.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    parse_u128(value).map(Some)
}

fn parse_u256(value: &str) -> Result<U256> {
    if let Some(hex_value) = value.strip_prefix("0x") {
        U256::from_str_radix(hex_value, 16)
            .map_err(|e| IroncladError::Wallet(format!("invalid hex quantity '{value}': {e}")))
    } else {
        U256::from_str_radix(value, 10)
            .map_err(|e| IroncladError::Wallet(format!("invalid decimal quantity '{value}': {e}")))
    }
}

fn parse_u128(value: &str) -> Result<u128> {
    if let Some(hex_value) = value.strip_prefix("0x") {
        u128::from_str_radix(hex_value, 16)
            .map_err(|e| IroncladError::Wallet(format!("invalid hex quantity '{value}': {e}")))
    } else {
        value
            .parse::<u128>()
            .map_err(|e| IroncladError::Wallet(format!("invalid decimal quantity '{value}': {e}")))
    }
}

fn parse_receipt_status_response(body: &serde_json::Value) -> Result<Option<bool>> {
    let Some(result) = body.get("result") else {
        return Err(IroncladError::Wallet(
            "missing result in transaction receipt response".into(),
        ));
    };
    if result.is_null() {
        return Ok(None);
    }
    let status = result
        .get("status")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IroncladError::Wallet("transaction receipt missing status field".into()))?;
    match status {
        "0x1" | "0x01" | "1" => Ok(Some(true)),
        "0x0" | "0x00" | "0" => Ok(Some(false)),
        other => Err(IroncladError::Wallet(format!(
            "unexpected transaction receipt status '{other}'"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_calldata_rejects_empty_string() {
        let err = parse_calldata("").unwrap_err();
        assert!(err.to_string().contains("calldata must be non-empty"));
    }

    #[test]
    fn parse_u256_accepts_hex_and_decimal() {
        assert_eq!(parse_u256("16").unwrap(), U256::from(16));
        assert_eq!(parse_u256("0x10").unwrap(), U256::from(16));
    }

    #[test]
    fn parse_u128_accepts_hex_and_decimal() {
        assert_eq!(parse_u128("16").unwrap(), 16);
        assert_eq!(parse_u128("0x10").unwrap(), 16);
    }

    #[test]
    fn parse_receipt_status_response_handles_pending_success_and_failure() {
        assert_eq!(
            parse_receipt_status_response(&serde_json::json!({"result": null})).unwrap(),
            None
        );
        assert_eq!(
            parse_receipt_status_response(&serde_json::json!({"result": {"status":"0x1"}}))
                .unwrap(),
            Some(true)
        );
        assert_eq!(
            parse_receipt_status_response(&serde_json::json!({"result": {"status":"0x0"}}))
                .unwrap(),
            Some(false)
        );
    }
}
