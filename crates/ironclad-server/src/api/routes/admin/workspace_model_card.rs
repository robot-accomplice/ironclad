#[derive(Deserialize)]
pub struct ChangeModelRequest {
    pub model: String,
    #[serde(default)]
    pub fallbacks: Option<Vec<String>>,
}

pub async fn change_agent_model(
    State(state): State<AppState>,
    Path(agent_name): Path<String>,
    axum::Json(body): axum::Json<ChangeModelRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let model = body.model.trim().to_string();
    if model.is_empty() {
        return Err(bad_request("model cannot be empty"));
    }
    let normalize_fallbacks = |primary: &str, candidates: Vec<String>| -> Vec<String> {
        let mut cleaned = Vec::new();
        for cand in candidates {
            let item = cand.trim();
            if item.is_empty() || item == primary {
                continue;
            }
            if !cleaned.iter().any(|existing: &String| existing == item) {
                cleaned.push(item.to_string());
            }
        }
        cleaned
    };

    let config = state.config.read().await;
    let is_orchestrator = agent_name == config.agent.name || agent_name == config.agent.id;
    let old_model;
    drop(config);

    if is_orchestrator {
        let mut config = state.config.write().await;
        old_model = config.models.primary.clone();
        let old_fallbacks = config.models.fallbacks.clone();
        config.models.primary = model.clone();
        config.models.fallbacks = if let Some(requested) = body.fallbacks.clone() {
            normalize_fallbacks(&model, requested)
        } else {
            // Preserve previous ordering semantics by demoting the old primary to first fallback.
            let mut reordered = vec![old_model.clone()];
            reordered.extend(old_fallbacks);
            normalize_fallbacks(&model, reordered)
        };
        let models = config.models.clone();
        drop(config);

        // Synchronize active router immediately for orchestrator model changes.
        {
            let mut llm = state.llm.write().await;
            llm.router.sync_runtime(
                models.primary.clone(),
                models.fallbacks.clone(),
                models.routing.clone(),
            );
        }

        // BUG-026: Persist model change to disk so it survives server restarts.
        let mut persisted = false;
        {
            let config = state.config.read().await;
            let config_path = state.config_path.as_ref().clone();
            match crate::config_runtime::write_config_atomic(
                std::path::Path::new(&config_path),
                &config,
            ) {
                Ok(()) => persisted = true,
                Err(e) => {
                    tracing::warn!("model change applied in-memory but failed to persist: {e}");
                }
            }
        }

        Ok(axum::Json(json!({
            "updated": true,
            "persisted": persisted,
            "agent": agent_name,
            "old_model": old_model,
            "new_model": model,
            "fallbacks": models.fallbacks,
            "model_order": std::iter::once(models.primary.clone())
                .chain(models.fallbacks.clone())
                .collect::<Vec<_>>(),
        })))
    } else {
        let agents =
            ironclad_db::agents::list_sub_agents(&state.db).map_err(|e| internal_err(&e))?;
        let existing = agents
            .iter()
            .find(|a| a.name == agent_name)
            .ok_or_else(|| {
                (
                    StatusCode::NOT_FOUND,
                    format!("agent '{agent_name}' not found"),
                )
            })?;
        crate::api::routes::subagents::validate_subagent_model_for_role(&existing.role, &model)?;
        old_model = existing.model.clone();
        let mut updated = existing.clone();
        updated.model = model.clone();
        if let Some(requested) = body.fallbacks
            && !requested.is_empty()
        {
            return Err(bad_request(
                "fallback order can only be changed for the orchestrator via this endpoint",
            ));
        }
        ironclad_db::agents::upsert_sub_agent(&state.db, &updated).map_err(|e| internal_err(&e))?;
        Ok(axum::Json(json!({
            "updated": true,
            "agent": agent_name,
            "old_model": old_model,
            "new_model": model,
            "fallback_models": crate::api::routes::subagents::parse_fallback_models_json(
                updated.fallback_models_json.as_deref()
            ),
            "scope": "subagent (persisted to database)",
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

