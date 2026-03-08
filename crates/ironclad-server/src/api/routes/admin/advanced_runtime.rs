
#[derive(Deserialize)]
pub struct RegisterDiscoveredAgentRequest {
    pub agent_id: String,
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
}

#[derive(Deserialize)]
pub struct PairDeviceRequest {
    pub device_id: String,
    pub public_key_hex: String,
    pub device_name: String,
}

pub async fn get_runtime_surfaces(State(state): State<AppState>) -> impl IntoResponse {
    let discovery = state.discovery.read().await;
    let devices = state.devices.read().await;
    let mcp_clients = state.mcp_clients.read().await;
    let mcp_server = state.mcp_server.read().await;
    Json(json!({
        "discovery": {
            "count": discovery.count(),
            "verified_count": discovery.verified_agents().len(),
        },
        "devices": {
            "device_id": devices.identity().device_id,
            "fingerprint": devices.identity().fingerprint(),
            "paired_count": devices.paired_count(),
            "trusted_count": devices.trusted_devices().len(),
        },
        "mcp": {
            "server_enabled": true,
            "tools_exposed": mcp_server.tool_count(),
            "resources_exposed": mcp_server.resource_count(),
            "client_total": mcp_clients.total_count(),
            "client_connected": mcp_clients.connected_count(),
        }
    }))
}

pub async fn list_discovered_agents(State(state): State<AppState>) -> impl IntoResponse {
    let discovery = state.discovery.read().await;
    let agents: Vec<_> = discovery
        .all_agents()
        .iter()
        .map(|a| {
            json!({
                "agent_id": a.agent_id,
                "name": a.name,
                "url": a.url,
                "capabilities": a.capabilities,
                "verified": a.verified,
                "discovery_method": format!("{}", a.discovery_method),
                "last_seen": a.last_seen,
            })
        })
        .collect();
    Json(json!({ "agents": agents, "count": agents.len() }))
}

pub async fn register_discovered_agent(
    State(state): State<AppState>,
    Json(body): Json<RegisterDiscoveredAgentRequest>,
) -> Result<impl IntoResponse, JsonError> {
    validate_short("agent_id", &body.agent_id)?;
    validate_short("name", &body.name)?;
    validate_short("url", &body.url)?;
    let body = RegisterDiscoveredAgentRequest {
        name: sanitize_html(&body.name),
        ..body
    };
    let mut discovery = state.discovery.write().await;
    discovery.register(ironclad_agent::discovery::DiscoveredAgent {
        agent_id: body.agent_id.clone(),
        name: body.name,
        url: body.url,
        capabilities: body.capabilities,
        verified: false,
        discovered_at: chrono::Utc::now(),
        last_seen: chrono::Utc::now(),
        discovery_method: ironclad_agent::discovery::DiscoveryMethod::Manual,
    });
    Ok(Json(json!({ "ok": true, "agent_id": body.agent_id })))
}

pub async fn verify_discovered_agent(
    State(state): State<AppState>,
    Path(agent_id): Path<String>,
) -> impl IntoResponse {
    let mut discovery = state.discovery.write().await;
    match discovery.verify(&agent_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({ "ok": true, "agent_id": agent_id })),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({ "ok": false, "error": e.to_string() })),
        )
            .into_response(),
    }
}

pub async fn list_paired_devices(State(state): State<AppState>) -> impl IntoResponse {
    let devices = state.devices.read().await;
    let device_list: Vec<_> = devices
        .all_devices()
        .iter()
        .map(|d| {
            json!({
                "device_id": d.device_id,
                "device_name": d.device_name,
                "state": format!("{:?}", d.state).to_lowercase(),
                "paired_at": d.paired_at,
                "last_seen": d.last_seen,
            })
        })
        .collect();
    Json(json!({
        "identity": {
            "device_id": devices.identity().device_id,
            "public_key_hex": devices.identity().public_key_hex,
            "fingerprint": devices.identity().fingerprint(),
        },
        "devices": device_list,
    }))
}

pub async fn pair_device(
    State(state): State<AppState>,
    Json(body): Json<PairDeviceRequest>,
) -> impl IntoResponse {
    let mut devices = state.devices.write().await;
    match devices.initiate_pairing(&body.device_id, &body.public_key_hex, &body.device_name) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok": true, "device_id": body.device_id})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn verify_paired_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    let mut devices = state.devices.write().await;
    match devices.verify_pairing(&device_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok": true, "device_id": device_id})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn unpair_device(
    State(state): State<AppState>,
    Path(device_id): Path<String>,
) -> impl IntoResponse {
    let mut devices = state.devices.write().await;
    match devices.unpair(&device_id) {
        Ok(()) => (
            StatusCode::OK,
            Json(json!({"ok": true, "device_id": device_id})),
        )
            .into_response(),
        Err(e) => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": e.to_string()})),
        )
            .into_response(),
    }
}

pub async fn get_mcp_runtime(State(state): State<AppState>) -> impl IntoResponse {
    let clients = state.mcp_clients.read().await;
    let server = state.mcp_server.read().await;
    let connections: Vec<_> = clients
        .list_connections()
        .iter()
        .map(|c| {
            json!({
                "name": c.name,
                "url": c.url,
                "connected": c.connected,
                "tools": c.available_tools.len(),
                "resources": c.available_resources.len(),
            })
        })
        .collect();
    let tools: Vec<_> = server
        .list_tools()
        .iter()
        .map(|t| json!({"name": t.name, "description": t.description}))
        .collect();
    let resources: Vec<_> = server
        .list_resources()
        .iter()
        .map(|r| json!({"uri": r.uri, "name": r.name}))
        .collect();
    Json(json!({
        "connections": connections,
        "connected_count": clients.connected_count(),
        "exposed_tools": tools,
        "exposed_resources": resources,
    }))
}

pub async fn mcp_client_discover(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mut clients = state.mcp_clients.write().await;
    match clients.get_connection_mut(&name) {
        Some(conn) => match conn.discover() {
            Ok(()) => (StatusCode::OK, Json(json!({"ok": true, "name": name}))).into_response(),
            Err(e) => (
                StatusCode::BAD_GATEWAY,
                Json(json!({"ok": false, "error": e.to_string()})),
            )
                .into_response(),
        },
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": "mcp client not found"})),
        )
            .into_response(),
    }
}

pub async fn mcp_client_disconnect(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let mut clients = state.mcp_clients.write().await;
    match clients.get_connection_mut(&name) {
        Some(conn) => {
            conn.disconnect();
            (StatusCode::OK, Json(json!({"ok": true, "name": name}))).into_response()
        }
        None => (
            StatusCode::NOT_FOUND,
            Json(json!({"ok": false, "error": "mcp client not found"})),
        )
            .into_response(),
    }
}

// ── WebSocket ticket issuance ─────────────────────────────────

pub async fn issue_ws_ticket(State(state): State<AppState>) -> impl IntoResponse {
    let ticket = state.ws_tickets.issue();
    Json(json!({ "ticket": ticket, "expires_in": 30 }))
}

/// GET /api/models/routing-diagnostics
///
/// Returns a comprehensive snapshot of routing state for operator diagnostics:
/// - Model profiles with metascores at current complexity
/// - Circuit breaker states
/// - Shadow prediction agreement summary
/// - Active routing config (accuracy floor, cost weight, canary, blocklist)
pub async fn get_routing_diagnostics(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let routing_config = &config.models.routing;
    let cost_aware = routing_config.cost_aware;
    let cost_weight = routing_config.cost_weight;
    let accuracy_floor = routing_config.accuracy_floor;
    let accuracy_min_obs = routing_config.accuracy_min_obs;
    let canary_model = routing_config.canary_model.clone();
    let canary_fraction = routing_config.canary_fraction;
    let blocked_models = routing_config.blocked_models.clone();
    let routing_mode = routing_config.mode.clone();
    drop(config);

    let llm_read = state.llm.read().await;

    // Build profiles for all configured models.
    let profiles = ironclad_llm::build_model_profiles(
        &llm_read.router,
        &llm_read.providers,
        &llm_read.quality,
        &llm_read.capacity,
        &llm_read.breakers,
    );

    // Trace-backed confidence inputs from executed turns (selected model -> observed quality).
    let trace_quality_by_model: HashMap<String, (i64, Option<f64>)> = {
        let conn = state.db.conn();
        let mut map = HashMap::new();
        if let Ok(mut stmt) = conn.prepare(
            "SELECT mse.selected_model, COUNT(*) AS obs, AVG(ic.quality_score) AS avg_quality
             FROM model_selection_events mse
             INNER JOIN inference_costs ic ON ic.turn_id = mse.turn_id
             WHERE ic.quality_score IS NOT NULL
             GROUP BY mse.selected_model",
        ) && let Ok(rows) = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, i64>(1)?,
                r.get::<_, Option<f64>>(2)?,
            ))
        }) {
            for (model, obs, avg_quality) in rows.flatten() {
                map.insert(model, (obs, avg_quality));
            }
        }
        map
    };

    // Compute metascores at a representative complexity (0.5 = medium).
    let profile_diagnostics: Vec<Value> = profiles
        .iter()
        .map(|p| {
            let breakdown = p.metascore_with_cost_weight(0.5, cost_aware, cost_weight);
            let (trace_obs, trace_avg_quality) = trace_quality_by_model
                .get(&p.model_name)
                .copied()
                .unwrap_or((0, None));
            let confidence_trace_backed =
                trace_obs >= accuracy_min_obs as i64 && trace_avg_quality.is_some();
            let confidence = if confidence_trace_backed {
                let observed = trace_avg_quality.unwrap_or(0.0).clamp(0.0, 1.0);
                Some(((breakdown.confidence + observed) / 2.0).clamp(0.0, 1.0))
            } else {
                None
            };
            json!({
                "model": p.model_name,
                "is_local": p.is_local,
                "tier": format!("{:?}", p.tier),
                "cost_per_1k_tokens": (p.cost_per_input_token + p.cost_per_output_token) * 1000.0,
                "estimated_quality": p.estimated_quality,
                "observation_count": p.observation_count,
                "availability": p.availability,
                "capacity_headroom": p.capacity_headroom,
                "metascore": {
                    "final_score": breakdown.final_score,
                    "efficacy": breakdown.efficacy,
                    "cost": breakdown.cost,
                    "availability": breakdown.availability,
                    "locality": breakdown.locality,
                    "confidence": confidence,
                    "confidence_raw": breakdown.confidence,
                    "confidence_trace_backed": confidence_trace_backed,
                    "confidence_trace_observations": trace_obs,
                    "confidence_trace_avg_quality": trace_avg_quality,
                },
                "blocked_by_config": blocked_models.contains(&p.model_name),
            })
        })
        .collect();

    // Circuit breaker states.
    let breaker_states: Vec<Value> = llm_read
        .breakers
        .list_providers()
        .into_iter()
        .map(|(name, state)| {
            json!({
                "provider": name,
                "state": format!("{state:?}"),
                "credit_tripped": llm_read.breakers.is_credit_tripped(&name),
                "operator_forced_open": llm_read.breakers.is_operator_forced_open(&name),
            })
        })
        .collect();

    // Shadow prediction summary (if any data exists).
    let shadow_summary =
        ironclad_db::shadow_routing::shadow_agreement_summary(&state.db, None)
            .inspect_err(|e| tracing::warn!(error = %e, "failed to load shadow agreement summary"))
            .ok();

    let shadow_json = shadow_summary.map(|s| {
        json!({
            "total": s.total,
            "agreed": s.agreed,
            "disagreed": s.disagreed,
            "agreement_rate": s.agreement_rate,
        })
    });

    Json(json!({
        "routing_mode": routing_mode,
        "config": {
            "cost_aware": cost_aware,
            "cost_weight": cost_weight,
            "accuracy_floor": accuracy_floor,
            "accuracy_min_obs": accuracy_min_obs,
            "canary_model": canary_model,
            "canary_fraction": canary_fraction,
            "blocked_models": blocked_models,
        },
        "profiles": profile_diagnostics,
        "circuit_breakers": breaker_states,
        "shadow_predictions": shadow_json,
    }))
}

