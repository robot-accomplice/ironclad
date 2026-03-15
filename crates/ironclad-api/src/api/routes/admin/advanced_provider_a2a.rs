pub async fn a2a_hello(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<A2aHelloRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let peer_did =
        ironclad_channels::a2a::A2aProtocol::verify_hello(&body.hello).map_err(bad_request)?;

    let mut a2a = state.a2a.write().await;
    a2a.check_rate_limit(&peer_did)
        .map_err(|e| JsonError(StatusCode::TOO_MANY_REQUESTS, e.to_string()))?;
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

// ── Keystore / provider key management ───────────────────────

#[derive(Deserialize)]
pub struct SetProviderKeyRequest {
    pub api_key: String,
}

pub async fn set_provider_key(
    State(state): State<AppState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<SetProviderKeyRequest>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let key = body.api_key.trim();
    if key.is_empty() {
        return Err(bad_request("api_key cannot be empty"));
    }

    let config = state.config.read().await;
    if !config.providers.contains_key(&name) {
        return Err(not_found(format!("provider '{name}' not found in config")));
    }
    drop(config);

    let ks_name = format!("{name}_api_key");
    state.keystore.set(&ks_name, key).map_err(|e| {
        tracing::error!(provider = %name, error = %e, "failed to store API key in keystore");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".to_string(),
        )
    })?;

    tracing::info!(provider = %name, keystore_entry = %ks_name, "API key stored in keystore via dashboard");

    Ok(axum::Json(json!({
        "stored": true,
        "provider": name,
        "keystore_entry": ks_name,
    })))
}

pub async fn delete_provider_key(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let config = state.config.read().await;
    if !config.providers.contains_key(&name) {
        return Err(not_found(format!("provider '{name}' not found in config")));
    }
    drop(config);

    let ks_name = format!("{name}_api_key");
    let removed = state.keystore.remove(&ks_name).map_err(|e| {
        tracing::error!(provider = %name, error = %e, "failed to remove API key from keystore");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".to_string(),
        )
    })?;

    if removed {
        tracing::info!(provider = %name, keystore_entry = %ks_name, "API key removed from keystore via dashboard");
    }

    Ok(axum::Json(json!({
        "removed": removed,
        "provider": name,
        "keystore_entry": ks_name,
    })))
}

