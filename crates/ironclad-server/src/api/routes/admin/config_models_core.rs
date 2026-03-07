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

const MERGE_JSON_MAX_DEPTH: usize = 10;

fn merge_json(base: &mut Value, patch: &Value) {
    merge_json_inner(base, patch, 0);
}

fn merge_json_inner(base: &mut Value, patch: &Value, depth: usize) {
    if depth > MERGE_JSON_MAX_DEPTH {
        tracing::warn!(
            depth,
            "merge_json exceeded max recursion depth, replacing subtree"
        );
        *base = patch.clone();
        return;
    }
    match (base, patch) {
        (Value::Object(base_map), Value::Object(patch_map)) => {
            for (k, v) in patch_map {
                let entry = base_map.entry(k.clone()).or_insert(Value::Null);
                merge_json_inner(entry, v, depth + 1);
            }
        }
        (base, patch) => {
            *base = patch.clone();
        }
    }
}

pub async fn get_config(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let mut cfg = match serde_json::to_value(&*config) {
        Ok(v) => v,
        Err(e) => {
            tracing::error!(error = %e, "failed to serialize config");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "failed to serialize config",
            )
                .into_response();
        }
    };
    if let Some(providers) = cfg.get_mut("providers")
        && let Some(obj) = providers.as_object_mut()
    {
        for (name, provider) in obj.iter_mut() {
            if let Some(p) = provider.as_object_mut() {
                let is_local = p.get("is_local").and_then(|v| v.as_bool()).unwrap_or(false);
                let api_key_ref = p.get("api_key_ref").and_then(|v| v.as_str());
                let api_key_env = p.get("api_key_env").and_then(|v| v.as_str());
                let auth_mode = p.get("auth_mode").and_then(|v| v.as_str());

                let (key_status, key_source) = check_key_status(
                    name,
                    is_local,
                    api_key_ref,
                    api_key_env,
                    auth_mode,
                    &state.keystore,
                );

                p.insert("_key_status".into(), json!(key_status));
                p.insert("_key_source".into(), json!(key_source));
                p.insert("_provider_name".into(), json!(name.clone()));

                // Allowlist approach: keep only known-safe display fields.
                // New secret fields are safe by default (excluded unless added here).
                const ALLOWED_FIELDS: &[&str] = &[
                    "url",
                    "chat_path",
                    "model",
                    "models",
                    "format",
                    "api_key_ref",
                    "api_key_env",
                    "auth_mode",
                    "auth_header",
                    "is_local",
                    "cost_per_input_token",
                    "cost_per_output_token",
                    "max_tokens",
                    "supports_streaming",
                    "_key_status",
                    "_key_source",
                    "_provider_name",
                ];
                p.retain(|k, _| ALLOWED_FIELDS.contains(&k.as_str()));
            }
        }
    }
    if let Some(wallet) = cfg.get_mut("wallet")
        && let Some(w) = wallet.as_object_mut()
    {
        w.remove("private_key");
        w.remove("mnemonic");
        w.remove("secret");
        w.remove("password");
    }
    axum::Json(cfg).into_response()
}

pub async fn get_config_capabilities() -> impl IntoResponse {
    axum::Json(json!({
        "immutable_sections": ["server", "a2a", "wallet"],
        "mutable_sections": ["agent", "server", "database", "models", "memory", "cache", "treasury", "yield", "wallet", "a2a", "skills", "channels", "circuit_breaker", "providers", "context", "approvals", "plugins", "browser", "daemon", "update", "tier_adapt", "personality", "session", "digest", "multimodal", "knowledge", "workspace_config", "mcp", "devices", "discovery", "obsidian", "security"],
        "notes": {
            "runtime_reload": "all sections are accepted and persisted to ironclad.toml with validation",
            "deferred_apply_examples": ["server.bind", "server.port", "wallet", "treasury.policy_engine", "browser.runtime"],
            "deferred_apply_behavior": "changes marked deferred are persisted immediately but may require restart for full runtime effect"
        }
    }))
}

pub async fn get_config_apply_status(State(state): State<AppState>) -> impl IntoResponse {
    let status = state.config_apply_status.read().await.clone();
    axum::Json(json!({
        "status": status
    }))
}

#[derive(Deserialize, Default)]
pub struct AvailableModelsQuery {
    pub provider: Option<String>,
    pub validation_level: Option<String>,
}

fn model_discovery_mode(
    provider_name: &str,
    provider_url: &str,
    is_local_flag: bool,
) -> (bool, String) {
    let name_l = provider_name.to_ascii_lowercase();
    let url_l = provider_url.to_ascii_lowercase();
    // Only Ollama-style providers should be probed with /api/tags.
    let ollama_like = name_l.contains("ollama") || url_l.contains("11434");
    let keyless_local = is_local_flag || ollama_like;
    let models_url = if ollama_like {
        format!("{provider_url}/api/tags")
    } else {
        format!("{provider_url}/v1/models")
    };
    (keyless_local, models_url)
}

fn apply_provider_auth(
    req: reqwest::RequestBuilder,
    auth_header_name: &str,
    key: &str,
) -> reqwest::RequestBuilder {
    if let Some(param_name) = auth_header_name.strip_prefix("query:") {
        req.query(&[(param_name, key)])
    } else if auth_header_name.eq_ignore_ascii_case("authorization") {
        req.header(auth_header_name, format!("Bearer {key}"))
    } else {
        req.header(auth_header_name, key)
    }
}

