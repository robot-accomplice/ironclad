
#[derive(Deserialize, Default)]
pub struct RevenueSwapListParams {
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Deserialize)]
pub struct RevenueSwapConfirmRequest {
    pub tx_hash: String,
}

#[derive(Deserialize)]
pub struct RevenueSwapSubmitRequest {
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
pub struct RevenueSwapFailRequest {
    pub reason: String,
}

pub async fn list_revenue_swap_tasks(
    State(state): State<AppState>,
    Query(query): Query<RevenueSwapListParams>,
) -> Result<impl IntoResponse, JsonError> {
    let limit = query.limit.unwrap_or(50).clamp(1, 200);
    let rows = ironclad_db::revenue_swap_tasks::list_revenue_swap_tasks(&state.db, limit)
        .map_err(|e| internal_err(&e))?;
    Ok(axum::Json(json!({
        "swap_tasks": rows,
        "count": rows.len(),
    })))
}

pub async fn start_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let updated = ironclad_db::revenue_swap_tasks::mark_revenue_swap_in_progress(
        &state.db,
        &opportunity_id,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(bad_request(
            "revenue swap task must exist and be pending before start",
        ));
    }
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "in_progress",
    })))
}

pub async fn submit_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueSwapSubmitRequest>,
) -> Result<impl IntoResponse, JsonError> {
    // Cap calldata length to prevent gas-griefing with oversized EVM payloads.
    // 131072 hex chars = 64KB decoded, well above any legitimate EVM transaction.
    if req.calldata.trim().len() > 131_072 {
        return Err(bad_request(
            "calldata exceeds maximum length of 131072 hex characters",
        ));
    }
    let task = ironclad_db::revenue_swap_tasks::get_revenue_swap_task(&state.db, &opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| {
            not_found(format!(
                "revenue swap task for opportunity '{}' not found",
                opportunity_id
            ))
        })?;
    if !task.status.eq_ignore_ascii_case("in_progress") {
        return Err(bad_request(format!(
            "revenue swap task must be in_progress before submission (current: {})",
            task.status
        )));
    }
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue swap task source is missing or invalid JSON"))?;
    let source_obj = source
        .as_object()
        .ok_or_else(|| bad_request("revenue swap task source must be a JSON object"))?;
    // F1: Prevent double-submission — if a tx_hash is already recorded, direct to reconcile
    if source_obj
        .get("swap_tx_hash")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .is_some()
    {
        return Err(bad_request(
            "swap already submitted; use the reconcile endpoint to check on-chain status",
        ));
    }
    let target_chain = source_obj
        .get("target_chain")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue swap task is missing target_chain"))?;
    let wallet_chain = wallet_chain_label(state.wallet.wallet.chain_id())
        .ok_or_else(|| bad_request(
            "wallet is not configured for a supported chain",
        ))?;
    if !target_chain.eq_ignore_ascii_case(wallet_chain) {
        return Err(bad_request(
            "wallet is not configured for the requested target chain",
        ));
    }
    let from_currency = source_obj
        .get("from_currency")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue swap task is missing from_currency"))?;
    let amount = source_obj
        .get("amount")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| bad_request("revenue swap task is missing amount"))?;
    let contract_address = req
        .contract_address
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            source_obj
                .get("swap_contract_address")
                .and_then(|v| v.as_str())
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| {
            bad_request(
                "swap submission requires contract_address or configured swap_contract_address",
            )
        })?;
    let current_balance = current_source_balance(&state.wallet.wallet, from_currency)
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

    // Atomically claim the submission slot to prevent concurrent double-submission.
    // This transitions the task from "in_progress" to "submitting" as a database-level mutex.
    let claimed = ironclad_db::revenue_swap_tasks::claim_revenue_swap_submission(
        &state.db,
        &opportunity_id,
    )
    .map_err(|e| internal_err(&e))?;
    if !claimed {
        return Err(bad_request(
            "revenue swap task is not available for submission; \
             another submission may be in progress",
        ));
    }

    let tx_hash = match ironclad_wallet::submit_evm_contract_call(
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
    {
        Ok(hash) => hash,
        Err(e) => {
            // Release the claim so the operator can retry after fixing the issue.
            let _ = ironclad_db::revenue_swap_tasks::release_revenue_swap_claim(
                &state.db,
                &opportunity_id,
            );
            return Err(bad_request(e.to_string()));
        }
    };

    tracing::info!(
        opportunity_id = %opportunity_id,
        tx_hash = %tx_hash,
        "swap EVM transaction submitted; persisting tx_hash"
    );
    let updated =
        ironclad_db::revenue_swap_tasks::mark_revenue_swap_submitted(&state.db, &opportunity_id, &tx_hash)
            .map_err(|e| {
                tracing::error!(
                    opportunity_id = %opportunity_id,
                    tx_hash = %tx_hash,
                    error = %e,
                    "CRITICAL: EVM swap tx was submitted on-chain but DB write failed; tx_hash may be lost"
                );
                internal_err(&e)
            })?;
    if !updated {
        // EVM transaction already submitted on-chain — we MUST return the tx_hash
        // even though the DB status guard rejected the write (concurrent status change).
        tracing::error!(
            opportunity_id = %opportunity_id,
            tx_hash = %tx_hash,
            "CRITICAL: swap EVM tx submitted but mark_submitted returned false; \
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
        "revenue_swap_submission",
        amount,
        from_currency,
        Some(contract_address.as_str()),
        Some(tx_hash.as_str()),
        Some(
            &json!({
                "opportunity_id": opportunity_id,
                "target_chain": target_chain,
                "status": "submitted",
            })
            .to_string(),
        ),
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

pub async fn confirm_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueSwapConfirmRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let tx_hash = req.tx_hash.trim();
    if tx_hash.is_empty() || tx_hash.len() > 128 {
        return Err(bad_request("tx_hash must be non-empty and <= 128 chars"));
    }
    mark_swap_confirmed_with_metrics(&state.db, &opportunity_id, tx_hash)?;
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "completed",
        "tx_hash": tx_hash,
    })))
}

pub async fn reconcile_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let task = ironclad_db::revenue_swap_tasks::get_revenue_swap_task(&state.db, &opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| {
            not_found(format!(
                "revenue swap task for opportunity '{}' not found",
                opportunity_id
            ))
        })?;
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue swap task source is missing or invalid JSON"))?;
    let tx_hash = source
        .get("swap_tx_hash")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue swap task does not have a submitted tx_hash"))?;
    match ironclad_wallet::get_evm_transaction_receipt_status(&state.wallet.wallet, tx_hash)
        .await
        .map_err(|e| bad_request(e.to_string()))?
    {
        None => Ok(axum::Json(json!({
            "opportunity_id": opportunity_id,
            "status": task.status,
            "tx_hash": tx_hash,
            "reconciled": false,
            "receipt_status": "pending",
        }))),
        Some(true) => {
            mark_swap_confirmed_with_metrics(&state.db, &opportunity_id, tx_hash)?;
            Ok(axum::Json(json!({
                "opportunity_id": opportunity_id,
                "status": "completed",
                "tx_hash": tx_hash,
                "reconciled": true,
                "receipt_status": "confirmed",
            })))
        }
        Some(false) => {
            let updated = ironclad_db::revenue_swap_tasks::mark_revenue_swap_failed(
                &state.db,
                &opportunity_id,
                "on-chain receipt status=failed",
            )
            .map_err(|e| internal_err(&e))?;
            if !updated {
                return Err(not_found(format!(
                    "revenue swap task for opportunity '{}' not found",
                    opportunity_id
                )));
            }
            Ok(axum::Json(json!({
                "opportunity_id": opportunity_id,
                "status": "failed",
                "tx_hash": tx_hash,
                "reconciled": true,
                "receipt_status": "failed",
            })))
        }
    }
}

pub async fn fail_revenue_swap_task(
    State(state): State<AppState>,
    Path(opportunity_id): Path<String>,
    Json(req): Json<RevenueSwapFailRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let reason = req.reason.trim();
    if reason.is_empty() {
        return Err(bad_request("reason must be non-empty"));
    }
    let updated = ironclad_db::revenue_swap_tasks::mark_revenue_swap_failed(
        &state.db,
        &opportunity_id,
        reason,
    )
    .map_err(|e| internal_err(&e))?;
    if !updated {
        return Err(not_found(format!(
            "revenue swap task for opportunity '{}' not found",
            opportunity_id
        )));
    }
    Ok(axum::Json(json!({
        "opportunity_id": opportunity_id,
        "status": "failed",
        "reason": reason,
    })))
}

fn wallet_chain_label(chain_id: u64) -> Option<&'static str> {
    match chain_id {
        1 => Some("ETH"),
        56 => Some("BSC"),
        10 => Some("OPTIMISM"),
        137 => Some("POLYGON"),
        42161 => Some("ARBITRUM"),
        8453 => Some("BASE"),
        _ => None,
    }
}

async fn current_source_balance(
    wallet: &ironclad_wallet::Wallet,
    from_currency: &str,
) -> ironclad_core::Result<f64> {
    match from_currency.to_ascii_uppercase().as_str() {
        "USDC" => wallet.get_usdc_balance().await,
        other => {
            let token = wallet
                .get_all_balances()
                .await
                .into_iter()
                .find(|t| t.symbol.eq_ignore_ascii_case(other))
                .ok_or_else(|| {
                    ironclad_core::IroncladError::Wallet(format!(
                        "source asset '{}' is not available on wallet chain '{}'",
                        other,
                        wallet.network_name()
                    ))
                })?;
            Ok(token.balance)
        }
    }
}

fn mark_swap_confirmed_with_metrics(
    db: &ironclad_db::Database,
    opportunity_id: &str,
    tx_hash: &str,
) -> Result<(), JsonError> {
    let task = ironclad_db::revenue_swap_tasks::get_revenue_swap_task(db, opportunity_id)
        .map_err(|e| internal_err(&e))?
        .ok_or_else(|| {
            not_found(format!(
                "revenue swap task for opportunity '{}' not found",
                opportunity_id
            ))
        })?;
    let source = task
        .source_json
        .as_deref()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .ok_or_else(|| bad_request("revenue swap task source is missing or invalid JSON"))?;
    // Guard: a transaction must have been submitted (swap_tx_hash recorded) before
    // confirmation is allowed.  Without this, a caller could confirm a swap that was
    // never actually submitted on-chain, marking the task complete without moving funds.
    if source.get("swap_tx_hash").and_then(|v| v.as_str()).map_or(true, str::is_empty) {
        return Err(bad_request(format!(
            "revenue swap task for opportunity '{}' has no prior submission (swap_tx_hash missing); \
             submit the transaction before confirming",
            opportunity_id
        )));
    }
    let amount = source
        .get("amount")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| bad_request("revenue swap task is missing amount"))?;
    let currency = source
        .get("from_currency")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| bad_request("revenue swap task is missing from_currency"))?;
    let updated =
        ironclad_db::revenue_swap_tasks::mark_revenue_swap_confirmed(db, opportunity_id, tx_hash)
            .map_err(|e| internal_err(&e))?;
    if !updated {
        // Task was already fetched above (would have returned 404 if missing),
        // so !updated means status is not in_progress (already completed/failed).
        return Err(bad_request(format!(
            "revenue swap task for opportunity '{}' is not in_progress; may already be completed or failed",
            opportunity_id
        )));
    }
    ironclad_db::metrics::record_transaction_with_metadata(
        db,
        "revenue_swap_execution",
        amount,
        currency,
        Some("revenue_swap"),
        Some(tx_hash),
        Some(
            &serde_json::to_string(&json!({
                "opportunity_id": opportunity_id,
                "status": "completed",
                "amount": amount,
                "currency": currency,
            }))
            .map_err(|e| internal_err(&ironclad_core::IroncladError::Database(e.to_string())))?,
        ),
    )
    .map_err(|e| internal_err(&e))?;
    Ok(())
}
