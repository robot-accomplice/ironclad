#[derive(Deserialize, Default)]
pub struct RevenueTaxListParams {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct RevenueTaxConfirmRequest {
    pub tx_hash: String,
}

#[derive(Deserialize)]
pub struct RevenueTaxSubmitRequest {
    pub calldata: String,
    #[serde(default)]
    pub contract_address: Option<String>,
    #[serde(default)]
    pub value_wei: Option<String>,
    #[serde(default)]
    pub gas_limit: Option<u64>,
    #[serde(default)]
    pub max_fee_per_gas_wei: Option<String>,
    #[serde(default)]
    pub max_priority_fee_per_gas_wei: Option<String>,
}

#[derive(Deserialize)]
pub struct RevenueTaxFailRequest {
    pub reason: String,
}

pub async fn list_revenue_tax_tasks(
    State(state): State<AppState>,
    Query(query): Query<RevenueTaxListParams>,
) -> Result<impl IntoResponse, JsonError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = ironclad_db::revenue_tax_tasks::list_revenue_tax_tasks(&state.db, limit)
        .map_err(|e| internal_err(&e))?;
    Ok(axum::Json(json!({"tax_tasks": rows, "count": rows.len()})))
}

pub async fn start_revenue_tax_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let updated = ironclad_db::revenue_tax_tasks::mark_revenue_tax_in_progress(&state.db, &opportunity_id)
        .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request("revenue tax payout task must exist and be pending before start"));
    }
    Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": "in_progress"})))
}

pub async fn submit_revenue_tax_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueTaxSubmitRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let task = ironclad_db::revenue_tax_tasks::get_revenue_tax_task(&state.db, &opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("revenue tax task for opportunity '{}' not found", opportunity_id)))?;
    if !task.status.eq_ignore_ascii_case("in_progress") {
        return Err(bad_request(format!(
            "revenue tax task must be in_progress before submission (current: {})",
            task.status
        )));
    }
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue tax task source is missing or invalid JSON"))?;
    let source_obj = source
        .as_object()
        .ok_or_else(|| bad_request("revenue tax task source must be a JSON object"))?;
    let target_chain = source_obj
        .get("target_chain")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task is missing target_chain"))?;
    let wallet_chain = wallet_chain_label(state.wallet.wallet.chain_id())
        .ok_or_else(|| bad_request(format!(
            "wallet chain_id {} is not a supported chain for tax payout submissions",
            state.wallet.wallet.chain_id()
        )))?;
    if !target_chain.eq_ignore_ascii_case(wallet_chain) {
        return Err(bad_request(format!(
            "wallet chain '{}' cannot submit tax payout for target_chain '{}'",
            wallet_chain, target_chain
        )));
    }
    let currency = source_obj
        .get("currency")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task is missing currency"))?;
    let amount = source_obj
        .get("amount")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| bad_request("revenue tax task is missing amount"))?;
    let destination_wallet = source_obj
        .get("destination_wallet")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task is missing destination_wallet"))?;
    let contract_address = req
        .contract_address
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            source_obj.get("contract_address").and_then(|v| v.as_str()).map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
        })
        .ok_or_else(|| bad_request("tax payout submission requires contract_address"))?;

    let current_balance = current_source_balance(&state.wallet.wallet, currency)
        .await
        .map_err(|e| bad_request(e.to_string()))?;
    let hourly_total = ironclad_db::metrics::sum_transaction_amounts(&state.db, 1)
        .map_err(|e| internal_err(&e))?;
    let daily_total = ironclad_db::metrics::sum_transaction_amounts(&state.db, 24)
        .map_err(|e| internal_err(&e))?;
    state
        .wallet
        .treasury
        .check_all(amount, current_balance, hourly_total, daily_total)
        .map_err(|e| bad_request(e.to_string()))?;

    let tx_hash = ironclad_wallet::submit_evm_contract_call(
        &state.wallet.wallet,
        &ironclad_wallet::EvmContractCall {
            to: contract_address.clone(),
            data_hex: req.calldata.clone(),
            value_wei: req.value_wei.clone(),
            gas_limit: req.gas_limit,
            max_fee_per_gas_wei: req.max_fee_per_gas_wei.clone(),
            max_priority_fee_per_gas_wei: req.max_priority_fee_per_gas_wei.clone(),
        },
    )
    .await
    .map_err(|e| bad_request(e.to_string()))?;

    tracing::info!(
        opportunity_id = %opportunity_id,
        tx_hash = %tx_hash,
        "tax payout EVM transaction submitted; persisting tx_hash"
    );
    let updated = ironclad_db::revenue_tax_tasks::mark_revenue_tax_submitted(&state.db, &opportunity_id, &tx_hash)
        .map_err(|e| {
            tracing::error!(
                opportunity_id = %opportunity_id,
                tx_hash = %tx_hash,
                error = %e,
                "CRITICAL: EVM tax payout tx was submitted on-chain but DB write failed; tx_hash may be lost"
            );
            internal_err(&e)
        })?;
    if !updated {
        // EVM transaction already submitted on-chain — we MUST return the tx_hash
        // even though the DB status guard rejected the write (concurrent status change).
        tracing::error!(
            opportunity_id = %opportunity_id,
            tx_hash = %tx_hash,
            "CRITICAL: tax payout EVM tx submitted but mark_submitted returned false; \
             concurrent status change likely caused tx_hash persistence failure"
        );
        return Ok(axum::Json(json!({
            "opportunity_id": opportunity_id,
            "status": "submitted_but_untracked",
            "tx_hash": tx_hash,
            "warning": "EVM transaction was submitted but the task status could not be updated; \
                        use reconcile endpoint to recover",
        })));
    }

    ironclad_db::metrics::record_transaction_with_metadata(
        &state.db,
        "revenue_tax_submission",
        amount,
        currency,
        Some(destination_wallet),
        Some(tx_hash.as_str()),
        Some(&json!({"opportunity_id": opportunity_id, "target_chain": target_chain, "status": "submitted"}).to_string()),
    )
    .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "in_progress",
        "tx_hash": tx_hash,
        "target_chain": target_chain,
        "wallet_chain": wallet_chain,
    })))
}

pub async fn confirm_revenue_tax_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueTaxConfirmRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let tx_hash = req.tx_hash.trim();
    if tx_hash.is_empty() || tx_hash.len() > 128 {
        return Err(bad_request("tx_hash must be non-empty and <= 128 chars"));
    }
    mark_tax_confirmed_with_metrics(&state.db, &opportunity_id, tx_hash)?;
    Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": "completed", "tx_hash": tx_hash})))
}

pub async fn reconcile_revenue_tax_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let task = ironclad_db::revenue_tax_tasks::get_revenue_tax_task(&state.db, &opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| not_found(format!("revenue tax task for opportunity '{}' not found", opportunity_id)))?;
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue tax task source is missing or invalid JSON"))?;
    let tx_hash = source
        .get("tax_tx_hash")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task does not have a submitted tx_hash"))?;
    match ironclad_wallet::get_evm_transaction_receipt_status(&state.wallet.wallet, tx_hash)
        .await
        .map_err(|e| bad_request(e.to_string()))? {
        None => Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": task.status, "tx_hash": tx_hash, "reconciled": false, "receipt_status": "pending"}))),
        Some(true) => {
            mark_tax_confirmed_with_metrics(&state.db, &opportunity_id, tx_hash)?;
            Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": "completed", "tx_hash": tx_hash, "reconciled": true, "receipt_status": "confirmed"})))
        }
        Some(false) => {
            let updated = ironclad_db::revenue_tax_tasks::mark_revenue_tax_failed(&state.db, &opportunity_id, "on-chain receipt status=failed")
                .map_err(|e| internal_err(&e))?;
            if !updated {
                return Err(not_found(format!("revenue tax task for opportunity '{}' not found", opportunity_id)));
            }
            Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": "failed", "tx_hash": tx_hash, "reconciled": true, "receipt_status": "failed"})))
        }
    }
}

pub async fn fail_revenue_tax_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueTaxFailRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(bad_request("reason must be non-empty"));
    }
    let updated = ironclad_db::revenue_tax_tasks::mark_revenue_tax_failed(&state.db, &opportunity_id, reason)
        .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!("revenue tax task for opportunity '{}' not found", opportunity_id)));
    }
    Ok(axum::Json(json!({"opportunity_id": opportunity_id, "status": "failed", "reason": reason})))
}

fn mark_tax_confirmed_with_metrics(
    db: &ironclad_db::Database,
    opportunity_id: &str,
    tx_hash: &str,
) -> Result<(), JsonError> {
    let task = ironclad_db::revenue_tax_tasks::get_revenue_tax_task(db, opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| {
            not_found(format!(
                "revenue tax task for opportunity '{}' not found",
                opportunity_id
            ))
        })?;
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue tax task source is missing or invalid JSON"))?;
    let amount = source
        .get("amount")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| bad_request("revenue tax task is missing amount"))?;
    let currency = source
        .get("currency")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task is missing currency"))?;
    let destination_wallet = source
        .get("destination_wallet")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue tax task is missing destination_wallet"))?;
    let updated = ironclad_db::revenue_tax_tasks::mark_revenue_tax_confirmed(db, opportunity_id, tx_hash)
        .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!("revenue tax task for opportunity '{}' not found", opportunity_id)));
    }
    ironclad_db::metrics::record_transaction_with_metadata(
        db,
        "revenue_tax_execution",
        amount,
        currency,
        Some(destination_wallet),
        Some(tx_hash),
        Some(&serde_json::to_string(&json!({"opportunity_id": opportunity_id, "status": "completed"}))
            .map_err(|e| internal_err(&ironclad_core::IroncladError::Database(e.to_string())))?),
    )
    .map_err(|e| internal_err(&e))?;
    Ok(())
}
