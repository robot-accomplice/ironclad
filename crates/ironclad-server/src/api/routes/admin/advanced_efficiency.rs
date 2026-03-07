// ── Prompt efficiency metrics ────────────────────────────────

#[derive(Deserialize)]
pub struct EfficiencyParams {
    pub period: Option<String>,
    pub model: Option<String>,
}

fn efficiency_window(period: &str) -> Option<String> {
    match period {
        "24h" => Some("-24 hours".to_string()),
        "7d" => Some("-7 days".to_string()),
        "30d" => Some("-30 days".to_string()),
        "all" => None,
        _ => Some("-7 days".to_string()),
    }
}

fn extract_delegated_subagent(output: Option<&str>) -> String {
    let Some(out) = output else {
        return "(none)".to_string();
    };
    let marker = "delegated_subagent=";
    let Some(idx) = out.find(marker) else {
        return "(none)".to_string();
    };
    let start = idx + marker.len();
    let tail = &out[start..];
    tail.split_whitespace()
        .next()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or("(none)")
        .to_string()
}

fn compute_subagent_assignment_efficacy(
    db: &ironclad_db::Database,
    period: &str,
) -> Result<serde_json::Value, IroncladError> {
    let window = efficiency_window(period);
    let rows: Vec<(String, String, Option<i64>, Option<String>)> = {
        let conn = db.conn();
        let mut rows: Vec<(String, String, Option<i64>, Option<String>)> = Vec::new();

        if let Some(w) = window.as_deref() {
            let mut stmt = conn
                .prepare(
                    "SELECT tool_name, status, duration_ms, output FROM tool_calls
                     WHERE tool_name IN ('assign-tasks','delegate-subagent','orchestrate-subagents')
                       AND created_at >= datetime('now', ?1)",
                )
                .map_err(|e| IroncladError::Database(e.to_string()))?;
            let mapped = stmt
                .query_map([w], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .map_err(|e| IroncladError::Database(e.to_string()))?;
            rows.extend(mapped.filter_map(std::result::Result::ok));
        } else {
            let mut stmt = conn
                .prepare(
                    "SELECT tool_name, status, duration_ms, output FROM tool_calls
                     WHERE tool_name IN ('assign-tasks','delegate-subagent','orchestrate-subagents')",
                )
                .map_err(|e| IroncladError::Database(e.to_string()))?;
            let mapped = stmt
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })
                .map_err(|e| IroncladError::Database(e.to_string()))?;
            rows.extend(mapped.filter_map(std::result::Result::ok));
        }
        rows
    };

    #[derive(Default)]
    struct Stats {
        total: i64,
        success: i64,
        failed: i64,
        timeout_like: i64,
        durations: Vec<i64>,
    }

    let mut by_subagent: std::collections::HashMap<String, Stats> =
        std::collections::HashMap::new();
    let mut overall = Stats::default();
    for (_tool, status, duration_ms, output) in rows {
        let subagent = extract_delegated_subagent(output.as_deref());
        let entry = by_subagent.entry(subagent).or_default();
        entry.total += 1;
        overall.total += 1;
        if status.eq_ignore_ascii_case("success") {
            entry.success += 1;
            overall.success += 1;
        } else {
            entry.failed += 1;
            overall.failed += 1;
            let lowered = output.as_deref().unwrap_or_default().to_ascii_lowercase();
            if lowered.contains("timeout") {
                entry.timeout_like += 1;
                overall.timeout_like += 1;
            }
        }
        if let Some(ms) = duration_ms {
            entry.durations.push(ms);
            overall.durations.push(ms);
        }
    }

    let mut assignment = serde_json::Map::new();
    let agents = ironclad_db::agents::list_sub_agents(db)?;
    for sa in agents {
        if sa.role.eq_ignore_ascii_case("model-proxy") {
            continue;
        }
        assignment.insert(
            sa.name.clone(),
            json!({
                "configured_model": sa.model,
                "fallback_models": crate::api::routes::subagents::parse_fallback_models_json(sa.fallback_models_json.as_deref()),
                "model_mode": match sa.model.trim().to_ascii_lowercase().as_str() {
                    "auto" => "auto",
                    "orchestrator" => "orchestrator",
                    _ => "fixed",
                },
            }),
        );
    }

    let stats_json = |s: &mut Stats| {
        s.durations.sort_unstable();
        let avg_ms = if s.durations.is_empty() {
            0.0
        } else {
            s.durations.iter().sum::<i64>() as f64 / s.durations.len() as f64
        };
        let p95_ms = if s.durations.is_empty() {
            0.0
        } else {
            let idx = ((s.durations.len() as f64) * 0.95).floor() as usize;
            s.durations[idx.min(s.durations.len() - 1)] as f64
        };
        let success_rate = if s.total > 0 {
            s.success as f64 / s.total as f64
        } else {
            0.0
        };
        json!({
            "total": s.total,
            "success": s.success,
            "failed": s.failed,
            "timeout_like": s.timeout_like,
            "success_rate": success_rate,
            "avg_duration_ms": avg_ms,
            "p95_duration_ms": p95_ms,
        })
    };

    let mut by_subagent_json = serde_json::Map::new();
    for (name, mut stats) in by_subagent {
        by_subagent_json.insert(name, stats_json(&mut stats));
    }

    Ok(json!({
        "period": period,
        "overall": stats_json(&mut overall),
        "by_subagent": by_subagent_json,
        "assignments": assignment,
    }))
}

pub async fn get_efficiency(
    State(state): State<AppState>,
    Query(params): Query<EfficiencyParams>,
) -> impl IntoResponse {
    let period = params.period.as_deref().unwrap_or("7d");
    let model = params.model.as_deref();

    match ironclad_db::efficiency::compute_efficiency(&state.db, period, model) {
        Ok(report) => match serde_json::to_value(report) {
            Ok(mut v) => {
                if let Some(obj) = v.as_object_mut() {
                    match compute_subagent_assignment_efficacy(&state.db, period) {
                        Ok(metrics) => {
                            obj.insert("subagent_assignment".to_string(), metrics);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, "failed to compute subagent assignment efficacy");
                        }
                    }
                }
                Json(v).into_response()
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to serialize efficiency report");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "failed to serialize report",
                )
                    .into_response()
            }
        },
        Err(e) => {
            tracing::error!(error = %e, "failed to compute efficiency report");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            )
                .into_response()
        }
    }
}

// ── Behavioral Recommendations ───────────────────────────────

#[derive(Deserialize)]
pub struct RecommendationsParams {
    pub period: Option<String>,
}

pub async fn get_recommendations(
    State(state): State<AppState>,
    Query(params): Query<RecommendationsParams>,
) -> impl IntoResponse {
    let period = params.period.as_deref().unwrap_or("30d");

    let profile = match ironclad_db::efficiency::build_user_profile(&state.db, period) {
        Ok(p) => p,
        Err(e) => {
            tracing::error!(error = %e, "failed to build user profile for recommendations");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            )
                .into_response();
        }
    };

    let engine = ironclad_agent::recommendations::RecommendationEngine::new();
    let recs = engine.generate(&profile);

    Json(json!({
        "period": period,
        "profile": profile,
        "recommendations": recs,
        "count": recs.len(),
    }))
    .into_response()
}

pub async fn generate_deep_analysis(
    State(state): State<AppState>,
    Query(params): Query<RecommendationsParams>,
) -> impl IntoResponse {
    let period = params.period.as_deref().unwrap_or("30d");

    let profile = match ironclad_db::efficiency::build_user_profile(&state.db, period) {
        Ok(p) => p,
        Err(e) => return Err(internal_err(&e)),
    };

    let engine = ironclad_agent::recommendations::RecommendationEngine::new();
    let recs = engine.generate(&profile);
    let prompt =
        ironclad_agent::recommendations::LlmRecommendationAnalyzer::build_prompt(&profile, &recs);
    let llm = run_llm_recommendation_analysis(&state, &prompt).await?;

    Ok(Json(json!({
        "status": "complete",
        "heuristic_recommendations": recs,
        "deep_analysis": llm["content"],
        "analysis_model": llm["model"],
        "tokens_in": llm["tokens_in"],
        "tokens_out": llm["tokens_out"],
        "cost": llm["cost"],
        "profile": profile,
    })))
}

async fn run_llm_recommendation_analysis(
    state: &AppState,
    prompt: &str,
) -> Result<serde_json::Value, JsonError> {
    let model = {
        let llm = state.llm.read().await;
        llm.router.select_model().to_string()
    };
    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let req = ironclad_llm::format::UnifiedRequest {
        model: model_for_api,
        messages: vec![ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: prompt.to_string(),
            parts: None,
        }],
        max_tokens: Some(2200),
        temperature: Some(0.2),
        system: None,
        quality_target: None,
        tools: vec![],
    };

    let llm = state.llm.read().await;
    let provider = match llm.providers.get_by_model(&model) {
        Some(p) => p.clone(),
        None => {
            return Err(JsonError(
                StatusCode::SERVICE_UNAVAILABLE,
                format!("no provider configured for model {model}"),
            ));
        }
    };
    drop(llm);

    let key = resolve_provider_key(
        &provider.name,
        provider.is_local,
        &provider.auth_mode,
        provider.api_key_ref.as_deref(),
        &provider.api_key_env,
        &state.oauth,
        &state.keystore,
    )
    .await
    .unwrap_or_default();
    if !provider.is_local && key.is_empty() {
        return Err(JsonError(
            StatusCode::SERVICE_UNAVAILABLE,
            format!("missing API key for provider {}", provider.name),
        ));
    }

    let url = format!("{}{}", provider.url, provider.chat_path);
    let body = ironclad_llm::format::translate_request(&req, provider.format).map_err(|e| {
        JsonError(
            StatusCode::BAD_REQUEST,
            format!("failed to translate request: {e}"),
        )
    })?;
    let llm = state.llm.read().await;
    let resp = llm
        .client
        .forward_with_provider(
            &url,
            &key,
            body,
            &provider.auth_header,
            &provider.extra_headers,
        )
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_GATEWAY,
                format!("analysis provider call failed: {e}"),
            )
        })?;
    drop(llm);

    let unified =
        ironclad_llm::format::translate_response(&resp, provider.format).unwrap_or_else(|_| {
            ironclad_llm::format::UnifiedResponse {
                content: "(no response)".into(),
                model: model.clone(),
                tokens_in: 0,
                tokens_out: 0,
                finish_reason: None,
            }
        });
    let tin = unified.tokens_in as i64;
    let tout = unified.tokens_out as i64;
    let cost = (tin.max(0) as f64 * provider.cost_per_input_token)
        + (tout.max(0) as f64 * provider.cost_per_output_token);
    ironclad_db::metrics::record_inference_cost(
        &state.db,
        &model,
        &provider.name,
        tin,
        tout,
        cost,
        Some("recommendations"),
        false,
        None,
        None,
        false,
        None,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record recommendation inference cost"))
    .ok();

    Ok(json!({
        "content": unified.content,
        "model": model,
        "provider": provider.name,
        "tokens_in": tin,
        "tokens_out": tout,
        "cost": cost,
    }))
}
