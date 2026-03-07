pub async fn wallet_balance(State(state): State<AppState>) -> impl IntoResponse {
    let balances = state.wallet.wallet.get_all_balances().await;
    let address = state.wallet.wallet.address();
    let chain_id = state.wallet.wallet.chain_id();
    let network = state.wallet.wallet.network_name();
    let config = state.config.read().await;
    let revenue_accounting =
        ironclad_db::revenue_accounting::revenue_accounting_summary(&state.db).unwrap_or_default();
    let revenue_swap_queue =
        ironclad_db::revenue_accounting::revenue_swap_queue_summary(&state.db).unwrap_or_default();
    let revenue_strategy_summary =
        ironclad_db::revenue_strategy_summary::revenue_strategy_summary(&state.db)
            .unwrap_or_default();
    let revenue_feedback_summary =
        ironclad_db::revenue_feedback::revenue_feedback_summary_by_strategy(&state.db)
            .unwrap_or_default();
    let default_swap_chain = config.treasury.revenue_swap.default_chain.clone();
    let default_swap_chain_cfg = config
        .treasury
        .revenue_swap
        .chains
        .iter()
        .find(|c| c.chain.trim().eq_ignore_ascii_case(&default_swap_chain));
    let revenue_swap_chains: Vec<serde_json::Value> = config
        .treasury
        .revenue_swap
        .chains
        .iter()
        .map(|chain| {
            json!({
                "chain": chain.chain,
                "target_contract_address": chain.target_contract_address,
                "swap_contract_address": chain.swap_contract_address,
            })
        })
        .collect();

    // Backward compat: "balance" field is still the USDC balance
    let usdc_balance = balances
        .iter()
        .find(|b| b.symbol == "USDC")
        .map(|b| b.balance)
        .unwrap_or(0.0);

    let tokens: Vec<serde_json::Value> = balances
        .iter()
        .map(|b| {
            json!({
                "symbol": b.symbol,
                "name": b.name,
                "balance": b.balance,
                "formatted": format_balance(b.balance, &b.symbol),
                "contract": b.contract,
                "decimals": b.decimals,
                "is_native": b.is_native,
            })
        })
        .collect();
    let stable_balance = balances
        .iter()
        .filter(|b| matches!(b.symbol.as_str(), "USDC" | "USDT" | "DAI"))
        .map(|b| b.balance)
        .sum::<f64>();
    let seed_target_usdc = 50.0;
    let seed_readiness = json!({
        "seed_target_usdc": seed_target_usdc,
        "stable_balance_usdc": stable_balance,
        "meets_seed_target": stable_balance >= seed_target_usdc,
        "minimum_reserve_configured": config.treasury.minimum_reserve > 0.0,
        "swap_enabled": config.treasury.revenue_swap.enabled,
        "default_chain": default_swap_chain,
        "default_chain_has_target_contract": default_swap_chain_cfg
            .map(|c| !c.target_contract_address.trim().is_empty())
            .unwrap_or(false),
        "default_chain_has_swap_contract": default_swap_chain_cfg
            .and_then(|c| c.swap_contract_address.as_deref())
            .map(|s| !s.trim().is_empty())
            .unwrap_or(false),
    });

    axum::Json(json!({
        "balance": format!("{usdc_balance:.2}"),
        "currency": "USDC",
        "address": address,
        "chain_id": chain_id,
        "network": network,
        "tokens": tokens,
        "treasury": {
            "per_payment_cap": config.treasury.per_payment_cap,
            "daily_inference_budget": config.treasury.daily_inference_budget,
            "minimum_reserve": config.treasury.minimum_reserve,
            "revenue_swap": {
                "enabled": config.treasury.revenue_swap.enabled,
                "target_symbol": config.treasury.revenue_swap.target_symbol,
                "default_chain": config.treasury.revenue_swap.default_chain,
                "chains": revenue_swap_chains,
            },
        },
        "self_funding": {
            "tax": {
                "enabled": config.self_funding.tax.enabled,
                "rate": config.self_funding.tax.rate,
                "destination_wallet": config.self_funding.tax.destination_wallet,
            },
        },
        "revenue_accounting": {
            "settled_jobs": revenue_accounting.settled_jobs,
            "gross_revenue_usdc": revenue_accounting.gross_revenue_usdc,
            "attributable_costs_usdc": revenue_accounting.attributable_costs_usdc,
            "net_profit_usdc": revenue_accounting.net_profit_usdc,
            "tax_paid_usdc": revenue_accounting.tax_paid_usdc,
            "retained_earnings_usdc": revenue_accounting.retained_earnings_usdc,
        },
        "revenue_swap_queue": {
            "total": revenue_swap_queue.total,
            "pending": revenue_swap_queue.pending,
            "in_progress": revenue_swap_queue.in_progress,
            "failed": revenue_swap_queue.failed,
            "completed": revenue_swap_queue.completed,
            "stale_in_progress": revenue_swap_queue.stale_in_progress,
        },
        "revenue_strategy_summary": revenue_strategy_summary,
        "revenue_feedback_summary": revenue_feedback_summary,
        "seed_exercise_readiness": seed_readiness,
    }))
}

fn format_balance(balance: f64, symbol: &str) -> String {
    match symbol {
        "USDC" | "USDT" | "DAI" => format!("{balance:.2}"),
        "ETH" | "WETH" | "MATIC" => format!("{balance:.6}"),
        "WBTC" | "cbBTC" => format!("{balance:.8}"),
        _ => format!("{balance:.4}"),
    }
}

fn plugin_tool_required_permissions(tool_name: &str, input: &Value) -> Vec<&'static str> {
    let mut required = Vec::new();
    let _ = tool_name;
    let scan = input_capability_scan::scan_input_capabilities(input);
    if scan.requires_filesystem {
        required.push("filesystem");
    }
    if scan.requires_network && !required.contains(&"network") {
        required.push("network");
    }
    required
}

pub async fn wallet_address(State(state): State<AppState>) -> impl IntoResponse {
    let address = state.wallet.wallet.address().to_string();
    let chain_id = state.wallet.wallet.chain_id();
    let network = state.wallet.wallet.network_name();

    axum::Json(json!({
        "address": address,
        "chain_id": chain_id,
        "network": network,
    }))
}

pub async fn get_plugins(State(state): State<AppState>) -> impl IntoResponse {
    let plugins = state.plugins.list_plugins().await;
    let count = plugins.len();
    let tools = state.plugins.list_all_tools().await;
    Json(json!({
        "plugins": plugins,
        "count": count,
        "total_tools": tools.len(),
    }))
}

pub async fn toggle_plugin(
    State(state): State<AppState>,
    axum::extract::Path(name): axum::extract::Path<String>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let plugins = state.plugins.list_plugins().await;
    let current = plugins.iter().find(|p| p.name == name);

    match current {
        Some(info) => {
            let result = if info.status == ironclad_plugin_sdk::PluginStatus::Active {
                state.plugins.disable_plugin(&name).await
            } else {
                state.plugins.enable_plugin(&name).await
            };

            match result {
                Ok(()) => {
                    let new_plugins = state.plugins.list_plugins().await;
                    let new_status = new_plugins
                        .iter()
                        .find(|p| p.name == name)
                        .map(|p| p.status);
                    Ok(Json(json!({
                        "name": name,
                        "toggled": true,
                        "status": new_status,
                    })))
                }
                Err(e) => Err(internal_err(&e)),
            }
        }
        None => Err(not_found(format!("plugin '{name}' not found"))),
    }
}

pub async fn execute_plugin_tool(
    State(state): State<AppState>,
    axum::extract::Path((name, tool)): axum::extract::Path<(String, String)>,
    Json(body): Json<Value>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let found = state.plugins.find_tool(&tool).await;
    match found {
        Some((plugin_name, tool_def)) if plugin_name == name => {
            let declared_permissions: Vec<String> = tool_def
                .permissions
                .iter()
                .map(|p| p.to_lowercase())
                .collect();
            let required_permissions = plugin_tool_required_permissions(&tool, &body);
            let missing: Vec<&str> = required_permissions
                .iter()
                .copied()
                .filter(|need| !declared_permissions.iter().any(|p| p == need))
                .collect();
            if !missing.is_empty() {
                return Err(JsonError(
                    StatusCode::FORBIDDEN,
                    format!(
                        "plugin '{}' tool '{}' missing required permissions: {}",
                        name,
                        tool,
                        missing.join(", ")
                    ),
                ));
            }
            let call = ToolCallRequest {
                tool_name: tool.clone(),
                params: body.clone(),
                risk_level: tool_def.risk_level,
            };
            let ctx = PolicyContext {
                authority: InputAuthority::External,
                survival_tier: SurvivalTier::Normal,
                claim: None,
            };
            let decision = state.policy_engine.evaluate_all(&call, &ctx);
            if !decision.is_allowed() {
                let reason = match &decision {
                    PolicyDecision::Deny { reason, .. } => reason.clone(),
                    _ => "policy denied".into(),
                };
                return Err(JsonError(StatusCode::FORBIDDEN, reason));
            }
            match state.plugins.execute_tool(&tool, &body).await {
                Ok(result) => Ok(Json(json!({
                    "plugin": name,
                    "tool": tool,
                    "result": result,
                }))),
                Err(e) => Err(internal_err(&e)),
            }
        }
        Some((other_plugin, _)) => Err(bad_request(format!(
            "tool '{tool}' belongs to plugin '{other_plugin}', not '{name}'"
        ))),
        None => Err(not_found(format!(
            "tool '{tool}' not found in plugin '{name}'"
        ))),
    }
}

pub async fn browser_status(State(state): State<AppState>) -> impl IntoResponse {
    let running = state.browser.is_running().await;
    let config = state.config.read().await;
    Json(json!({
        "running": running,
        "enabled": config.browser.enabled,
        "headless": config.browser.headless,
        "cdp_port": config.browser.cdp_port,
    }))
}

pub async fn browser_start(
    State(state): State<AppState>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    if state.browser.is_running().await {
        return Ok(Json(json!({"status": "already_running"})));
    }
    match state.browser.start().await {
        Ok(()) => Ok(Json(json!({
            "status": "started",
            "cdp_port": state.browser.cdp_port(),
        }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn browser_stop(
    State(state): State<AppState>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    match state.browser.stop().await {
        Ok(()) => Ok(Json(json!({"status": "stopped"}))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn browser_action(
    State(state): State<AppState>,
    Json(action): Json<ironclad_browser::actions::BrowserAction>,
) -> impl IntoResponse {
    let result = state.browser.execute_action(&action).await;
    let status = if result.success {
        StatusCode::OK
    } else {
        StatusCode::BAD_REQUEST
    };
    (status, Json(json!(result)))
}

pub async fn get_agents(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.registry.list_agents().await;
    let count = agents.len();
    Json(json!({"agents": agents, "count": count}))
}

pub async fn start_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    state.registry.start_agent(&id).await.map_err(not_found)?;
    let event = json!({"type": "agent_started", "agent_id": id});
    state.event_bus.publish(event.to_string());
    Ok(Json(json!({"id": id, "action": "started"})))
}

pub async fn stop_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    state.registry.stop_agent(&id).await.map_err(not_found)?;
    let event = json!({"type": "agent_stopped", "agent_id": id});
    state.event_bus.publish(event.to_string());
    Ok(Json(json!({"id": id, "action": "stopped"})))
}

const WORKSPACE_PALETTE: &[&str] = &[
    "#6366f1", "#22c55e", "#f59e0b", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#14b8a6",
    "#f97316", "#84cc16", "#a855f7", "#0ea5e9",
];
const ROLE_SUBAGENT: &str = "subagent";
const ROLE_MODEL_PROXY: &str = "model-proxy";

const WORKSPACE_ACTIVITY_WINDOW_SECS: i64 = 120;
