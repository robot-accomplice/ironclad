//! Config, stats, circuit breaker, wallet, plugins, browser, agents, workspace, A2A.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use ironclad_agent::policy::{PolicyContext, ToolCallRequest};
use ironclad_core::{InputAuthority, IroncladConfig, PolicyDecision, RiskLevel, SurvivalTier};

use super::{AppState, internal_err};

// ── Approval management routes ───────────────────────────────

pub async fn list_approvals(State(state): State<AppState>) -> impl IntoResponse {
    state.approvals.expire_timed_out();
    let pending = state.approvals.list_pending();
    let all = state.approvals.list_all();
    Json(json!({
        "pending": pending,
        "total": all.len(),
    }))
}

#[derive(Deserialize)]
pub struct ApprovalDecisionRequest {
    #[serde(default = "default_decided_by")]
    pub decided_by: String,
}
fn default_decided_by() -> String {
    "api".into()
}

pub async fn approve_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<ApprovalDecisionRequest>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    match state.approvals.approve(&id, &body.decided_by) {
        Ok(req) => Ok(Json(json!(req))),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

pub async fn deny_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<ApprovalDecisionRequest>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    match state.approvals.deny(&id, &body.decided_by) {
        Ok(req) => Ok(Json(json!(req))),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

// ── Audit trail routes ───────────────────────────────────────

pub async fn get_policy_audit(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let decisions = ironclad_db::policy::get_decisions_for_turn(&state.db, &turn_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "turn_id": turn_id,
        "decisions": decisions.iter().map(|d| json!({
            "id": d.id,
            "tool_name": d.tool_name,
            "decision": d.decision,
            "rule_name": d.rule_name,
            "reason": d.reason,
            "created_at": d.created_at,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn get_tool_audit(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &turn_id)
        .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(Json(json!({
        "turn_id": turn_id,
        "tool_calls": calls.iter().map(|c| json!({
            "id": c.id,
            "tool_name": c.tool_name,
            "input": c.input,
            "output": c.output,
            "status": c.status,
            "duration_ms": c.duration_ms,
            "created_at": c.created_at,
        })).collect::<Vec<_>>(),
    })))
}

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
        && let Some(obj) = providers.as_object_mut()
    {
        for (_name, provider) in obj.iter_mut() {
            if let Some(p) = provider.as_object_mut() {
                p.remove("api_key");
                p.remove("secret");
                p.remove("token");
            }
        }
    }
    if let Some(wallet) = cfg.get_mut("wallet")
        && let Some(w) = wallet.as_object_mut()
    {
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
    let mut current = serde_json::to_value(&*config).map_err(|e| internal_err(&e))?;

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
            provider_states.insert(name.clone(), json!({ "state": "closed", "blocked": false }));
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
    let balances = state.wallet.wallet.get_all_balances().await;
    let address = state.wallet.wallet.address();
    let chain_id = state.wallet.wallet.chain_id();
    let network = state.wallet.wallet.network_name();
    let config = state.config.read().await;

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
        },
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
    "#6366f1", "#22c55e", "#f59e0b", "#ef4444", "#8b5cf6", "#06b6d4", "#ec4899", "#14b8a6",
    "#f97316", "#84cc16", "#a855f7", "#0ea5e9",
];

pub async fn workspace_state(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.registry.list_agents().await;
    let config = state.config.read().await;

    let systems: Vec<Value> = vec![
        json!({ "id": "llm",        "name": "LLM Inference",   "kind": "Inference",   "x": 0.18, "y": 0.22 }),
        json!({ "id": "memory",     "name": "Memory",          "kind": "Storage",     "x": 0.82, "y": 0.22 }),
        json!({ "id": "exec",       "name": "Code Execution",  "kind": "Execution",   "x": 0.18, "y": 0.78 }),
        json!({ "id": "blockchain", "name": "Blockchain",      "kind": "Blockchain",  "x": 0.82, "y": 0.78 }),
        json!({ "id": "web",        "name": "Web / APIs",      "kind": "Tool",        "x": 0.50, "y": 0.12 }),
        json!({ "id": "files",      "name": "File System",     "kind": "Tool",        "x": 0.50, "y": 0.88 }),
    ];

    let skills = ironclad_db::skills::list_skills(&state.db).unwrap_or_default();
    let enabled_skills: Vec<String> = skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.clone())
        .collect();

    let agent_list: Vec<Value> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let color = WORKSPACE_PALETTE[(i + 1) % WORKSPACE_PALETTE.len()];
            json!({
                "id": a.id,
                "name": a.name,
                "role": "specialist",
                "state": a.state,
                "color": color,
                "model": a.model,
                "current_workstation": null,
                "active_skill": null,
                "subordinates": [],
                "supervisor": config.agent.id,
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
        "active_skill": null,
        "skills": enabled_skills,
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
        "systems": systems,
        "interactions": [],
    }))
}

pub async fn roster(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let agents_in_registry = state.registry.list_agents().await;

    let workspace = std::path::Path::new(&config.agent.workspace);
    let os = ironclad_core::personality::load_os(workspace);
    let firmware = ironclad_core::personality::load_firmware(workspace);
    let directives = ironclad_core::personality::load_directives(workspace);

    let skills = ironclad_db::skills::list_skills(&state.db).unwrap_or_default();
    let enabled_skills: Vec<&str> = skills.iter().filter(|s| s.enabled).map(|s| s.name.as_str()).collect();
    let skill_kinds: std::collections::HashMap<&str, Vec<&str>> = {
        let mut map: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for s in &skills {
            if s.enabled {
                map.entry(s.kind.as_str()).or_default().push(s.name.as_str());
            }
        }
        map
    };

    let voice = os.as_ref().map(|o| {
        json!({
            "formality": o.voice.formality,
            "proactiveness": o.voice.proactiveness,
            "verbosity": o.voice.verbosity,
            "humor": o.voice.humor,
            "domain": o.voice.domain,
        })
    });

    let missions: Vec<Value> = directives.as_ref().map(|d| {
        d.missions.iter().map(|m| json!({
            "name": m.name,
            "timeframe": m.timeframe,
            "priority": m.priority,
            "description": m.description,
        })).collect()
    }).unwrap_or_default();

    let firmware_rules: Vec<Value> = firmware.as_ref().map(|f| {
        f.rules.iter().map(|r| json!({
            "type": r.rule_type,
            "rule": r.rule,
        })).collect()
    }).unwrap_or_default();

    let running_count = agents_in_registry.iter()
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .count();
    let stats = json!({
        "subordinate_count": agents_in_registry.len(),
        "running_subordinates": running_count,
        "total_skills": skills.len(),
        "enabled_skills": enabled_skills.len(),
    });

    let main_agent = json!({
        "id": config.agent.id,
        "name": config.agent.name,
        "display_name": config.agent.name,
        "role": "commander",
        "model": config.models.primary,
        "enabled": true,
        "color": WORKSPACE_PALETTE[0],
        "session_count": null,
        "description": os.as_ref().and_then(|o| {
            let first_line = o.prompt_text.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("Autonomous agent");
            Some(first_line.to_string())
        }),
        "voice": voice,
        "missions": missions,
        "firmware_rules": firmware_rules,
        "skills": enabled_skills,
        "skill_breakdown": skill_kinds,
        "subordinates": agents_in_registry.iter().map(|a| a.id.clone()).collect::<Vec<_>>(),
        "stats": stats,
    });

    let sub_agents = ironclad_db::agents::list_sub_agents(&state.db).unwrap_or_default();
    let specialist_cards: Vec<Value> = sub_agents.iter().enumerate().map(|(i, sa)| {
        let runtime = agents_in_registry.iter().find(|a| a.id == sa.name);
        let state_str = runtime.map(|r| format!("{:?}", r.state)).unwrap_or_else(|| {
            if sa.enabled { "Idle".into() } else { "Disabled".into() }
        });
        let color = WORKSPACE_PALETTE[(i + 1) % WORKSPACE_PALETTE.len()];
        json!({
            "id": sa.id,
            "name": sa.name,
            "display_name": sa.display_name,
            "role": sa.role,
            "model": sa.model,
            "enabled": sa.enabled,
            "color": color,
            "state": state_str,
            "session_count": sa.session_count,
            "description": sa.description,
            "skills": sa.skills_json.as_ref().and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()).unwrap_or_default(),
            "supervisor": config.agent.id,
        })
    }).collect();

    let mut roster = vec![main_agent];
    roster.extend(specialist_cards);

    Json(json!({ "roster": roster, "count": roster.len() }))
}

#[derive(Deserialize)]
pub struct ChangeModelRequest {
    pub model: String,
}

pub async fn change_agent_model(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
    axum::Json(body): axum::Json<ChangeModelRequest>,
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = body.model.trim().to_string();
    if model.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "model cannot be empty".into()));
    }

    let config = state.config.read().await;
    let is_commander = agent_name == config.agent.name || agent_name == config.agent.id;
    let old_model;
    drop(config);

    if is_commander {
        let mut config = state.config.write().await;
        old_model = config.models.primary.clone();
        config.models.primary = model.clone();
        Ok(axum::Json(json!({
            "updated": true,
            "agent": agent_name,
            "old_model": old_model,
            "new_model": model,
            "scope": "commander (runtime only, not persisted to disk)",
        })))
    } else {
        let agents = ironclad_db::agents::list_sub_agents(&state.db)
            .map_err(|e| internal_err(&e))?;
        let existing = agents.iter().find(|a| a.name == agent_name).ok_or_else(|| {
            (StatusCode::NOT_FOUND, format!("agent '{agent_name}' not found"))
        })?;
        old_model = existing.model.clone();
        let mut updated = existing.clone();
        updated.model = model.clone();
        ironclad_db::agents::upsert_sub_agent(&state.db, &updated)
            .map_err(|e| internal_err(&e))?;
        Ok(axum::Json(json!({
            "updated": true,
            "agent": agent_name,
            "old_model": old_model,
            "new_model": model,
            "scope": "specialist (persisted to database)",
        })))
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_json_flat_replacement() {
        let mut base = json!({"a": 1, "b": 2});
        merge_json(&mut base, &json!({"b": 99}));
        assert_eq!(base["a"], 1);
        assert_eq!(base["b"], 99);
    }

    #[test]
    fn merge_json_adds_new_keys() {
        let mut base = json!({"a": 1});
        merge_json(&mut base, &json!({"b": 2, "c": 3}));
        assert_eq!(base["a"], 1);
        assert_eq!(base["b"], 2);
        assert_eq!(base["c"], 3);
    }

    #[test]
    fn merge_json_deep_nested() {
        let mut base = json!({"outer": {"inner": 1, "keep": true}});
        merge_json(&mut base, &json!({"outer": {"inner": 99, "new": "added"}}));
        assert_eq!(base["outer"]["inner"], 99);
        assert_eq!(base["outer"]["keep"], true);
        assert_eq!(base["outer"]["new"], "added");
    }

    #[test]
    fn merge_json_array_replaces() {
        let mut base = json!({"list": [1, 2, 3]});
        merge_json(&mut base, &json!({"list": [4, 5]}));
        assert_eq!(base["list"], json!([4, 5]));
    }

    #[test]
    fn merge_json_null_replacement() {
        let mut base = json!({"a": 1});
        merge_json(&mut base, &json!({"a": null}));
        assert!(base["a"].is_null());
    }

    #[test]
    fn merge_json_empty_patch_is_noop() {
        let mut base = json!({"a": 1, "b": 2});
        merge_json(&mut base, &json!({}));
        assert_eq!(base, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_json_scalar_to_object() {
        let mut base = json!({"a": "string"});
        merge_json(&mut base, &json!({"a": {"nested": true}}));
        assert_eq!(base["a"]["nested"], true);
    }

    #[test]
    fn merge_json_object_to_scalar() {
        let mut base = json!({"a": {"nested": true}});
        merge_json(&mut base, &json!({"a": 42}));
        assert_eq!(base["a"], 42);
    }

    #[test]
    fn merge_json_three_levels() {
        let mut base = json!({"l1": {"l2": {"l3": "old"}}});
        merge_json(
            &mut base,
            &json!({"l1": {"l2": {"l3": "new", "extra": true}}}),
        );
        assert_eq!(base["l1"]["l2"]["l3"], "new");
        assert_eq!(base["l1"]["l2"]["extra"], true);
    }
}
