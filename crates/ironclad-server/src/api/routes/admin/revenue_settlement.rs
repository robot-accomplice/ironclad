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
    let attributable_costs_usdc = req.attributable_costs_usdc.unwrap_or(0.0);
    if attributable_costs_usdc < 0.0 {
        return Err(bad_request(
            "attributable_costs_usdc must be non-negative",
        ));
    }
    if attributable_costs_usdc > req.amount_usdc {
        return Err(bad_request(
            "attributable_costs_usdc cannot exceed amount_usdc",
        ));
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
    let (swap_policy, tax_policy) = {
        let config = state.config.read().await;
        (
            config.treasury.revenue_swap.clone(),
            config.self_funding.tax.clone(),
        )
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

    let net_profit_usdc = req.amount_usdc - attributable_costs_usdc;
    let tax_rate = if tax_policy.enabled { tax_policy.rate } else { 0.0 };
    let tax_amount_usdc = (net_profit_usdc.max(0.0) * tax_rate * 100.0).round() / 100.0;
    let retained_earnings_usdc = (net_profit_usdc - tax_amount_usdc).max(0.0);
    let tax_destination_wallet = tax_policy
        .destination_wallet
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let result = ironclad_db::service_revenue::settle_revenue_opportunity(
        &state.db,
        &id,
        settlement_ref,
        req.amount_usdc,
        &ironclad_db::service_revenue::RevenueSettlementAccounting {
            attributable_costs_usdc,
            tax_rate,
            tax_amount_usdc,
            retained_earnings_usdc,
            tax_destination_wallet,
        },
    )
    .map_err(|e| internal_err(&e))?;
    match &result {
        ironclad_db::service_revenue::SettlementResult::NotFound => {
            return Err(not_found(format!("revenue opportunity '{id}' not found")));
        }
        ironclad_db::service_revenue::SettlementResult::WrongState(reason) => {
            return Err(bad_request(reason.clone()));
        }
        _ => {}
    }
    let mut swap_queued = false;
    if matches!(
        result,
        ironclad_db::service_revenue::SettlementResult::Settled
    ) {
        let settlement_metadata = json!({
            "opportunity_id": id,
            "settlement_ref": settlement_ref,
            "gross_revenue_usdc": req.amount_usdc,
            "attributable_costs_usdc": attributable_costs_usdc,
            "net_profit_usdc": net_profit_usdc,
            "tax_rate": tax_rate,
            "tax_amount_usdc": tax_amount_usdc,
            "retained_earnings_usdc": retained_earnings_usdc,
        })
        .to_string();
        ironclad_db::metrics::record_transaction_with_metadata(
            &state.db,
            "revenue_settlement",
            req.amount_usdc,
            settlement_currency.as_str(),
            Some("revenue_control_plane"),
            Some(settlement_ref),
            Some(&settlement_metadata),
        )
        .map_err(|e| internal_err(&e))?;
        if tax_amount_usdc > 0.0 {
            let tax_metadata = json!({
                "opportunity_id": id,
                "settlement_ref": settlement_ref,
                "tax_rate": tax_rate,
                "source_net_profit_usdc": net_profit_usdc,
                "destination_wallet": tax_destination_wallet,
            })
            .to_string();
            ironclad_db::metrics::record_transaction_with_metadata(
                &state.db,
                "revenue_tax",
                tax_amount_usdc,
                settlement_currency.as_str(),
                tax_destination_wallet,
                Some(settlement_ref),
                Some(&tax_metadata),
            )
            .map_err(|e| internal_err(&e))?;
            if let Some(destination_wallet) = tax_destination_wallet {
                let tax_chain_config = swap_policy
                    .chains
                    .iter()
                    .find(|c| c.chain.trim().eq_ignore_ascii_case(&target_chain));
                let tax_contract_address = tax_chain_config
                    .map(|c| c.target_contract_address.as_str());
                queue_revenue_tax_payout(
                    &state.db,
                    RevenueTaxPayoutTask {
                        opportunity_id: &id,
                        amount: tax_amount_usdc,
                        currency: settlement_currency.as_str(),
                        target_chain: target_chain.as_str(),
                        destination_wallet,
                        contract_address: tax_contract_address,
                    },
                )
                .map_err(|e| internal_err(&e))?;
            }
        }
        let retained_metadata = json!({
            "opportunity_id": id,
            "settlement_ref": settlement_ref,
            "net_profit_usdc": net_profit_usdc,
            "tax_amount_usdc": tax_amount_usdc,
        })
        .to_string();
        ironclad_db::metrics::record_transaction_with_metadata(
            &state.db,
            "revenue_retained",
            retained_earnings_usdc,
            settlement_currency.as_str(),
            Some("treasury"),
            Some(settlement_ref),
            Some(&retained_metadata),
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
                RevenueSwapTask {
                    opportunity_id: &id,
                    amount: req.amount_usdc,
                    from_currency: settlement_currency.as_str(),
                    target_symbol: target_symbol.as_str(),
                    target_chain: target_chain.as_str(),
                    target_contract_address,
                    swap_contract_address: swap_contract_address.as_deref(),
                },
            )
            .map_err(|e| internal_err(&e))?;
            swap_queued = true;
        }
    } else if matches!(
        result,
        ironclad_db::service_revenue::SettlementResult::AlreadySettled
    ) {
        tracing::info!(
            opportunity_id = %id,
            settlement_ref = %settlement_ref,
            auto_swap = auto_swap,
            tax_amount_usdc = tax_amount_usdc,
            "settlement idempotent replay: secondary accounting (metrics, swap/tax task queuing) \
             was skipped; verify swap/tax tasks exist if this was a crash-recovery retry"
        );
    }

    Ok(axum::Json(json!({
        "opportunity_id": id,
        "status": ironclad_db::service_revenue::OPPORTUNITY_STATUS_SETTLED,
        "swap_queued": swap_queued,
        "swap_target_asset": target_symbol,
        "swap_target_chain": target_chain,
        "auto_swap": auto_swap,
        "gross_revenue_usdc": req.amount_usdc,
        "attributable_costs_usdc": attributable_costs_usdc,
        "net_profit_usdc": net_profit_usdc,
        "tax_rate": tax_rate,
        "tax_amount_usdc": tax_amount_usdc,
        "retained_earnings_usdc": retained_earnings_usdc,
        "tax_destination_wallet": tax_destination_wallet,
        "idempotent": matches!(
            result,
            ironclad_db::service_revenue::SettlementResult::AlreadySettled
        ),
    })))
}

struct RevenueSwapTask<'a> {
    opportunity_id: &'a str,
    amount: f64,
    from_currency: &'a str,
    target_symbol: &'a str,
    target_chain: &'a str,
    target_contract_address: &'a str,
    swap_contract_address: Option<&'a str>,
}

struct RevenueTaxPayoutTask<'a> {
    opportunity_id: &'a str,
    amount: f64,
    currency: &'a str,
    target_chain: &'a str,
    destination_wallet: &'a str,
    contract_address: Option<&'a str>,
}

fn queue_revenue_swap(
    db: &ironclad_db::Database,
    task: RevenueSwapTask<'_>,
) -> std::result::Result<(), ironclad_core::IroncladError> {
    let conn = db.conn();
    let source_json = json!({
        "origin": "revenue_settlement",
        "type": "revenue_swap",
        "from_currency": task.from_currency,
        "target_asset": task.target_symbol,
        "target_chain": task.target_chain,
        "target_contract_address": task.target_contract_address,
        "swap_contract_address": task.swap_contract_address,
        "amount": task.amount,
        "opportunity_id": task.opportunity_id,
    })
    .to_string();
    let title = format!(
        "Swap {} {:.6} to {} on {}",
        task.from_currency, task.amount, task.target_symbol, task.target_chain
    );
    let task_id = format!("rev_swap:{}", task.opportunity_id);
    let description = format!(
        "Auto-queued stablecoin conversion to {} on {} after revenue settlement",
        task.target_symbol, task.target_chain
    );
    conn.execute(
        "INSERT INTO tasks (id, title, description, status, priority, source, created_at, updated_at) \
         VALUES (?1, ?2, ?3, 'pending', 95, ?4, datetime('now'), datetime('now')) \
         ON CONFLICT(id) DO UPDATE SET \
           title=excluded.title, description=excluded.description, status='pending', priority=95, source=excluded.source, updated_at=datetime('now')",
        rusqlite::params![
            task_id,
            title,
            description,
            source_json
        ],
    )
    .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
    Ok(())
}

fn queue_revenue_tax_payout(
    db: &ironclad_db::Database,
    task: RevenueTaxPayoutTask<'_>,
) -> std::result::Result<(), ironclad_core::IroncladError> {
    let conn = db.conn();
    let source_json = json!({
        "origin": "revenue_settlement",
        "type": "revenue_tax_payout",
        "currency": task.currency,
        "target_chain": task.target_chain,
        "destination_wallet": task.destination_wallet,
        "contract_address": task.contract_address,
        "amount": task.amount,
        "opportunity_id": task.opportunity_id,
    })
    .to_string();
    let title = format!(
        "Tax payout {} {:.6} to {} on {}",
        task.currency, task.amount, task.destination_wallet, task.target_chain
    );
    let task_id = format!("rev_tax:{}", task.opportunity_id);
    conn.execute(
        "INSERT INTO tasks (id, title, description, status, priority, source, created_at, updated_at)          VALUES (?1, ?2, ?3, 'pending', 96, ?4, datetime('now'), datetime('now'))          ON CONFLICT(id) DO UPDATE SET            title=excluded.title, description=excluded.description, status='pending', priority=96, source=excluded.source, updated_at=datetime('now')",
        rusqlite::params![
            task_id,
            title,
            "Auto-queued profit tax payout after revenue settlement",
            source_json
        ],
    )
    .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
    Ok(())
}
