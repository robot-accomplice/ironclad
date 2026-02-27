//! Config, stats, circuit breaker, wallet, plugins, browser, agents, workspace, A2A.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::config_runtime;
use ironclad_agent::policy::{PolicyContext, ToolCallRequest};
use ironclad_core::{
    InputAuthority, IroncladConfig, PolicyDecision, SurvivalTier, input_capability_scan,
};

use super::{AppState, internal_err};

// ── Key resolution helper ────────────────────────────────────

/// Where a provider's API key was found (or that none is needed/available).
pub(crate) enum KeySource {
    NotRequired,
    OAuth,
    Keystore(String),
    EnvVar(String),
    Missing,
}

impl KeySource {
    pub fn status_pair(&self) -> (&'static str, &'static str) {
        match self {
            Self::NotRequired => ("not_required", "local"),
            Self::OAuth => ("configured", "oauth"),
            Self::Keystore(_) => ("configured", "keystore"),
            Self::EnvVar(_) => ("configured", "env"),
            Self::Missing => ("missing", "none"),
        }
    }
}

/// Determine the source and value of a provider's API key using a priority
/// cascade:
///   1. Local provider → `NotRequired`
///   2. OAuth (auth_mode == "oauth") → `OAuth`
///   3. Explicit keystore ref (api_key_ref = "keystore:name")
///   4. Conventional keystore name ({provider_name}_api_key)
///   5. Non-empty environment variable (api_key_env)
///   6. `Missing`
fn resolve_key_source(
    provider_name: &str,
    is_local: bool,
    api_key_ref: Option<&str>,
    api_key_env: Option<&str>,
    auth_mode: Option<&str>,
    keystore: &ironclad_core::keystore::Keystore,
) -> KeySource {
    if is_local {
        return KeySource::NotRequired;
    }

    if auth_mode.is_some_and(|m| m == "oauth") {
        return KeySource::OAuth;
    }

    if let Some(ks_name) = api_key_ref.and_then(|r| r.strip_prefix("keystore:"))
        && let Some(val) = keystore.get(ks_name)
    {
        return KeySource::Keystore(val);
    }

    let conventional = format!("{provider_name}_api_key");
    if let Some(val) = keystore.get(&conventional)
        && !val.is_empty()
    {
        return KeySource::Keystore(val);
    }

    if let Some(env_name) = api_key_env
        && let Ok(val) = std::env::var(env_name)
        && !val.is_empty()
    {
        return KeySource::EnvVar(val);
    }

    KeySource::Missing
}

/// Resolve an API key for a provider. Returns `None` when no key is
/// configured (or when the provider is local and doesn't need one).
pub(crate) async fn resolve_provider_key(
    provider_name: &str,
    is_local: bool,
    auth_mode: &str,
    api_key_ref: Option<&str>,
    api_key_env: &str,
    oauth: &ironclad_llm::OAuthManager,
    keystore: &ironclad_core::keystore::Keystore,
) -> Option<String> {
    let source = resolve_key_source(
        provider_name,
        is_local,
        api_key_ref,
        Some(api_key_env),
        Some(auth_mode),
        keystore,
    );
    match source {
        KeySource::NotRequired | KeySource::Missing => None,
        KeySource::OAuth => oauth.resolve_token(provider_name).await.ok(),
        KeySource::Keystore(v) | KeySource::EnvVar(v) => Some(v),
    }
}

/// Check whether a key is present for a provider, returning (status, source).
pub(crate) fn check_key_status(
    provider_name: &str,
    is_local: bool,
    api_key_ref: Option<&str>,
    api_key_env: Option<&str>,
    auth_mode: Option<&str>,
    keystore: &ironclad_core::keystore::Keystore,
) -> (&'static str, &'static str) {
    resolve_key_source(
        provider_name,
        is_local,
        api_key_ref,
        api_key_env,
        auth_mode,
        keystore,
    )
    .status_pair()
}

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

const MAX_DECIDED_BY_LEN: usize = 256;

#[derive(Deserialize)]
pub struct ApprovalDecisionRequest {
    #[serde(default = "default_decided_by")]
    pub decided_by: String,
}
fn default_decided_by() -> String {
    "api".into()
}

/// Sanitize the `decided_by` field: enforce max length and strip control characters.
fn sanitize_decided_by(raw: &str) -> Result<String, (StatusCode, String)> {
    if raw.len() > MAX_DECIDED_BY_LEN {
        return Err((
            StatusCode::BAD_REQUEST,
            format!("decided_by exceeds max length of {MAX_DECIDED_BY_LEN} characters"),
        ));
    }
    let sanitized: String = raw.chars().filter(|c| !c.is_control()).collect();
    Ok(sanitized)
}

pub async fn approve_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<ApprovalDecisionRequest>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let decided_by = sanitize_decided_by(&body.decided_by)?;
    match state.approvals.approve(&id, &decided_by) {
        Ok(req) => Ok(Json(json!(req))),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

pub async fn deny_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<ApprovalDecisionRequest>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let decided_by = sanitize_decided_by(&body.decided_by)?;
    match state.approvals.deny(&id, &decided_by) {
        Ok(req) => Ok(Json(json!(req))),
        Err(e) => Err((StatusCode::NOT_FOUND, e.to_string())),
    }
}

// ── Audit trail routes ───────────────────────────────────────

pub async fn get_policy_audit(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let decisions =
        ironclad_db::policy::get_decisions_for_turn(&state.db, &turn_id).map_err(|e| {
            tracing::error!(error = %e, "failed to fetch policy audit");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".to_string(),
            )
        })?;
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
    let calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &turn_id).map_err(|e| {
        tracing::error!(error = %e, "failed to fetch tool audit");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".to_string(),
        )
    })?;
    Ok(Json(json!({
        "turn_id": turn_id,
        "tool_calls": calls.iter().map(|c| json!({
            "id": c.id,
            "tool_name": c.tool_name,
            "skill_id": c.skill_id,
            "skill_name": c.skill_name,
            "skill_hash": c.skill_hash,
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
    let mut cfg = serde_json::to_value(&*config).unwrap_or_default();
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

                // Blocklist approach: strip all known secret-bearing fields.
                // WARNING: when adding new provider config fields that contain
                // secrets, you MUST add them here or they will be exposed via
                // the GET /api/config endpoint.
                p.remove("api_key");
                p.remove("api_key_env");
                p.remove("api_key_ref");
                p.remove("secret");
                p.remove("token");
                p.remove("password");
                p.remove("auth_token");
                p.remove("client_secret");
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
    axum::Json(cfg)
}

pub async fn get_config_capabilities() -> impl IntoResponse {
    axum::Json(json!({
        "immutable_sections": [],
        "mutable_sections": ["agent", "server", "database", "models", "memory", "cache", "treasury", "yield", "wallet", "a2a", "skills", "channels", "circuit_breaker", "providers", "context", "approvals", "plugins", "browser", "daemon", "update", "tier_adapt", "personality", "session", "digest", "multimodal", "knowledge", "workspace_config", "mcp", "devices", "discovery", "obsidian"],
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

fn is_loopback_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.contains("127.0.0.1") || lower.contains("localhost")
}

fn legacy_loopback_support_state() -> &'static str {
    "removed_v0_8"
}

fn classify_provider_connectivity_status(
    provider_name: &str,
    provider_url: &str,
    models_url: &str,
    _error: &str,
    localish: bool,
) -> (&'static str, Option<String>) {
    let remote_discovery_target = models_url.contains("/v1/models");
    if !localish && is_loopback_url(provider_url) && remote_discovery_target {
        return (
            "legacy_proxy_unsupported",
            Some(format!(
                "legacy loopback provider URL is unsupported in v0.8.0+: update providers.{provider_name}.url to a direct provider base URL"
            )),
        );
    }
    ("unreachable", None)
}

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
                    Err(_) => json!({}),
                };
                let has_ollama_shape = body.get("models").and_then(|v| v.as_array()).is_some();
                let has_openai_shape = body.get("data").and_then(|v| v.as_array()).is_some();
                if !has_ollama_shape && !has_openai_shape {
                    let status = if !localish && is_loopback_url(&url) {
                        "legacy_proxy_unsupported"
                    } else {
                        "error"
                    };
                    let hint = if status == "legacy_proxy_unsupported" {
                        Some(format!(
                            "legacy loopback provider URL is unsupported in v0.8.0+: update providers.{name}.url to a direct provider base URL"
                        ))
                    } else {
                        None
                    };
                    provider_reports.insert(
                        name.clone(),
                        json!({
                            "status": status,
                            "error": "invalid models discovery response",
                            "hint": hint,
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
                let (status, hint) =
                    classify_provider_connectivity_status(&name, &url, &models_url, &err, localish);
                provider_reports.insert(
                    name.clone(),
                    json!({
                        "status": status,
                        "error": err,
                        "hint": hint,
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
            "loopback_listener_required": false,
            "legacy_loopback_support": legacy_loopback_support_state()
        },
        "providers": provider_reports,
    }))
}

pub async fn update_config(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<UpdateConfigRequest>,
) -> impl IntoResponse {
    {
        let mut status = state.config_apply_status.write().await;
        status.last_attempt_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_error = None;
    }

    let runtime_cfg = state.config.read().await.clone();
    let mut current = match config_runtime::config_value_from_file_or_runtime(
        state.config_path.as_ref(),
        &runtime_cfg,
    ) {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    };

    merge_json(&mut current, &body.patch);
    let updated: IroncladConfig = match serde_json::from_value(current) {
        Ok(v) => v,
        Err(e) => {
            let msg = format!("invalid config: {e}");
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err((StatusCode::BAD_REQUEST, msg));
        }
    };
    if let Err(e) = updated.validate() {
        let msg = format!("validation failed: {e}");
        state.config_apply_status.write().await.last_error = Some(msg.clone());
        return Err((StatusCode::BAD_REQUEST, msg));
    }

    let report = match config_runtime::apply_runtime_config(&state, updated).await {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            state.config_apply_status.write().await.last_error = Some(msg.clone());
            return Err((StatusCode::INTERNAL_SERVER_ERROR, msg));
        }
    };

    {
        let mut status = state.config_apply_status.write().await;
        status.last_success_at = Some(chrono::Utc::now().to_rfc3339());
        status.last_backup_path = report.backup_path.clone();
        status.deferred_apply = report.deferred_apply.clone();
    }

    Ok::<_, (StatusCode, String)>(axum::Json(json!({
        "updated": true,
        "persisted": true,
        "message": "configuration updated and reloaded from disk-backed state",
        "backup_path": report.backup_path,
        "deferred_apply": report.deferred_apply,
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

#[derive(Deserialize)]
pub struct TimeSeriesQuery {
    pub hours: Option<i64>,
}

pub async fn get_overview_timeseries(
    State(state): State<AppState>,
    Query(params): Query<TimeSeriesQuery>,
) -> impl IntoResponse {
    let hours = params.hours.unwrap_or(24).clamp(1, 168) as usize;
    let conn = state.db.conn();
    let now = chrono::Utc::now().naive_utc();
    let mut labels = Vec::with_capacity(hours);
    let mut cost_per_hour = vec![0.0f64; hours];
    let mut tokens_per_hour = vec![0.0f64; hours];
    let mut sessions_per_hour = vec![0i64; hours];
    let mut latency_samples: Vec<Vec<i64>> = (0..hours).map(|_| Vec::new()).collect();
    let mut cron_success = vec![0.0f64; hours];
    let mut cron_total = vec![0i64; hours];
    let mut cron_ok = vec![0i64; hours];

    for i in 0..hours {
        let hr = (now - chrono::Duration::hours((hours - 1 - i) as i64))
            .format("%H:00")
            .to_string();
        labels.push(hr);
    }

    let parse_ts = |s: &str| -> Option<chrono::NaiveDateTime> {
        chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok()
    };
    let bucket_for = |ts: chrono::NaiveDateTime| -> Option<usize> {
        let age = now - ts;
        let mins = age.num_minutes();
        if mins < 0 {
            return None;
        }
        let idx_from_end = (mins / 60) as usize;
        if idx_from_end >= hours {
            None
        } else {
            Some(hours - 1 - idx_from_end)
        }
    };

    if let Ok(mut stmt) = conn.prepare(
        "SELECT cost, tokens_in, tokens_out, created_at FROM inference_costs
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((
                row.get::<_, f64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        }) {
            for (cost, tin, tout, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    cost_per_hour[idx] += cost;
                    tokens_per_hour[idx] += (tin + tout) as f64;
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT created_at FROM sessions
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| row.get::<_, String>(0)) {
            for created_at in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    sessions_per_hour[idx] += 1;
                }
            }
        }
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT duration_ms, created_at FROM tool_calls
         WHERE created_at >= datetime('now', ?1) AND duration_ms IS NOT NULL",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        }) {
            for (dur, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    latency_samples[idx].push(dur);
                }
            }
        }
    }

    let mut latency_p50 = vec![0.0f64; hours];
    for i in 0..hours {
        if latency_samples[i].is_empty() {
            continue;
        }
        latency_samples[i].sort_unstable();
        let mid = latency_samples[i].len() / 2;
        latency_p50[i] = latency_samples[i][mid] as f64;
    }

    if let Ok(mut stmt) = conn.prepare(
        "SELECT status, created_at FROM cron_runs
         WHERE created_at >= datetime('now', ?1)",
    ) {
        let window = format!("-{} hours", hours);
        if let Ok(rows) = stmt.query_map([window], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        }) {
            for (status, created_at) in rows.flatten() {
                if let Some(ts) = parse_ts(&created_at)
                    && let Some(idx) = bucket_for(ts)
                {
                    cron_total[idx] += 1;
                    if status == "success" {
                        cron_ok[idx] += 1;
                    }
                }
            }
        }
    }
    for i in 0..hours {
        cron_success[i] = if cron_total[i] > 0 {
            cron_ok[i] as f64 / cron_total[i] as f64
        } else {
            1.0
        };
    }

    Ok::<_, (StatusCode, String)>(axum::Json(json!({
        "hours": hours,
        "labels": labels,
        "series": {
            "cost_per_hour": cost_per_hour,
            "tokens_per_hour": tokens_per_hour,
            "sessions_per_hour": sessions_per_hour,
            "latency_p50_ms": latency_p50,
            "cron_success_rate": cron_success
        }
    })))
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
                return Err((
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
const ROLE_SUBAGENT: &str = "subagent";
const ROLE_MODEL_PROXY: &str = "model-proxy";

const WORKSPACE_ACTIVITY_WINDOW_SECS: i64 = 120;

fn workspace_files_snapshot(workspace_root: &std::path::Path) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(workspace_root) {
        for entry in read_dir.flatten().take(200) {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() || name.starts_with('.') {
                continue;
            }
            let kind = if path.is_dir() { "dir" } else { "file" };
            entries.push(json!({ "name": name, "kind": kind }));
        }
    }
    entries.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });
    let entry_count = entries.len();
    json!({
        "workspace_root": workspace_root.display().to_string(),
        "top_level_entries": entries,
        "entry_count": entry_count,
    })
}

fn parse_db_timestamp_utc(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::from_naive_utc_and_offset(
            ndt,
            chrono::Utc,
        ));
    }
    None
}

fn is_recent_activity(ts: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    parse_db_timestamp_utc(ts)
        .map(|t| (now - t).num_seconds() <= WORKSPACE_ACTIVITY_WINDOW_SECS)
        .unwrap_or(false)
}

fn has_tool_token(tool_name_lower: &str, token: &str) -> bool {
    tool_name_lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == token)
}

fn workstation_for_tool(tool_name: &str) -> (&'static str, &'static str) {
    let t = tool_name.to_lowercase();
    // Classify local file/search tooling before broad "search" web matching.
    if t.contains("read")
        || t.contains("write")
        || t.contains("file")
        || t.contains("glob")
        || has_tool_token(&t, "rg")
        || t.contains("patch")
        || t.contains("edit")
    {
        return ("files", "tool_execution");
    }
    if t.contains("web") || t.contains("http") || t.contains("fetch") || t.contains("search") {
        return ("web", "tool_execution");
    }
    if t.contains("memory") {
        return ("memory", "working");
    }
    if t.contains("wallet")
        || t.contains("chain")
        || t.contains("block")
        || t.contains("contract")
        || t.contains("token")
    {
        return ("blockchain", "tool_execution");
    }
    ("exec", "tool_execution")
}

fn derive_workspace_activity(
    db: &ironclad_db::Database,
    agent_id: &str,
    running: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> (Option<&'static str>, &'static str, Option<String>) {
    if !running {
        return (Some("standby"), "idle", None);
    }

    let conn = db.conn();

    let latest_tool: Option<(String, String)> = conn
        .query_row(
            "SELECT tc.tool_name, tc.created_at
             FROM tool_calls tc
             INNER JOIN turns t ON t.id = tc.turn_id
             INNER JOIN sessions s ON s.id = t.session_id
             WHERE s.agent_id = ?1
             ORDER BY tc.created_at DESC
             LIMIT 1",
            [agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    if let Some((tool_name, created_at)) = latest_tool
        && is_recent_activity(&created_at, now)
    {
        let (workstation, activity) = workstation_for_tool(&tool_name);
        return (Some(workstation), activity, Some(tool_name));
    }

    let latest_turn_created: Option<String> = conn
        .query_row(
            "SELECT t.created_at
             FROM turns t
             INNER JOIN sessions s ON s.id = t.session_id
             WHERE s.agent_id = ?1
             ORDER BY t.created_at DESC
             LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .ok();

    if let Some(created_at) = latest_turn_created
        && is_recent_activity(&created_at, now)
    {
        return (Some("llm"), "inference", None);
    }

    (Some("standby"), "idle", None)
}

pub async fn workspace_state(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.registry.list_agents().await;
    let config = state.config.read().await;
    let now = chrono::Utc::now();
    let workspace_root = std::path::Path::new(&config.agent.workspace);
    let files = workspace_files_snapshot(workspace_root);

    let systems: Vec<Value> = vec![
        json!({ "id": "llm",        "name": "LLM Inference",   "kind": "Inference",   "x": 0.18, "y": 0.22 }),
        json!({ "id": "memory",     "name": "Memory",          "kind": "Storage",     "x": 0.82, "y": 0.22 }),
        json!({ "id": "exec",       "name": "Code Execution",  "kind": "Execution",   "x": 0.18, "y": 0.78 }),
        json!({ "id": "blockchain", "name": "Blockchain",      "kind": "Blockchain",  "x": 0.82, "y": 0.78 }),
        json!({ "id": "web",        "name": "Web / APIs",      "kind": "Tool",        "x": 0.50, "y": 0.12 }),
        json!({ "id": "files",      "name": "File System",     "kind": "Tool",        "x": 0.50, "y": 0.88 }),
        json!({ "id": "standby",    "name": "Standby Bay",     "kind": "Standby",     "x": 0.08, "y": 0.50 }),
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
            let running = format!("{:?}", a.state).to_lowercase() == "running";
            let (workstation, activity, active_skill) =
                derive_workspace_activity(&state.db, &a.id, running, now);
            json!({
                "id": a.id,
                "name": a.name,
                "role": ROLE_SUBAGENT,
                "state": a.state,
                "color": color,
                "model": a.model,
                "current_workstation": workstation,
                "activity": activity,
                "active_skill": active_skill,
                "updated_at": chrono::Utc::now().to_rfc3339(),
                "subordinates": [],
                "supervisor": config.agent.id,
            })
        })
        .collect();

    let (main_workstation, main_activity, main_active_skill) =
        derive_workspace_activity(&state.db, &config.agent.id, true, now);

    let main_agent = json!({
        "id": config.agent.id,
        "name": config.agent.name,
        "role": "agent",
        "state": "Running",
        "color": WORKSPACE_PALETTE[0],
        "model": config.models.primary,
        "current_workstation": main_workstation,
        "activity": main_activity,
        "active_skill": main_active_skill.or_else(|| enabled_skills.first().cloned()),
        "skills": enabled_skills,
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "subordinates": agent_list.iter()
            .filter(|a| a["role"] == ROLE_SUBAGENT)
            .map(|a| a["id"].clone())
            .collect::<Vec<_>>(),
        "supervisor": null,
    });

    let mut all_agents = vec![main_agent];
    all_agents.extend(agent_list);

    Json(json!({
        "agents": all_agents,
        "systems": systems,
        "files": files,
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
    let enabled_skills: Vec<&str> = skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.as_str())
        .collect();
    let skill_kinds: std::collections::HashMap<&str, Vec<&str>> = {
        let mut map: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for s in &skills {
            if s.enabled {
                map.entry(s.kind.as_str())
                    .or_default()
                    .push(s.name.as_str());
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

    let missions: Vec<Value> = directives
        .as_ref()
        .map(|d| {
            d.missions
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "timeframe": m.timeframe,
                        "priority": m.priority,
                        "description": m.description,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let firmware_rules: Vec<Value> = firmware
        .as_ref()
        .map(|f| {
            f.rules
                .iter()
                .map(|r| {
                    json!({
                        "type": r.rule_type,
                        "rule": r.rule,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let sub_agents = ironclad_db::agents::list_sub_agents(&state.db).unwrap_or_default();
    let session_counts =
        ironclad_db::agents::list_session_counts_by_agent(&state.db).unwrap_or_default();
    let taskable_sub_agents: Vec<&ironclad_db::agents::SubAgentRow> = sub_agents
        .iter()
        .filter(|sa| !sa.role.eq_ignore_ascii_case(ROLE_MODEL_PROXY))
        .collect();
    let model_proxies: Vec<&ironclad_db::agents::SubAgentRow> = sub_agents
        .iter()
        .filter(|sa| sa.role.eq_ignore_ascii_case(ROLE_MODEL_PROXY))
        .collect();

    let running_count = agents_in_registry
        .iter()
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .filter(|a| {
            taskable_sub_agents
                .iter()
                .any(|sa| sa.name.eq_ignore_ascii_case(&a.id))
        })
        .count();
    let stats = json!({
        "subordinate_count": taskable_sub_agents.len(),
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
        "description": os.as_ref().map(|o| {
            let first_line = o.prompt_text.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("Autonomous agent");
            first_line.to_string()
        }),
        "voice": voice,
        "missions": missions,
        "firmware_rules": firmware_rules,
        "skills": [],
        "capabilities": [
            "orchestrate-subagents",
            "assign-tasks",
            "select-subagent-model"
        ],
        "skill_breakdown": skill_kinds,
        "subordinates": taskable_sub_agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "stats": stats,
    });

    let specialist_cards: Vec<Value> = taskable_sub_agents.iter().enumerate().map(|(i, sa)| {
        let runtime = agents_in_registry.iter().find(|a| a.id == sa.name);
        let state_str = runtime.map(|r| format!("{:?}", r.state)).unwrap_or_else(|| {
            if sa.enabled { "Idle".into() } else { "Disabled".into() }
        });
        let model_mode = match sa.model.trim().to_ascii_lowercase().as_str() {
            "auto" => "auto",
            "commander" => "commander",
            _ => "fixed",
        };
        let color = WORKSPACE_PALETTE[(i + 1) % WORKSPACE_PALETTE.len()];
        json!({
            "id": sa.id,
            "name": sa.name,
            "display_name": sa.display_name,
            "role": ROLE_SUBAGENT,
            "model": sa.model,
            "model_mode": model_mode,
            "resolved_model": runtime.map(|r| r.model.clone()),
            "enabled": sa.enabled,
            "color": color,
            "state": state_str,
            "session_count": session_counts.get(&sa.name).copied().unwrap_or(sa.session_count),
            "description": sa.description,
            "skills": sa.skills_json.as_ref().and_then(|s| serde_json::from_str::<Vec<String>>(s).ok()).unwrap_or_default(),
            "supervisor": config.agent.id,
        })
    }).collect();

    let mut roster = vec![main_agent];
    roster.extend(specialist_cards);

    let proxies: Vec<Value> = model_proxies
        .iter()
        .map(|sa| {
            json!({
                "id": sa.id,
                "name": sa.name,
                "display_name": sa.display_name,
                "role": ROLE_MODEL_PROXY,
                "model": sa.model,
                "enabled": sa.enabled
            })
        })
        .collect();

    Json(json!({
        "roster": roster,
        "count": roster.len(),
        "taskable_subagent_count": taskable_sub_agents.len(),
        "model_proxy_count": proxies.len(),
        "model_proxies": proxies
    }))
}

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
) -> Result<impl IntoResponse, (StatusCode, String)> {
    let model = body.model.trim().to_string();
    if model.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "model cannot be empty".into()));
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
    let is_commander = agent_name == config.agent.name || agent_name == config.agent.id;
    let old_model;
    drop(config);

    if is_commander {
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

        // Synchronize active router immediately for commander model changes.
        {
            let mut llm = state.llm.write().await;
            llm.router.sync_runtime(
                models.primary.clone(),
                models.fallbacks.clone(),
                models.routing.clone(),
            );
        }
        Ok(axum::Json(json!({
            "updated": true,
            "agent": agent_name,
            "old_model": old_model,
            "new_model": model,
            "fallbacks": models.fallbacks,
            "model_order": std::iter::once(models.primary.clone())
                .chain(models.fallbacks.clone())
                .collect::<Vec<_>>(),
            "scope": "commander (runtime only, not persisted to disk)",
        })))
    } else {
        if body.fallbacks.is_some() {
            return Err((
                StatusCode::BAD_REQUEST,
                "fallback ordering is only supported for the commander agent".into(),
            ));
        }
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
        old_model = existing.model.clone();
        let mut updated = existing.clone();
        updated.model = model.clone();
        ironclad_db::agents::upsert_sub_agent(&state.db, &updated).map_err(|e| internal_err(&e))?;
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

// ── Keystore / provider key management ───────────────────────

#[derive(Deserialize)]
pub struct SetProviderKeyRequest {
    pub api_key: String,
}

pub async fn set_provider_key(
    State(state): State<AppState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<SetProviderKeyRequest>,
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let key = body.api_key.trim();
    if key.is_empty() {
        return Err((StatusCode::BAD_REQUEST, "api_key cannot be empty".into()));
    }

    let config = state.config.read().await;
    if !config.providers.contains_key(&name) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("provider '{name}' not found in config"),
        ));
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
) -> std::result::Result<impl IntoResponse, (StatusCode, String)> {
    let config = state.config.read().await;
    if !config.providers.contains_key(&name) {
        return Err((
            StatusCode::NOT_FOUND,
            format!("provider '{name}' not found in config"),
        ));
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

// ── Prompt efficiency metrics ────────────────────────────────

#[derive(Deserialize)]
pub struct EfficiencyParams {
    pub period: Option<String>,
    pub model: Option<String>,
}

pub async fn get_efficiency(
    State(state): State<AppState>,
    Query(params): Query<EfficiencyParams>,
) -> impl IntoResponse {
    let period = params.period.as_deref().unwrap_or("7d");
    let model = params.model.as_deref();

    match ironclad_db::efficiency::compute_efficiency(&state.db, period, model) {
        Ok(report) => Json(serde_json::to_value(report).unwrap_or_default()).into_response(),
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
) -> Result<serde_json::Value, (StatusCode, String)> {
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
    };

    let llm = state.llm.read().await;
    let provider = match llm.providers.get_by_model(&model) {
        Some(p) => p.clone(),
        None => {
            return Err((
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
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            format!("missing API key for provider {}", provider.name),
        ));
    }

    let url = format!("{}{}", provider.url, provider.chat_path);
    let body = ironclad_llm::format::translate_request(&req, provider.format)
        .unwrap_or_else(|_| serde_json::json!({}));
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
    )
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
) -> impl IntoResponse {
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
    Json(json!({ "ok": true, "agent_id": body.agent_id }))
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

    #[test]
    fn workstation_for_tool_uses_token_match_for_rg() {
        assert_eq!(workstation_for_tool("rg"), ("files", "tool_execution"));
        assert_eq!(
            workstation_for_tool("plugin-rg-runner"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("search_files"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("target-analyzer"),
            ("exec", "tool_execution")
        );
    }

    #[test]
    fn plugin_permissions_do_not_treat_urls_or_model_ids_as_filesystem() {
        let url_required = plugin_tool_required_permissions(
            "api_call",
            &json!({"endpoint": "https://example.com/v1/chat"}),
        );
        assert!(url_required.contains(&"network"));
        assert!(!url_required.contains(&"filesystem"));

        let model_required =
            plugin_tool_required_permissions("select_model", &json!({"model": "openai/gpt-4o"}));
        assert!(!model_required.contains(&"filesystem"));
        let nested_model_required = plugin_tool_required_permissions(
            "select_model",
            &json!({"model": {"id": "openai/gpt-4o"}}),
        );
        assert!(!nested_model_required.contains(&"filesystem"));
        let model_path_required =
            plugin_tool_required_permissions("select_model", &json!({"model": "/etc/passwd"}));
        assert!(model_path_required.contains(&"filesystem"));

        let generic_relative_path_required = plugin_tool_required_permissions(
            "process_input",
            &json!({"input": "secrets/config.yaml"}),
        );
        assert!(generic_relative_path_required.contains(&"filesystem"));

        let nested_path_required =
            plugin_tool_required_permissions("process_input", &json!({"path": {"name": "secret"}}));
        assert!(nested_path_required.contains(&"filesystem"));

        let file_required =
            plugin_tool_required_permissions("load_file", &json!({"path": "C:\\tmp\\input.txt"}));
        assert!(file_required.contains(&"filesystem"));

        let regex_required =
            plugin_tool_required_permissions("process_input", &json!({"pattern": "\\d+"}));
        assert!(!regex_required.contains(&"filesystem"));
    }

    #[test]
    fn plugin_permissions_match_shared_scan_for_input_matrix() {
        let cases = vec![
            json!({}),
            json!({"endpoint": "https://example.com/v1"}),
            json!({"socket": "wss://example.com/stream"}),
            json!({"model": "openai/gpt-4o"}),
            json!({"model": "/etc/passwd"}),
            json!({"path": "src/main.rs"}),
            json!({"input": "secrets/config.yaml"}),
            json!({"pattern": "\\d+\\w+\\s*"}),
            json!({"env_var": "SECRET_TOKEN"}),
        ];

        for input in cases {
            let scan = input_capability_scan::scan_input_capabilities(&input);
            let required = plugin_tool_required_permissions("neutral_tool", &input);
            assert_eq!(
                required.contains(&"filesystem"),
                scan.requires_filesystem,
                "filesystem mismatch for input: {input}"
            );
            assert_eq!(
                required.contains(&"network"),
                scan.requires_network,
                "network mismatch for input: {input}"
            );
        }
    }

    #[test]
    fn plugin_permissions_do_not_infer_network_from_tool_name_alone() {
        let required = plugin_tool_required_permissions("api_call", &json!({}));
        assert!(!required.contains(&"network"));
    }

    #[test]
    fn model_discovery_uses_ollama_tags_only_for_ollama_like_providers() {
        let (localish_ollama, url_ollama) =
            model_discovery_mode("ollama-gpu", "http://192.168.50.253:11434", true);
        assert!(localish_ollama);
        assert_eq!(url_ollama, "http://192.168.50.253:11434/api/tags");

        let (localish_proxy, url_proxy) =
            model_discovery_mode("anthropic", "http://127.0.0.1:8788/anthropic", false);
        assert!(!localish_proxy);
        assert_eq!(url_proxy, "http://127.0.0.1:8788/anthropic/v1/models");
    }

    #[test]
    fn apply_provider_auth_supports_query_key_mode() {
        let client = reqwest::Client::new();
        let req = client.get("http://example.test/v1/models");
        let built = apply_provider_auth(req, "query:key", "secret")
            .build()
            .expect("request builds");
        assert_eq!(
            built.url().as_str(),
            "http://example.test/v1/models?key=secret"
        );
    }

    #[test]
    fn classify_provider_connectivity_status_marks_local_proxy_refusal() {
        let (status, hint) = classify_provider_connectivity_status(
            "anthropic",
            "http://127.0.0.1:8788/anthropic",
            "http://127.0.0.1:8788/anthropic/v1/models",
            "error sending request for url: connect: connection refused",
            false,
        );
        assert_eq!(status, "legacy_proxy_unsupported");
        assert!(hint.unwrap_or_default().contains("providers.anthropic.url"));
    }

    #[test]
    fn loopback_nonlocal_proxy_can_be_marked_misconfigured() {
        assert!(is_loopback_url("http://127.0.0.1:8788/anthropic"));
        assert!(!is_loopback_url("https://api.anthropic.com"));
    }

    #[test]
    fn legacy_loopback_support_state_is_removed() {
        assert_eq!(legacy_loopback_support_state(), "removed_v0_8");
    }

    #[test]
    fn parse_db_timestamp_supports_rfc3339_and_sqlite_formats() {
        let rfc = parse_db_timestamp_utc("2026-02-26T10:11:12Z").unwrap();
        assert_eq!(rfc.to_rfc3339(), "2026-02-26T10:11:12+00:00");

        let sqlite = parse_db_timestamp_utc("2026-02-26 10:11:12").unwrap();
        assert_eq!(sqlite.to_rfc3339(), "2026-02-26T10:11:12+00:00");

        assert!(parse_db_timestamp_utc("not-a-time").is_none());
    }

    #[test]
    fn is_recent_activity_respects_window() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:12:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(is_recent_activity("2026-02-26T10:11:10Z", now));
        assert!(!is_recent_activity("2026-02-26T10:00:00Z", now));
    }

    #[test]
    fn has_tool_token_matches_exact_split_tokens_only() {
        assert!(has_tool_token("plugin-rg-runner", "rg"));
        assert!(!has_tool_token("merge", "rg"));
        assert!(!has_tool_token("larger", "rg"));
    }

    #[test]
    fn format_balance_rounds_and_appends_symbol() {
        assert_eq!(format_balance(1.2345, "USDC"), "1.23");
        assert_eq!(format_balance(0.0, "ETH"), "0.000000");
        assert_eq!(format_balance(0.123456789, "WBTC"), "0.12345679");
        assert_eq!(format_balance(0.123456, "OTHER"), "0.1235");
    }

    #[test]
    fn workspace_files_snapshot_filters_hidden_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.txt"), "z").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join(".hidden"), "h").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();

        let snap = workspace_files_snapshot(dir.path());
        let entries = snap["top_level_entries"].as_array().unwrap();
        let names: Vec<String> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.txt", "sub", "z.txt"]);
        assert_eq!(snap["entry_count"].as_u64(), Some(3));
    }

    #[test]
    fn derive_workspace_activity_prefers_recent_tool_call_then_turn_then_idle() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES (?1, ?2, 'agent', 'active')",
            rusqlite::params!["s1", "agent-1"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["t1", "s1", "2026-02-26T10:11:50Z"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_calls (id, turn_id, tool_name, input, status, created_at) VALUES (?1, ?2, ?3, '{}', 'ok', ?4)",
            rusqlite::params!["tc1", "t1", "read_file", "2026-02-26T10:11:59Z"],
        )
        .unwrap();
        drop(conn);

        let now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:12:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let active = derive_workspace_activity(&db, "agent-1", true, now);
        assert_eq!(active.0, Some("files"));
        assert_eq!(active.1, "tool_execution");
        assert_eq!(active.2.as_deref(), Some("read_file"));

        let conn = db.conn();
        conn.execute("DELETE FROM tool_calls", []).unwrap();
        drop(conn);
        let turn_only = derive_workspace_activity(&db, "agent-1", true, now);
        assert_eq!(turn_only.0, Some("llm"));
        assert_eq!(turn_only.1, "inference");

        let idle_now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:30:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let idle = derive_workspace_activity(&db, "agent-1", true, idle_now);
        assert_eq!(idle.0, Some("standby"));
        assert_eq!(idle.1, "idle");
    }

    // ── sanitize_decided_by tests ────────────────────────────────

    #[test]
    fn sanitize_decided_by_accepts_normal_input() {
        let result = sanitize_decided_by("admin-user").unwrap();
        assert_eq!(result, "admin-user");
    }

    #[test]
    fn sanitize_decided_by_strips_control_characters() {
        let result = sanitize_decided_by("user\x00\x01\x02name").unwrap();
        assert_eq!(result, "username");
    }

    #[test]
    fn sanitize_decided_by_rejects_too_long_input() {
        let long_input = "a".repeat(MAX_DECIDED_BY_LEN + 1);
        let result = sanitize_decided_by(&long_input);
        assert!(result.is_err());
        let (status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("max length"));
    }

    #[test]
    fn sanitize_decided_by_accepts_max_length() {
        let exact = "a".repeat(MAX_DECIDED_BY_LEN);
        let result = sanitize_decided_by(&exact);
        assert!(result.is_ok());
    }

    #[test]
    fn sanitize_decided_by_empty_is_ok() {
        let result = sanitize_decided_by("").unwrap();
        assert_eq!(result, "");
    }

    // ── merge_json depth limit tests ─────────────────────────────

    #[test]
    fn merge_json_depth_limit_replaces_at_max_depth() {
        // Build a deeply nested structure beyond MERGE_JSON_MAX_DEPTH
        let mut patch = json!("leaf");
        for _ in 0..12 {
            patch = json!({"nested": patch});
        }
        let mut base = json!({"nested": {"nested": {"nested": "old"}}});
        merge_json(&mut base, &patch);
        // Should not panic and should merge/replace
        assert!(base.is_object());
    }

    // ── format_balance additional tests ──────────────────────────

    #[test]
    fn format_balance_dai_two_decimals() {
        assert_eq!(format_balance(100.999, "DAI"), "101.00");
    }

    #[test]
    fn format_balance_usdt_two_decimals() {
        assert_eq!(format_balance(0.5, "USDT"), "0.50");
    }

    #[test]
    fn format_balance_matic_six_decimals() {
        assert_eq!(format_balance(1.0, "MATIC"), "1.000000");
    }

    #[test]
    fn format_balance_weth_six_decimals() {
        assert_eq!(format_balance(0.1, "WETH"), "0.100000");
    }

    #[test]
    fn format_balance_cbbtc_eight_decimals() {
        assert_eq!(format_balance(0.5, "cbBTC"), "0.50000000");
    }

    // ── is_loopback_url additional tests ─────────────────────────

    #[test]
    fn is_loopback_url_localhost_case_insensitive() {
        assert!(is_loopback_url("http://LOCALHOST:8080"));
    }

    #[test]
    fn is_loopback_url_rejects_remote() {
        assert!(!is_loopback_url("https://api.openai.com/v1"));
    }

    // ── model_discovery_mode additional tests ────────────────────

    #[test]
    fn model_discovery_mode_local_flag_makes_keyless() {
        let (keyless, url) = model_discovery_mode("custom-local", "http://192.168.1.5:8080", true);
        assert!(keyless);
        assert_eq!(url, "http://192.168.1.5:8080/v1/models");
    }

    #[test]
    fn model_discovery_mode_remote_not_keyless() {
        let (keyless, url) = model_discovery_mode("openai", "https://api.openai.com", false);
        assert!(!keyless);
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    #[test]
    fn model_discovery_mode_port_11434_is_ollama_like() {
        let (keyless, url) =
            model_discovery_mode("my-provider", "http://192.168.50.253:11434", false);
        assert!(keyless);
        assert_eq!(url, "http://192.168.50.253:11434/api/tags");
    }

    // ── workstation_for_tool additional categories ────────────────

    #[test]
    fn workstation_for_tool_web_tools() {
        assert_eq!(workstation_for_tool("web_fetch"), ("web", "tool_execution"));
        assert_eq!(
            workstation_for_tool("http_request"),
            ("web", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_memory() {
        assert_eq!(workstation_for_tool("memory_store"), ("memory", "working"));
    }

    #[test]
    fn workstation_for_tool_blockchain() {
        assert_eq!(
            workstation_for_tool("wallet_balance"),
            ("blockchain", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("contract_call"),
            ("blockchain", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_file_operations() {
        assert_eq!(
            workstation_for_tool("read_file"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("write_output"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("glob_search"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("edit_code"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("patch_file"),
            ("files", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_unknown_falls_to_exec() {
        assert_eq!(
            workstation_for_tool("completely_unknown"),
            ("exec", "tool_execution")
        );
    }

    // ── has_tool_token additional tests ───────────────────────────

    #[test]
    fn has_tool_token_matches_at_boundaries() {
        assert!(has_tool_token("rg", "rg"));
        assert!(has_tool_token("my-rg-tool", "rg"));
        assert!(has_tool_token("rg-runner", "rg"));
    }

    #[test]
    fn has_tool_token_no_partial_match() {
        assert!(!has_tool_token("debugging", "bug"));
    }

    // ── workspace_files_snapshot edge case ────────────────────────

    #[test]
    fn workspace_files_snapshot_handles_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let snap = workspace_files_snapshot(dir.path());
        let entries = snap["top_level_entries"].as_array().unwrap();
        assert!(entries.is_empty());
        assert_eq!(snap["entry_count"].as_u64(), Some(0));
    }

    #[test]
    fn workspace_files_snapshot_handles_nonexistent_directory() {
        let snap = workspace_files_snapshot(std::path::Path::new("/nonexistent/path"));
        let entries = snap["top_level_entries"].as_array().unwrap();
        assert!(entries.is_empty());
    }

    // ── derive_workspace_activity standby ────────────────────────

    #[test]
    fn derive_workspace_activity_returns_standby_when_not_running() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let now = chrono::Utc::now();
        let (workstation, phase, _tool) = derive_workspace_activity(&db, "agent-1", false, now);
        assert_eq!(workstation, Some("standby"));
        assert_eq!(phase, "idle");
    }

    // ── default_decided_by test ──────────────────────────────────

    #[test]
    fn default_decided_by_returns_api() {
        assert_eq!(default_decided_by(), "api");
    }

    // ── legacy_loopback_support_state test ────────────────────────

    #[test]
    fn legacy_loopback_removed() {
        assert_eq!(legacy_loopback_support_state(), "removed_v0_8");
    }

    // ── parse_db_timestamp_utc edge cases ────────────────────────

    #[test]
    fn parse_db_timestamp_utc_handles_offset_timestamps() {
        use chrono::Timelike;
        let dt = parse_db_timestamp_utc("2026-01-15T08:30:00+05:30").unwrap();
        assert_eq!(dt.hour(), 3); // 08:30 +05:30 = 03:00 UTC
    }

    #[test]
    fn parse_db_timestamp_utc_empty_string() {
        assert!(parse_db_timestamp_utc("").is_none());
    }

    // ── KeySource status_pair tests ──────────────────────────────

    #[test]
    fn key_source_status_pairs() {
        assert_eq!(KeySource::NotRequired.status_pair(), ("not_required", "local"));
        assert_eq!(KeySource::OAuth.status_pair(), ("configured", "oauth"));
        assert_eq!(
            KeySource::Keystore("test".into()).status_pair(),
            ("configured", "keystore")
        );
        assert_eq!(
            KeySource::EnvVar("TEST_KEY".into()).status_pair(),
            ("configured", "env")
        );
        assert_eq!(KeySource::Missing.status_pair(), ("missing", "none"));
    }
}
