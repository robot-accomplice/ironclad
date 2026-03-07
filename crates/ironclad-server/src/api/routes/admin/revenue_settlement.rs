pub async fn settle_revenue_opportunity(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(req): Json<RevenueOpportunitySettleRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let settlement_ref = req.settlement_ref.trim();
    if settlement_ref.is_empty() {
        return Err(bad_request("settlement_ref cannot be empty"));
    }
    if req.amount_usdc <= 0.0 {
        return Err(bad_request("amount_usdc must be positive"));
    }
    let settlement_currency = req.currency.to_ascii_uppercase();
    if settlement_currency != "USDC"
        && settlement_currency != "USDT"
        && settlement_currency != "DAI"
    {
        return Err(bad_request(
            "only USDC, USDT, or DAI settlements are supported",
        ));
    }
    let swap_policy = {
        let config = state.config.read().await;
        config.treasury.revenue_swap.clone()
    };
    let auto_swap = req.auto_swap.unwrap_or(swap_policy.enabled);
    let target_symbol = req
        .target_symbol
        .as_deref()
        .unwrap_or(swap_policy.target_symbol.as_str())
        .trim()
        .to_string();
    let target_chain = req
        .target_chain
        .as_deref()
        .unwrap_or(swap_policy.default_chain.as_str())
        .trim()
        .to_ascii_uppercase();
    if auto_swap && target_symbol.is_empty() {
        return Err(bad_request(
            "target_symbol must be non-empty when auto_swap is enabled",
        ));
    }
    if auto_swap && target_chain.is_empty() {
        return Err(bad_request(
            "target_chain must be non-empty when auto_swap is enabled",
        ));
    }
    let planned_swap = if auto_swap {
        let configured_chain = swap_policy
            .chains
            .iter()
            .find(|c| c.chain.trim().eq_ignore_ascii_case(&target_chain));
        let target_contract_address = req
            .target_contract_address
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| configured_chain.map(|c| c.target_contract_address.trim().to_string()))
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                bad_request(
                    "auto_swap requires target_contract_address unless the target_chain is configured in treasury.revenue_swap.chains",
                )
            })?;
        let swap_contract_address = req
            .swap_contract_address
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .or_else(|| {
                configured_chain.and_then(|c| {
                    c.swap_contract_address
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
            });
        Some((target_contract_address, swap_contract_address))
    } else {
        None
    };

    let result = ironclad_db::service_revenue::settle_revenue_opportunity(
        &state.db,
        &id,
        settlement_ref,
        req.amount_usdc,
    )
    .map_err(|e| internal_err(&e))?;
    let mut swap_queued = false;
    if matches!(
        result,
        ironclad_db::service_revenue::SettlementResult::Settled
    ) {
        ironclad_db::metrics::record_transaction(
            &state.db,
            "revenue_settlement",
            req.amount_usdc,
            settlement_currency.as_str(),
            Some("revenue_control_plane"),
            Some(settlement_ref),
        )
        .map_err(|e| internal_err(&e))?;

        if auto_swap {
            let Some((target_contract_address, swap_contract_address)) = planned_swap.as_ref()
            else {
                return Err(internal_err(
                    &"auto_swap was enabled without a planned swap after validation",
                ));
            };

            queue_revenue_swap(
                &state.db,
                &id,
                req.amount_usdc,
                settlement_currency.as_str(),
                target_symbol.as_str(),
                target_chain.as_str(),
                target_contract_address,
                swap_contract_address.as_deref(),
            )
            .map_err(|e| internal_err(&e))?;
            swap_queued = true;
        }
    }

    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_SETTLED,
        "swap_queued": swap_queued,
        "swap_target_asset": target_symbol,
        "swap_target_chain": target_chain,
        "auto_swap": auto_swap,
        "idempotent": matches!(
            result,
            ironclad_db::service_revenue::SettlementResult::AlreadySettled
        ),
    })))
}

fn queue_revenue_swap(
    db: &ironclad_db::Database,
    opportunity_id: &str,
    amount: f64,
    from_currency: &str,
    target_symbol: &str,
    target_chain: &str,
    target_contract_address: &str,
    swap_contract_address: Option<&str>,
) -> std::result::Result<(), ironclad_core::IroncladError> {
    let conn = db.conn();
    let source_json = json!({
        "origin": "revenue_settlement",
        "type": "revenue_swap",
        "from_currency": from_currency,
        "target_asset": target_symbol,
        "target_chain": target_chain,
        "target_contract_address": target_contract_address,
        "swap_contract_address": swap_contract_address,
        "amount": amount,
        "opportunity_id": opportunity_id,
    })
    .to_string();
    let title = format!(
        "Swap {} {:.6} to {} on {}",
        from_currency, amount, target_symbol, target_chain
    );
    let task_id = format!("rev_swap:{opportunity_id}");
    conn.execute(
        "INSERT INTO tasks (id, title, description, status, priority, source, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 'pending', 95, ?4, datetime('now'), datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET \
           title=excluded.title, description=excluded.description, status='pending', priority=95, source=excluded.source, updated_at=datetime('now')",
        rusqlite::params![
            task_id,
            title,
            "Auto-queued immediate stablecoin conversion to Palm USD after revenue settlement",
            source_json
        ],
    )
    .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
    Ok(())
}

