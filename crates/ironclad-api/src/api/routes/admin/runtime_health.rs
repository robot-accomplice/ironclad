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

pub async fn get_capacity_stats(State(state): State<AppState>) -> impl IntoResponse {
    let llm = state.llm.read().await;
    let mut providers = serde_json::Map::new();
    for (name, stats) in llm.capacity.list_stats() {
        let sustained_hot = llm.capacity.is_sustained_hot(&name);
        providers.insert(
            name,
            json!({
                "headroom": stats.headroom,
                "near_capacity": stats.near_capacity,
                "sustained_hot": sustained_hot,
                "tokens_used": stats.tokens_used,
                "requests_used": stats.requests_used,
                "tpm_limit": stats.tpm_limit,
                "rpm_limit": stats.rpm_limit,
                "token_utilization": stats.token_utilization,
                "request_utilization": stats.request_utilization,
            }),
        );
    }
    axum::Json(json!({ "providers": Value::Object(providers) }))
}

pub async fn get_throttle_stats(State(state): State<AppState>) -> impl IntoResponse {
    let snapshot = state.rate_limiter.snapshot().await;
    axum::Json(json!(snapshot))
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
                "credit_tripped": llm.breakers.is_credit_tripped(name),
                "operator_forced_open": llm.breakers.is_operator_forced_open(name),
            }),
        );
    }

    for name in config.providers.keys() {
        if !provider_states.contains_key(name) {
            provider_states.insert(
                name.clone(),
                json!({
                    "state": "closed",
                    "blocked": false,
                    "credit_tripped": false,
                    "operator_forced_open": false,
                }),
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
) -> Result<impl IntoResponse, JsonError> {
    let provider_known = {
        let cfg = state.config.read().await;
        cfg.providers.contains_key(&provider)
    };
    if !provider_known {
        return Err(not_found(format!("unknown provider '{provider}'")));
    }

    let mut llm = state.llm.write().await;
    // Always allow reset for configured providers, even if no breaker state exists yet.
    llm.breakers.reset(&provider);
    tracing::warn!(provider = %provider, "operator requested breaker reset");

    Ok(axum::Json(json!({
        "provider": provider,
        "state": "closed",
        "reset": true,
    })))
}

pub async fn breaker_open(
    State(state): State<AppState>,
    Path(provider): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let provider_known = {
        let cfg = state.config.read().await;
        cfg.providers.contains_key(&provider)
    };
    if !provider_known {
        return Err(not_found(format!("unknown provider '{provider}'")));
    }

    let mut llm = state.llm.write().await;
    llm.breakers.force_open(&provider);
    tracing::warn!(provider = %provider, "operator requested breaker force-open");

    Ok(axum::Json(json!({
        "provider": provider,
        "state": "open",
        "blocked": true,
        "operator_forced_open": true,
    })))
}

#[derive(Debug, Deserialize)]
pub struct RoutingDatasetQuery {
    pub since: Option<String>,
    pub until: Option<String>,
    pub schema_version: Option<i64>,
    pub limit: Option<usize>,
    pub format: Option<String>,
    pub include_user_excerpt: Option<bool>,
}

const MAX_DATASET_LIMIT: usize = 50_000;

#[derive(Debug, Deserialize)]
pub struct RoutingEvalRequest {
    pub since: Option<String>,
    pub until: Option<String>,
    pub schema_version: Option<i64>,
    pub limit: Option<usize>,
    pub cost_aware: Option<bool>,
    pub cost_weight: Option<f64>,
    pub accuracy_floor: Option<f64>,
    pub accuracy_min_obs: Option<usize>,
    pub include_verdicts: Option<bool>,
}

fn build_dataset_filter(q: &RoutingDatasetQuery) -> ironclad_db::routing_dataset::DatasetFilter {
    let limit = q.limit.map(|n| n.min(MAX_DATASET_LIMIT));
    ironclad_db::routing_dataset::DatasetFilter {
        since: q.since.clone(),
        until: q.until.clone(),
        schema_version: q.schema_version,
        limit,
    }
}

fn valid_time_filter(value: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(value).is_ok()
        || chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
}

pub async fn get_routing_dataset(
    State(state): State<AppState>,
    Query(q): Query<RoutingDatasetQuery>,
) -> Result<impl IntoResponse, JsonError> {
    if let Some(ref since) = q.since
        && !valid_time_filter(since)
    {
        return Err(bad_request("since must be RFC3339 or YYYY-MM-DD"));
    }
    if let Some(ref until) = q.until
        && !valid_time_filter(until)
    {
        return Err(bad_request("until must be RFC3339 or YYYY-MM-DD"));
    }

    let filter = build_dataset_filter(&q);
    let include_user_excerpt = q.include_user_excerpt.unwrap_or(false);
    if q.format.as_deref() == Some("tsv") {
        if !include_user_excerpt {
            return Err(bad_request(
                "TSV export includes user excerpts; pass include_user_excerpt=true to confirm.",
            ));
        }
        let tsv = ironclad_db::routing_dataset::extract_routing_dataset_tsv(&state.db, &filter)
            .map_err(|e| internal_err(&e))?;
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/tab-separated-values; charset=utf-8",
            )],
            tsv,
        )
            .into_response());
    }

    let mut rows = ironclad_db::routing_dataset::extract_routing_dataset(&state.db, &filter)
        .map_err(|e| internal_err(&e))?;
    if !include_user_excerpt {
        for row in &mut rows {
            row.user_excerpt = "[redacted]".to_string();
        }
    }
    let mut schema_versions: Vec<i64> = rows
        .iter()
        .map(|r| r.schema_version)
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    schema_versions.sort_unstable();
    let summary = ironclad_db::routing_dataset::DatasetSummary {
        total_rows: rows.len(),
        distinct_models: rows
            .iter()
            .map(|r| r.selected_model.clone())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        distinct_strategies: rows
            .iter()
            .map(|r| r.strategy.clone())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        total_cost: rows.iter().map(|r| r.total_cost).sum(),
        avg_cost_per_decision: if rows.is_empty() {
            0.0
        } else {
            rows.iter().map(|r| r.total_cost).sum::<f64>() / rows.len() as f64
        },
        schema_versions,
    };
    Ok(axum::Json(json!({
        "rows": rows,
        "summary": {
            "total_rows": summary.total_rows,
            "distinct_models": summary.distinct_models,
            "distinct_strategies": summary.distinct_strategies,
            "total_cost": summary.total_cost,
            "avg_cost_per_decision": summary.avg_cost_per_decision,
            "schema_versions": summary.schema_versions,
        }
    }))
    .into_response())
}

pub async fn run_routing_eval(
    State(state): State<AppState>,
    Json(req): Json<RoutingEvalRequest>,
) -> Result<impl IntoResponse, JsonError> {
    if let Some(ref since) = req.since
        && !valid_time_filter(since)
    {
        return Err(bad_request("since must be RFC3339 or YYYY-MM-DD"));
    }
    if let Some(ref until) = req.until
        && !valid_time_filter(until)
    {
        return Err(bad_request("until must be RFC3339 or YYYY-MM-DD"));
    }
    if let Some(cost_weight) = req.cost_weight
        && !(0.0..=1.0).contains(&cost_weight)
    {
        return Err(bad_request("cost_weight must be in [0.0, 1.0]"));
    }
    if let Some(accuracy_floor) = req.accuracy_floor
        && !(0.0..=1.0).contains(&accuracy_floor)
    {
        return Err(bad_request("accuracy_floor must be in [0.0, 1.0]"));
    }
    if let Some(min_obs) = req.accuracy_min_obs
        && min_obs < 1
    {
        return Err(bad_request("accuracy_min_obs must be >= 1"));
    }

    let filter = ironclad_db::routing_dataset::DatasetFilter {
        since: req.since.clone(),
        until: req.until.clone(),
        schema_version: req.schema_version,
        limit: req.limit.map(|n| n.min(MAX_DATASET_LIMIT)).or(Some(1000)),
    };
    let rows = ironclad_db::routing_dataset::extract_routing_dataset(&state.db, &filter)
        .map_err(|e| internal_err(&e))?;

    let llm = state.llm.read().await;
    let profiles = ironclad_llm::build_model_profiles(
        &llm.router,
        &llm.providers,
        &llm.quality,
        &llm.capacity,
        &llm.breakers,
    );
    let profile_by_model: std::collections::HashMap<String, ironclad_llm::ModelProfile> = profiles
        .into_iter()
        .map(|p| (p.model_name.clone(), p))
        .collect();
    drop(llm);

    #[derive(Deserialize)]
    struct CandidateWire {
        model: String,
        usable: bool,
    }

    let mut eval_rows = Vec::new();
    for row in rows {
        let complexity = row
            .complexity
            .as_deref()
            .and_then(|s| s.parse::<f64>().ok())
            .unwrap_or(0.5)
            .clamp(0.0, 1.0);
        let wire: Vec<CandidateWire> =
            serde_json::from_str(&row.candidates_json).map_err(|_| {
                bad_request(format!(
                    "routing dataset row {} has malformed candidates_json",
                    row.turn_id
                ))
            })?;
        let mut candidates: Vec<ironclad_llm::ModelProfile> = wire
            .into_iter()
            .filter(|c| c.usable)
            .filter_map(|c| profile_by_model.get(&c.model).cloned())
            .collect();
        if let Some(prod) = profile_by_model.get(&row.selected_model).cloned()
            && !candidates.iter().any(|c| c.model_name == prod.model_name)
        {
            candidates.push(prod);
        }
        if candidates.is_empty() {
            continue;
        }
        eval_rows.push(ironclad_llm::eval_harness::EvalRow {
            turn_id: row.turn_id,
            production_model: row.selected_model,
            complexity,
            candidates,
            observed_cost: row.total_cost,
            observed_quality: row.avg_quality_score,
        });
    }

    let config = ironclad_llm::eval_harness::EvalConfig {
        cost_aware: req.cost_aware.unwrap_or(false),
        cost_weight: req.cost_weight,
        accuracy_floor: req.accuracy_floor.unwrap_or(0.0),
        accuracy_min_obs: req.accuracy_min_obs.unwrap_or(10),
    };
    let verdicts = ironclad_llm::eval_harness::replay(&eval_rows, &config);
    let summary = ironclad_llm::eval_harness::summarize(&verdicts);
    let include_verdicts = req.include_verdicts.unwrap_or(false);

    Ok(axum::Json(json!({
        "rows_considered": eval_rows.len(),
        "summary": summary,
        "config": {
            "cost_aware": config.cost_aware,
            "cost_weight": config.cost_weight,
            "accuracy_floor": config.accuracy_floor,
            "accuracy_min_obs": config.accuracy_min_obs,
        },
        "verdicts": if include_verdicts {
            json!(verdicts)
        } else {
            json!([])
        }
    })))
}

