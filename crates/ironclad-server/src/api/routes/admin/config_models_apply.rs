pub async fn get_available_models(
    State(state): State<AppState>,
    Query(query): Query<AvailableModelsQuery>,
) -> impl IntoResponse {
    let provider_filter = query.provider.map(|p| p.to_lowercase());
    let validation_level = query
        .validation_level
        .as_deref()
        .unwrap_or("zero")
        .to_lowercase();
    let (providers, configured_models) = {
        let config = state.config.read().await;
        let mut configured = Vec::new();
        configured.push(config.models.primary.clone());
        configured.extend(config.models.fallbacks.clone());
        (config.providers.clone(), configured)
    };

    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            return Json(json!({
                "models": [],
                "count": 0,
                "providers": {},
                "error": format!("failed to initialize HTTP client: {e}"),
            }));
        }
    };

    let mut all_models = std::collections::BTreeSet::<String>::new();
    let mut provider_reports = serde_json::Map::new();

    for (name, provider_cfg) in providers {
        if let Some(filter) = provider_filter.as_deref()
            && name.to_lowercase() != filter
        {
            continue;
        }

        let url = provider_cfg.url.trim().trim_end_matches('/').to_string();
        if url.is_empty() {
            provider_reports.insert(
                name.clone(),
                json!({
                    "status": "skipped",
                    "reason": "missing_url",
                    "models": [],
                    "count": 0,
                }),
            );
            continue;
        }

        let (localish, models_url) =
            model_discovery_mode(&name, &url, provider_cfg.is_local.unwrap_or(false));

        let auth_mode = provider_cfg.auth_mode.as_deref().unwrap_or("api_key");
        let api_key_env = provider_cfg.api_key_env.as_deref().unwrap_or("");
        let api_key_ref = provider_cfg.api_key_ref.as_deref();
        let api_key = resolve_provider_key(
            &name,
            localish,
            auth_mode,
            api_key_ref,
            api_key_env,
            &state.oauth,
            &state.keystore,
        )
        .await;

        let mut req = client.get(&models_url);
        if let Some(k) = api_key
            && !k.is_empty()
        {
            let auth_header_name = provider_cfg
                .auth_header
                .as_deref()
                .unwrap_or("Authorization")
                .trim();
            req = apply_provider_auth(req, auth_header_name, &k);
        }
        if let Some(extra) = &provider_cfg.extra_headers {
            for (k, v) in extra {
                req = req.header(k, v);
            }
        }

        match req.send().await {
            Ok(resp) if resp.status().is_success() => {
                let body: serde_json::Value = match resp.json().await {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::warn!(provider = %name, error = %e, "failed to parse model-list response JSON");
                        json!({})
                    }
                };
                let has_ollama_shape = body.get("models").and_then(|v| v.as_array()).is_some();
                let has_openai_shape = body.get("data").and_then(|v| v.as_array()).is_some();
                if !has_ollama_shape && !has_openai_shape {
                    provider_reports.insert(
                        name.clone(),
                        json!({
                            "status": "error",
                            "error": "invalid models discovery response",
                            "models": [],
                            "count": 0,
                        }),
                    );
                    continue;
                }
                let mut models: Vec<String> =
                    if let Some(arr) = body.get("models").and_then(|v| v.as_array()) {
                        arr.iter()
                            .filter_map(|m| {
                                m.get("name")
                                    .or_else(|| m.get("model"))
                                    .and_then(|v| v.as_str())
                            })
                            .map(|m| m.to_string())
                            .collect()
                    } else if let Some(arr) = body.get("data").and_then(|v| v.as_array()) {
                        arr.iter()
                            .filter_map(|m| m.get("id").and_then(|v| v.as_str()))
                            .map(|m| m.to_string())
                            .collect()
                    } else {
                        vec![]
                    };

                for model in &mut models {
                    if !model.contains('/') {
                        *model = format!("{name}/{model}");
                    }
                }

                models.sort();
                models.dedup();
                for m in &models {
                    all_models.insert(m.clone());
                }
                provider_reports.insert(
                    name.clone(),
                    json!({
                        "status": "ok",
                        "models": models,
                        "count": models.len(),
                    }),
                );
            }
            Ok(resp) => {
                provider_reports.insert(
                    name.clone(),
                    json!({
                        "status": "error",
                        "error": format!("http {}", resp.status()),
                        "models": [],
                        "count": 0,
                    }),
                );
            }
            Err(e) => {
                let err = e.to_string();
                provider_reports.insert(
                    name.clone(),
                    json!({
                        "status": "unreachable",
                        "error": err,
                        "models": [],
                        "count": 0,
                    }),
                );
            }
        }
    }

    // Always include configured model order entries so UI selectors remain
    // usable even when remote provider model discovery is unavailable.
    for model in configured_models {
        let m = model.trim();
        if m.is_empty() {
            continue;
        }
        if let Some(filter) = provider_filter.as_deref() {
            let provider_prefix = m.split('/').next().unwrap_or_default().to_lowercase();
            if provider_prefix != filter {
                continue;
            }
        }
        all_models.insert(m.to_string());
    }

    let models: Vec<String> = all_models.into_iter().collect();
    Json(json!({
        "models": models,
        "count": models.len(),
        "validation_level": validation_level,
        "proxy": {
            "mode": "in_process",
        },
        "providers": provider_reports,
    }))
}

/// Known top-level config section names (must match `IroncladConfig` fields).
const KNOWN_CONFIG_SECTIONS: &[&str] = &[
    "agent",
    "server",
    "database",
    "models",
    "providers",
    "circuit_breaker",
    "memory",
    "cache",
    "treasury",
    "yield",
    "wallet",
    "a2a",
    "skills",
    "channels",
    "context",
    "approvals",
    "plugins",
    "browser",
    "daemon",
    "update",
    "tier_adapt",
    "personality",
    "session",
    "digest",
    "multimodal",
    "knowledge",
    "workspace_config",
    "mcp",
    "devices",
    "discovery",
    "obsidian",
];

pub async fn update_config(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    {
        let mut status = state.config_apply_status.write().await;
        status.last_attempt_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_error = None;
    }

    // BUG-023: Detect unknown top-level keys in the patch.
    let ignored_keys: Vec<String> = if let Some(obj) = body.patch.as_object() {
        obj.keys()
            .filter(|k| !KNOWN_CONFIG_SECTIONS.contains(&k.as_str()))
            .cloned()
            .collect()
    } else {
        vec![]
    };

    let runtime_cfg = state.config.read().await.clone();
    let mut current = match config_runtime::config_value_from_file_or_runtime(
        state.config_path.as_ref(),
        &runtime_cfg,
    ) {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err(JsonError(StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    };

    // BUG-025: Snapshot pre-merge config for change detection.
    let pre_merge = current.clone();

    merge_json(&mut current, &body.patch);

    // BUG-025: If config is unchanged after merge, skip persistence.
    if current == pre_merge {
        return Ok::<_, JsonError>(axum::Json(json!({
            "updated": false,
            "persisted": false,
            "message": "no effective changes detected",
            "ignored_keys": ignored_keys,
        })));
    }

    let mut updated: IroncladConfig = match serde_json::from_value(current) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "config deserialization failed");
            let msg = "invalid config: schema validation failed".to_string();
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err(bad_request(msg));
        }
    };
    // Keep runtime JSON patch behavior aligned with TOML load path resolution.
    updated.normalize_paths();
    if let Err(e) = updated.validate() {
        tracing::warn!(error = %e, "config validation failed");
        let msg = "invalid config: validation failed".to_string();
        state.config_apply_status.write().await.last_error = Some(msg.clone());
        return Err(bad_request(msg));
    }

    let report = match config_runtime::apply_runtime_config(&state, updated).await {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err(JsonError(StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    };

    {
        let mut status = state.config_apply_status.write().await;
        status.last_success_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_backup_path = report.backup_path.clone();
        status.deferred_apply = report.deferred_apply.clone();
    }

    Ok::<_, JsonError>(axum::Json(json!({
        "updated": true,
        "persisted": true,
        "message": "configuration updated and reloaded from disk-backed state",
        "backup_path": report.backup_path,
        "deferred_apply": report.deferred_apply,
        "ignored_keys": ignored_keys,
    })))
}
