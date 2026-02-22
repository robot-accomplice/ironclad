//! Config, stats, circuit breaker, wallet, plugins, browser, agents, workspace, A2A.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use serde::Deserialize;
use serde_json::{Value, json};

use ironclad_agent::policy::{PolicyContext, ToolCallRequest};
use ironclad_core::{InputAuthority, IroncladConfig, PolicyDecision, RiskLevel, SurvivalTier};

use super::{internal_err, AppState};

#[derive(Deserialize)]
pub struct UpdateConfigRequest {
    #[serde(flatten)]
    pub patch: Value,
}

#[derive(Deserialize)]
pub struct TransactionsQuery {
    pub hours: Option<i64>,
}

#[derive(Deserialize)]
pub struct A2aHelloRequest {
    #[serde(flatten)]
    pub hello: Value,
}

fn merge_json(base: &mut Value, patch: &Value) {
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                let entry = base_map.entry(k.clone()).or_insert(Value::Null);
                merge_json(entry, v);
            }
        }
        (base, patch) => {
            *base = patch.clone();
        }
    }
}

pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let mut cfg = serde_json::to_value(&*config).unwrap_or_default();
    if let Some(providers) = cfg.get_mut("providers")
        && let Some(obj) = providers.as_object_mut() {
            for (_name, provider) in obj.iter_mut() {
                if let Some(p) = provider.as_object_mut() {
                    p.remove("api_key");
                    p.remove("secret");
                    p.remove("token");
                }
            }
        }
    if let Some(wallet) = cfg.get_mut("wallet")
        && let Some(w) = wallet.as_object_mut() {
            w.remove("private_key");
            w.remove("mnemonic");
        }
    axum::Json(cfg)
}

pub async fn update_config(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    const IMMUTABLE_KEYS: &[&str] = &["server", "treasury", "a2a", "wallet"];
    if let Some(obj) = body.patch.as_object() {
        for key in IMMUTABLE_KEYS {
            if obj.contains_key(*key) {
                return Err((
                    StatusCode::FORBIDDEN,
                    format!("cannot modify '{key}' at runtime; edit ironclad.toml and restart"),
                ));
            }
        }
    }

    let mut config = state.config.write().await;
    let mut current = serde_json::to_value(&*config)
        .map_err(|e| internal_err(&e))?;

    merge_json(&mut current, &body.patch);

    let updated: IroncladConfig = serde_json::from_value(current)
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("invalid config: {e}")))?;

    updated
        .validate()
        .map_err(|e| (StatusCode::BAD_REQUEST, format!("validation failed: {e}")))?;

    *config = updated;

    Ok::<_, (StatusCode, String)>(axum::Json(json!({
        "updated": true,
        "message": "configuration updated (runtime only, not persisted to disk)",
    })))
}

pub async fn get_costs(State(state): State<AppState>) -> impl IntoResponse {
    let conn = state.db.conn();
    let mut stmt = conn
        .prepare(
            "SELECT id, model, provider, tokens_in, tokens_out, cost, tier, cached, created_at \
             FROM inference_costs ORDER BY created_at DESC LIMIT 100",
        )
        .map_err(|e| internal_err(&e))?;

    let rows = stmt
        .query_map([], |row| {
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "model": row.get::<_, String>(1)?,
                "provider": row.get::<_, String>(2)?,
                "tokens_in": row.get::<_, i64>(3)?,
                "tokens_out": row.get::<_, i64>(4)?,
                "cost": row.get::<_, f64>(5)?,
                "tier": row.get::<_, Option<String>>(6)?,
                "cached": row.get::<_, i32>(7)? != 0,
                "created_at": row.get::<_, String>(8)?,
            }))
        })
        .map_err(|e| internal_err(&e))?;

    let costs: Vec<Value> = rows.filter_map(|r| r.ok()).collect();
    Ok::<_, (StatusCode, String)>(axum::Json(json!({ "costs": costs })))
}

pub async fn get_transactions(
    State(state): State<AppState>,
    Query(params): Query<TransactionsQuery>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(24);
    match ironclad_db::metrics::query_transactions(&state.db, hours) {
        Ok(txs) => {
            let items: Vec<Value> = txs
                .into_iter()
                .map(|t| {
                    json!({
                        "id": t.id,
                        "tx_type": t.tx_type,
                        "amount": t.amount,
                        "currency": t.currency,
                        "counterparty": t.counterparty,
                        "tx_hash": t.tx_hash,
                        "metadata_json": t.metadata_json,
                        "created_at": t.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(json!({ "transactions": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_cache_stats(State(state): State<AppState>) -> impl IntoResponse {
    let llm = state.llm.read().await;
    let hits = llm.cache.hit_count() as u64;
    let misses = llm.cache.miss_count() as u64;
    let entries = llm.cache.size() as u64;
    let total = hits + misses;
    let hit_rate = if total > 0 {
        (hits as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    axum::Json(json!({
        "hits": hits,
        "misses": misses,
        "entries": entries,
        "hit_rate": hit_rate,
    }))
}

pub async fn breaker_status(State(state): State<AppState>) -> impl IntoResponse {
    let llm = state.llm.read().await;
    let providers = llm.breakers.list_providers();
    let config = state.config.read().await;

    let mut provider_states = serde_json::Map::new();
    for (name, circuit_state) in &providers {
        let state_str = match circuit_state {
            ironclad_llm::CircuitState::Closed => "closed",
            ironclad_llm::CircuitState::Open => "open",
            ironclad_llm::CircuitState::HalfOpen => "half_open",
        };
        provider_states.insert(
            name.clone(),
            json!({
                "state": state_str,
                "blocked": *circuit_state == ironclad_llm::CircuitState::Open,
            }),
        );
    }

    for name in config.providers.keys() {
        if !provider_states.contains_key(name) {
            provider_states.insert(
                name.clone(),
                json!({ "state": "closed", "blocked": false }),
            );
        }
    }

    axum::Json(json!({
        "providers": Value::Object(provider_states),
        "config": {
            "threshold": config.circuit_breaker.threshold,
            "cooldown_seconds": config.circuit_breaker.cooldown_seconds,
            "max_cooldown_seconds": config.circuit_breaker.max_cooldown_seconds,
            "window_seconds": config.circuit_breaker.window_seconds,
        },
    }))
}

pub async fn breaker_reset(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let mut llm = state.llm.write().await;
    llm.breakers.reset(&provider);

    axum::Json(json!({
        "provider": provider,
        "state": "closed",
        "reset": true,
    }))
}

pub async fn wallet_balance(State(state): State<AppState>) -> impl IntoResponse {
    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let address = state.wallet.wallet.address();
    let chain_id = state.wallet.wallet.chain_id();
    let config = state.config.read().await;

    axum::Json(json!({
        "balance": format!("{balance:.2}"),
        "currency": "USDC",
        "address": address,
        "chain_id": chain_id,
        "treasury": {
            "per_payment_cap": config.treasury.per_payment_cap,
            "daily_inference_budget": config.treasury.daily_inference_budget,
            "minimum_reserve": config.treasury.minimum_reserve,
        },
    }))
}

pub async fn wallet_address(State(state): State<AppState>) -> impl IntoResponse {
    let address = state.wallet.wallet.address().to_string();
    let chain_id = state.wallet.wallet.chain_id();

    axum::Json(json!({
        "address": address,
        "chain_id": chain_id,
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
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
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
        None => Err((StatusCode::NOT_FOUND, format!("plugin '{name}' not found"))),
    }
}

pub async fn execute_plugin_tool(
    State(state): State<AppState>,
    axum::extract::Path((name, tool)): axum::extract::Path<(String, String)>,
    Json(body): Json<Value>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let found = state.plugins.find_tool(&tool).await;
    match found {
        Some((plugin_name, _)) if plugin_name == name => {
            let call = ToolCallRequest {
                tool_name: tool.clone(),
                params: body.clone(),
                risk_level: RiskLevel::Caution,
            };
            let ctx = PolicyContext {
                authority: InputAuthority::External,
                survival_tier: SurvivalTier::Normal,
            };
            let decision = state.policy_engine.evaluate_all(&call, &ctx);
            if !decision.is_allowed() {
                let reason = match &decision {
                    PolicyDecision::Deny { reason, .. } => reason.clone(),
                    _ => "policy denied".into(),
                };
                return Err((StatusCode::FORBIDDEN, reason));
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
        Some((other_plugin, _)) => Err((
            StatusCode::BAD_REQUEST,
            format!("tool '{tool}' belongs to plugin '{other_plugin}', not '{name}'"),
        )),
        None => Err((
            StatusCode::NOT_FOUND,
            format!("tool '{tool}' not found in plugin '{name}'"),
        )),
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
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
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
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
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
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state
        .registry
        .start_agent(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let event = json!({"type": "agent_started", "agent_id": id});
    state.event_bus.publish(event.to_string());
    Ok(Json(json!({"id": id, "action": "started"})))
}

pub async fn stop_agent(
    State(state): State<AppState>,
    axum::extract::Path(id): axum::extract::Path<String>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    state
        .registry
        .stop_agent(&id)
        .await
        .map_err(|e| (StatusCode::NOT_FOUND, e.to_string()))?;
    let event = json!({"type": "agent_stopped", "agent_id": id});
    state.event_bus.publish(event.to_string());
    Ok(Json(json!({"id": id, "action": "stopped"})))
}

const WORKSPACE_PALETTE: &[&str] = &[
    "#6366f1", "#22c55e", "#f59e0b", "#ef4444", "#8b5cf6", "#06b6d4",
    "#ec4899", "#14b8a6", "#f97316", "#84cc16", "#a855f7", "#0ea5e9",
];

pub async fn workspace_state(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.registry.list_agents().await;
    let config = state.config.read().await;

    let skills = ironclad_db::skills::list_skills(&state.db).unwrap_or_default();
    let workstations: Vec<Value> = skills
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let count = skills.len().max(1) as f64;
            let angle = (i as f64 / count) * std::f64::consts::TAU;
            let x = 0.5 + 0.35 * angle.cos();
            let y = 0.5 + 0.35 * angle.sin();
            json!({
                "id": s.id,
                "name": s.name,
                "kind": s.kind,
                "x": (x * 1000.0).round() / 1000.0,
                "y": (y * 1000.0).round() / 1000.0,
            })
        })
        .collect();

    let agent_list: Vec<Value> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let color = WORKSPACE_PALETTE[i % WORKSPACE_PALETTE.len()];
            json!({
                "id": a.id,
                "name": a.name,
                "role": if i == 0 { "agent" } else { "specialist" },
                "state": a.state,
                "color": color,
                "model": a.model,
                "current_workstation": null,
                "subordinates": [],
                "supervisor": if i > 0 { agents.first().map(|a| &a.id) } else { None::<&String> },
            })
        })
        .collect();

    let main_agent = json!({
        "id": config.agent.id,
        "name": config.agent.name,
        "role": "agent",
        "state": "Running",
        "color": WORKSPACE_PALETTE[0],
        "model": config.models.primary,
        "current_workstation": null,
        "subordinates": agent_list.iter()
            .filter(|a| a["role"] == "specialist")
            .map(|a| a["id"].clone())
            .collect::<Vec<_>>(),
        "supervisor": null,
    });

    let mut all_agents = vec![main_agent];
    all_agents.extend(agent_list);

    Json(json!({
        "agents": all_agents,
        "workstations": workstations,
        "interactions": [],
    }))
}

pub async fn agent_card(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let card = serde_json::json!({
        "@context": "https://schema.org",
        "@type": "Agent",
        "name": config.agent.name,
        "identifier": config.agent.id,
        "url": format!("http://{}:{}", config.server.bind, config.server.port),
        "capabilities": ["chat", "a2a"],
        "version": env!("CARGO_PKG_VERSION"),
    });
    axum::Json(card)
}

pub async fn a2a_hello(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<A2aHelloRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let peer_did = ironclad_channels::a2a::A2aProtocol::verify_hello(&body.hello)
        .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;

    let mut a2a = state.a2a.write().await;
    a2a.check_rate_limit(&peer_did)
        .map_err(|e| (StatusCode::TOO_MANY_REQUESTS, e.to_string()))?;
    drop(a2a);

    let config = state.config.read().await;
    let our_did = format!("did:ironclad:{}", config.agent.id);
    drop(config);

    let nonce = uuid::Uuid::new_v4();
    let our_hello = ironclad_channels::a2a::A2aProtocol::generate_hello(&our_did, nonce.as_bytes());

    Ok(axum::Json(json!({
        "protocol": "a2a",
        "version": "0.1",
        "status": "ok",
        "peer_did": peer_did,
        "hello": our_hello,
    })))
}
