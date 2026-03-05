use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;

use super::{
    AppState, JsonError, bad_request, internal_err, not_found, sanitize_html, validate_long,
    validate_short,
};

pub(crate) const ROLE_SUBAGENT: &str = "subagent";
pub(crate) const ROLE_MODEL_PROXY: &str = "model-proxy";
pub(crate) const MODEL_MODE_AUTO: &str = "auto";
pub(crate) const MODEL_MODE_ORCHESTRATOR: &str = "orchestrator";

#[derive(Deserialize)]
pub struct CreateSubAgentRequest {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub fallback_models: Vec<String>,
    #[serde(default)]
    pub personality: Option<Value>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_role() -> String {
    ROLE_SUBAGENT.into()
}

fn default_model() -> String {
    MODEL_MODE_AUTO.into()
}
fn default_true() -> bool {
    true
}

#[derive(Deserialize)]
pub struct UpdateSubAgentRequest {
    pub display_name: Option<String>,
    pub model: Option<String>,
    pub role: Option<String>,
    pub description: Option<String>,
    pub skills: Option<Vec<String>>,
    pub fallback_models: Option<Vec<String>>,
    #[serde(default)]
    pub personality: Option<Value>,
    pub enabled: Option<bool>,
}

const MAX_SUBAGENT_NAME_LEN: usize = 128;

pub(crate) fn validate_subagent_name(name: &str) -> Result<(), JsonError> {
    if name.is_empty() {
        return Err(bad_request("subagent name cannot be empty"));
    }
    if name.len() > MAX_SUBAGENT_NAME_LEN {
        return Err(bad_request(format!(
            "subagent name exceeds max length of {MAX_SUBAGENT_NAME_LEN} characters"
        )));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(bad_request(
            "subagent name may only contain alphanumeric characters, hyphens, and underscores",
        ));
    }
    Ok(())
}

pub(crate) fn normalize_role(raw: &str) -> Option<&'static str> {
    let v = raw.trim().to_ascii_lowercase();
    match v.as_str() {
        ROLE_SUBAGENT | "specialist" => Some(ROLE_SUBAGENT),
        ROLE_MODEL_PROXY => Some(ROLE_MODEL_PROXY),
        _ => None,
    }
}

pub(crate) fn normalize_skills(skills: &[String]) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for s in skills {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            out.insert(trimmed.to_string());
        }
    }
    out.into_iter().collect()
}

pub(crate) fn normalize_model_input(model: &str) -> String {
    model.trim().to_string()
}

pub(crate) fn parse_fallback_models_json(raw: Option<&str>) -> Vec<String> {
    raw.and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

pub(crate) fn normalize_fallback_models(models: &[String], primary_model: &str) -> Vec<String> {
    let primary = primary_model.trim();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for m in models {
        let trimmed = m.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case(primary) {
            continue;
        }
        // Preserve caller order while de-duplicating case-insensitively.
        let key = trimmed.to_ascii_lowercase();
        if seen.insert(key) {
            out.push(trimmed.to_string());
        }
    }
    out
}

pub(crate) fn is_model_mode(model: &str) -> bool {
    matches!(
        model.trim().to_ascii_lowercase().as_str(),
        MODEL_MODE_AUTO | MODEL_MODE_ORCHESTRATOR
    )
}

pub(crate) fn is_concrete_provider_model(model: &str) -> bool {
    let trimmed = model.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((provider, model_name)) = trimmed.split_once('/') else {
        return false;
    };
    !provider.trim().is_empty() && !model_name.trim().is_empty()
}

pub(crate) fn validate_subagent_model_for_role(role: &str, model: &str) -> Result<(), JsonError> {
    let normalized = normalize_role(role).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "role must be 'subagent' or 'model-proxy'".to_string(),
        )
    })?;
    if model.trim().is_empty() {
        return Err(bad_request(
            "model cannot be empty; use a concrete provider/model, 'auto', or 'orchestrator'",
        ));
    }
    if normalized == ROLE_MODEL_PROXY && is_model_mode(model) {
        return Err(bad_request(
            "model-proxy entries require a concrete provider/model, not 'auto' or 'orchestrator'",
        ));
    }
    if !is_model_mode(model) && !is_concrete_provider_model(model) {
        return Err(bad_request(
            "model must be provider/model format, or one of: 'auto', 'orchestrator'",
        ));
    }
    Ok(())
}

pub(crate) async fn resolve_taskable_subagent_runtime_model(
    state: &AppState,
    configured_model: &str,
) -> String {
    let model = configured_model.trim().to_ascii_lowercase();
    match model.as_str() {
        MODEL_MODE_AUTO => super::agent::select_routed_model(state, "").await,
        MODEL_MODE_ORCHESTRATOR => {
            let llm = state.llm.read().await;
            llm.router.select_model().to_string()
        }
        _ if is_concrete_provider_model(configured_model) => configured_model.trim().to_string(),
        _ => {
            tracing::warn!(
                configured_model = %configured_model,
                "invalid fixed subagent model; falling back to auto model routing"
            );
            super::agent::select_routed_model(state, "").await
        }
    }
}

pub(crate) fn validate_subagent_contract(
    role: &str,
    model: &str,
    skills: &[String],
    personality: Option<&Value>,
) -> Result<(), JsonError> {
    let normalized = normalize_role(role).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "role must be 'subagent' or 'model-proxy'".to_string(),
        )
    })?;
    if personality.is_some() {
        return Err(bad_request(
            "personality is not supported for subagents; subagents must be personality-free",
        ));
    }
    if normalized == ROLE_MODEL_PROXY && !skills.is_empty() {
        return Err(bad_request(
            "model-proxy entries cannot own skills; only taskable subagents may have fixed skills",
        ));
    }
    validate_subagent_model_for_role(normalized, model)?;
    Ok(())
}

fn runtime_state_label(state: ironclad_agent::subagents::AgentRunState) -> &'static str {
    match state {
        ironclad_agent::subagents::AgentRunState::Idle => "idle",
        ironclad_agent::subagents::AgentRunState::Starting => "booting",
        ironclad_agent::subagents::AgentRunState::Running => "running",
        ironclad_agent::subagents::AgentRunState::Stopped => "stopped",
        ironclad_agent::subagents::AgentRunState::Error => "error",
    }
}

pub async fn list_sub_agents(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::agents::list_sub_agents(&state.db) {
        Ok(agents) => {
            let runtime = state.registry.list_agents().await;
            let runtime_by_name: HashMap<String, ironclad_agent::subagents::AgentInstance> =
                runtime
                    .into_iter()
                    .map(|a| (a.id.to_ascii_lowercase(), a))
                    .collect();
            let session_counts = ironclad_db::agents::list_session_counts_by_agent(&state.db)
                .inspect_err(
                    |e| tracing::warn!(error = %e, "failed to load session counts for sub-agents"),
                )
                .unwrap_or_default();
            let mut booting = 0usize;
            let mut running = 0usize;
            let mut errored = 0usize;
            let items: Vec<serde_json::Value> = agents
                .into_iter()
                .map(|a| {
                    let normalized_role = normalize_role(&a.role).unwrap_or(ROLE_SUBAGENT);
                    let runtime_entry = runtime_by_name.get(&a.name.to_ascii_lowercase());
                    let skills = a
                        .skills_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                        .unwrap_or_default();
                    let fallback_models =
                        parse_fallback_models_json(a.fallback_models_json.as_deref());
                    let session_count = session_counts
                        .get(&a.name)
                        .copied()
                        .unwrap_or(a.session_count);
                    let runtime_state = if normalized_role == ROLE_MODEL_PROXY {
                        "n/a".to_string()
                    } else if let Some(inst) = runtime_entry {
                        runtime_state_label(inst.state).to_string()
                    } else if a.enabled {
                        "booting".to_string()
                    } else {
                        "stopped".to_string()
                    };
                    let taskable = a.enabled && runtime_state == "running";
                    if normalized_role != ROLE_MODEL_PROXY {
                        match runtime_state.as_str() {
                            "booting" | "idle" => booting += 1,
                            "running" => running += 1,
                            "error" => errored += 1,
                            _ => {}
                        }
                    }
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "display_name": a.display_name,
                        "model": a.model,
                        "role": normalized_role,
                        "description": a.description,
                        "skills": skills,
                        "fallback_models": fallback_models,
                        "enabled": a.enabled,
                        "session_count": session_count,
                        "runtime_state": runtime_state,
                        "taskable": taskable,
                    })
                })
                .collect();
            let count = items.len();
            Ok(axum::Json(serde_json::json!({
                "agents": items,
                "count": count,
                "runtime_summary": {
                    "booting": booting,
                    "running": running,
                    "error": errored,
                }
            })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn create_sub_agent(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateSubAgentRequest>,
) -> Result<impl IntoResponse, JsonError> {
    validate_short("name", &body.name)?;
    if let Some(ref d) = body.description {
        validate_long("description", d)?;
    }
    let body = CreateSubAgentRequest {
        name: sanitize_html(&body.name),
        description: body.description.as_deref().map(sanitize_html),
        display_name: body.display_name.as_deref().map(sanitize_html),
        ..body
    };
    validate_subagent_name(&body.name)?;
    let role = normalize_role(&body.role)
        .ok_or_else(|| {
            (
                axum::http::StatusCode::BAD_REQUEST,
                "role must be 'subagent' or 'model-proxy'".to_string(),
            )
        })?
        .to_string();
    let model = normalize_model_input(&body.model);
    let skills = normalize_skills(&body.skills);
    let fallback_models = normalize_fallback_models(&body.fallback_models, &model);
    validate_subagent_contract(&role, &model, &skills, body.personality.as_ref())?;
    let agent = ironclad_db::agents::SubAgentRow {
        id: uuid::Uuid::new_v4().to_string(),
        name: body.name.clone(),
        display_name: body.display_name.or_else(|| {
            Some(
                body.name
                    .split('-')
                    .map(|w| {
                        let mut c = w.chars();
                        match c.next() {
                            None => String::new(),
                            Some(f) => f.to_uppercase().to_string() + c.as_str(),
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" "),
            )
        }),
        model,
        fallback_models_json: Some(
            serde_json::to_string(&fallback_models).unwrap_or_else(|_| "[]".to_string()),
        ),
        role,
        description: body.description,
        skills_json: Some(serde_json::to_string(&skills).unwrap_or_else(|_| "[]".to_string())),
        enabled: body.enabled,
        session_count: 0,
    };

    match ironclad_db::agents::upsert_sub_agent(&state.db, &agent) {
        Ok(()) => {
            if agent.enabled && agent.role == ROLE_SUBAGENT {
                let config = ironclad_agent::subagents::AgentInstanceConfig {
                    id: agent.name.clone(),
                    name: agent
                        .display_name
                        .clone()
                        .unwrap_or_else(|| agent.name.clone()),
                    model: resolve_taskable_subagent_runtime_model(&state, &agent.model).await,
                    skills,
                    allowed_subagents: vec![],
                    max_concurrent: 4,
                };
                if let Err(e) = state.registry.register(config).await {
                    tracing::error!(agent = %agent.name, error = %e, "failed to register sub-agent in runtime");
                }
                if let Err(e) = state.registry.start_agent(&agent.name).await {
                    tracing::error!(agent = %agent.name, error = %e, "failed to start sub-agent in runtime");
                }
            }
            Ok(axum::Json(serde_json::json!({
                "id": agent.id,
                "name": agent.name,
                "created": true,
            })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn update_sub_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
    axum::Json(body): axum::Json<UpdateSubAgentRequest>,
) -> Result<impl IntoResponse, JsonError> {
    if let Some(ref d) = body.description {
        validate_long("description", d)?;
    }
    if let Some(ref d) = body.display_name {
        validate_short("display_name", d)?;
    }
    let agents = ironclad_db::agents::list_sub_agents(&state.db).map_err(|e| internal_err(&e))?;

    let existing = agents.iter().find(|a| a.name == name).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("sub-agent '{name}' not found"),
        )
    })?;

    let merged_role = body
        .role
        .as_deref()
        .or(Some(existing.role.as_str()))
        .unwrap_or(ROLE_SUBAGENT);
    let normalized_role = normalize_role(merged_role).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "role must be 'subagent' or 'model-proxy'".to_string(),
        )
    })?;
    let merged_skills = body.skills.as_deref().map_or_else(
        || {
            existing
                .skills_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default()
        },
        normalize_skills,
    );
    let merged_model = body
        .model
        .as_deref()
        .map(normalize_model_input)
        .filter(|m| !m.is_empty())
        .unwrap_or_else(|| existing.model.clone());
    let merged_fallback_models = body.fallback_models.as_deref().map_or_else(
        || parse_fallback_models_json(existing.fallback_models_json.as_deref()),
        |v| normalize_fallback_models(v, &merged_model),
    );
    validate_subagent_contract(
        normalized_role,
        &merged_model,
        &merged_skills,
        body.personality.as_ref(),
    )?;

    let updated = ironclad_db::agents::SubAgentRow {
        id: existing.id.clone(),
        name: existing.name.clone(),
        display_name: body.display_name.or(existing.display_name.clone()),
        model: merged_model,
        fallback_models_json: Some(
            serde_json::to_string(&merged_fallback_models).unwrap_or_else(|_| "[]".to_string()),
        ),
        role: normalized_role.to_string(),
        description: body.description.or(existing.description.clone()),
        skills_json: Some(
            serde_json::to_string(&merged_skills).unwrap_or_else(|_| "[]".to_string()),
        ),
        enabled: body.enabled.unwrap_or(existing.enabled),
        session_count: existing.session_count,
    };

    ironclad_db::agents::upsert_sub_agent(&state.db, &updated).map_err(|e| internal_err(&e))?;

    if updated.role == ROLE_SUBAGENT && updated.enabled {
        if state.registry.get_agent(&updated.name).await.is_none() {
            let config = ironclad_agent::subagents::AgentInstanceConfig {
                id: updated.name.clone(),
                name: updated
                    .display_name
                    .clone()
                    .unwrap_or_else(|| updated.name.clone()),
                model: resolve_taskable_subagent_runtime_model(&state, &updated.model).await,
                skills: merged_skills.clone(),
                allowed_subagents: vec![],
                max_concurrent: 4,
            };
            if let Err(e) = state.registry.register(config).await {
                tracing::error!(agent = %updated.name, error = %e, "failed to register sub-agent in runtime");
            }
        }
        if let Err(e) = state.registry.start_agent(&updated.name).await {
            tracing::error!(agent = %updated.name, error = %e, "failed to start sub-agent in runtime");
        }
    } else {
        if let Err(e) = state.registry.stop_agent(&updated.name).await {
            tracing::error!(agent = %updated.name, error = %e, "failed to stop sub-agent in runtime");
        }
        if !state.registry.unregister(&updated.name).await {
            tracing::warn!(agent = %updated.name, "sub-agent was not registered in runtime during unregister");
        }
    }

    Ok(axum::Json(serde_json::json!({
        "updated": true,
        "name": name,
    })))
}

pub async fn delete_sub_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    match ironclad_db::agents::delete_sub_agent(&state.db, &name) {
        Ok(true) => {
            if let Err(e) = state.registry.stop_agent(&name).await {
                tracing::error!(agent = %name, error = %e, "failed to stop sub-agent in runtime during delete");
            }
            if !state.registry.unregister(&name).await {
                tracing::warn!(agent = %name, "sub-agent was not registered in runtime during delete");
            }
            Ok(axum::Json(
                serde_json::json!({ "deleted": true, "name": name }),
            ))
        }
        Ok(false) => Err(not_found(format!("sub-agent '{name}' not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn toggle_sub_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    let agents = ironclad_db::agents::list_sub_agents(&state.db).map_err(|e| internal_err(&e))?;

    let existing = agents.iter().find(|a| a.name == name).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("sub-agent '{name}' not found"),
        )
    })?;

    let new_enabled = !existing.enabled;
    let mut updated = existing.clone();
    updated.enabled = new_enabled;

    ironclad_db::agents::upsert_sub_agent(&state.db, &updated).map_err(|e| internal_err(&e))?;

    if updated.role == ROLE_SUBAGENT && updated.enabled {
        if state.registry.get_agent(&updated.name).await.is_none() {
            let skills = updated
                .skills_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default();
            let config = ironclad_agent::subagents::AgentInstanceConfig {
                id: updated.name.clone(),
                name: updated
                    .display_name
                    .clone()
                    .unwrap_or_else(|| updated.name.clone()),
                model: resolve_taskable_subagent_runtime_model(&state, &updated.model).await,
                skills,
                allowed_subagents: vec![],
                max_concurrent: 4,
            };
            if let Err(e) = state.registry.register(config).await {
                tracing::error!(agent = %updated.name, error = %e, "failed to register sub-agent in runtime");
            }
        }
        if let Err(e) = state.registry.start_agent(&updated.name).await {
            tracing::error!(agent = %updated.name, error = %e, "failed to start sub-agent in runtime");
        }
    } else {
        if let Err(e) = state.registry.stop_agent(&updated.name).await {
            tracing::error!(agent = %updated.name, error = %e, "failed to stop sub-agent in runtime");
        }
        if !state.registry.unregister(&updated.name).await {
            tracing::warn!(agent = %updated.name, "sub-agent was not registered in runtime during toggle");
        }
    }

    Ok(axum::Json(serde_json::json!({
        "name": name,
        "enabled": new_enabled,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── default_role / default_model / default_true ──────────────

    #[test]
    fn default_role_is_subagent() {
        assert_eq!(default_role(), "subagent");
    }

    #[test]
    fn default_model_is_auto() {
        assert_eq!(default_model(), "auto");
    }

    #[test]
    fn default_true_returns_true() {
        assert!(default_true());
    }

    // ── validate_subagent_name ──────────────────────────────────

    #[test]
    fn validate_subagent_name_accepts_valid_names() {
        assert!(validate_subagent_name("geo-specialist").is_ok());
        assert!(validate_subagent_name("agent_1").is_ok());
        assert!(validate_subagent_name("A").is_ok());
        assert!(validate_subagent_name("abc-def_123").is_ok());
    }

    #[test]
    fn validate_subagent_name_rejects_empty() {
        let err = validate_subagent_name("").unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.1.contains("empty"));
    }

    #[test]
    fn validate_subagent_name_rejects_too_long() {
        let long_name = "a".repeat(MAX_SUBAGENT_NAME_LEN + 1);
        let err = validate_subagent_name(&long_name).unwrap_err();
        assert_eq!(err.0, axum::http::StatusCode::BAD_REQUEST);
        assert!(err.1.contains("max length"));
    }

    #[test]
    fn validate_subagent_name_rejects_special_chars() {
        let err = validate_subagent_name("agent name").unwrap_err();
        assert!(err.1.contains("alphanumeric"));

        let err = validate_subagent_name("agent.name").unwrap_err();
        assert!(err.1.contains("alphanumeric"));

        let err = validate_subagent_name("agent/name").unwrap_err();
        assert!(err.1.contains("alphanumeric"));
    }

    #[test]
    fn validate_subagent_name_at_boundary_length() {
        let exactly_max = "a".repeat(MAX_SUBAGENT_NAME_LEN);
        assert!(validate_subagent_name(&exactly_max).is_ok());
    }

    // ── normalize_role ──────────────────────────────────────────

    #[test]
    fn normalize_role_accepts_subagent_and_specialist() {
        assert_eq!(normalize_role("subagent"), Some(ROLE_SUBAGENT));
        assert_eq!(normalize_role("specialist"), Some(ROLE_SUBAGENT));
        assert_eq!(normalize_role("  Subagent  "), Some(ROLE_SUBAGENT));
        assert_eq!(normalize_role("SPECIALIST"), Some(ROLE_SUBAGENT));
    }

    #[test]
    fn normalize_role_accepts_model_proxy() {
        assert_eq!(normalize_role("model-proxy"), Some(ROLE_MODEL_PROXY));
        assert_eq!(normalize_role("MODEL-PROXY"), Some(ROLE_MODEL_PROXY));
    }

    #[test]
    fn normalize_role_rejects_unknown() {
        assert_eq!(normalize_role("orchestrator"), None);
        assert_eq!(normalize_role(""), None);
        assert_eq!(normalize_role("worker"), None);
    }

    // ── normalize_skills ────────────────────────────────────────

    #[test]
    fn normalize_skills_deduplicates_and_sorts() {
        let skills = vec![
            "risk".to_string(),
            "geo".to_string(),
            "risk".to_string(),
            "  geo  ".to_string(),
        ];
        let out = normalize_skills(&skills);
        assert_eq!(out, vec!["geo", "risk"]);
    }

    #[test]
    fn normalize_skills_removes_empty_and_whitespace() {
        let skills = vec!["".to_string(), "  ".to_string(), "analysis".to_string()];
        let out = normalize_skills(&skills);
        assert_eq!(out, vec!["analysis"]);
    }

    #[test]
    fn normalize_skills_empty_input() {
        assert!(normalize_skills(&[]).is_empty());
    }

    #[test]
    fn normalize_fallback_models_preserves_order() {
        let models = vec![
            "moonshot/kimi-k2-turbo-preview".to_string(),
            "openrouter/openai/gpt-4o".to_string(),
            "moonshot/kimi-k2-turbo-preview".to_string(),
        ];
        let out = normalize_fallback_models(&models, "openai/gpt-5.3-codex");
        assert_eq!(
            out,
            vec!["moonshot/kimi-k2-turbo-preview", "openrouter/openai/gpt-4o"]
        );
    }

    #[test]
    fn normalize_fallback_models_drops_primary_case_insensitively() {
        let models = vec![
            "moonshot/kimi-k2-turbo-preview".to_string(),
            "MOONSHOT/KIMI-K2-TURBO-PREVIEW".to_string(),
            "openrouter/openai/gpt-4o".to_string(),
        ];
        let out = normalize_fallback_models(&models, "moonshot/kimi-k2-turbo-preview");
        assert_eq!(out, vec!["openrouter/openai/gpt-4o"]);
    }

    // ── normalize_model_input ───────────────────────────────────

    #[test]
    fn normalize_model_input_trims_whitespace() {
        assert_eq!(normalize_model_input("  openai/gpt-4o  "), "openai/gpt-4o");
        assert_eq!(normalize_model_input("auto"), "auto");
    }

    // ── is_model_mode ───────────────────────────────────────────

    #[test]
    fn is_model_mode_detects_auto_and_orchestrator() {
        assert!(is_model_mode("auto"));
        assert!(is_model_mode("AUTO"));
        assert!(is_model_mode("orchestrator"));
        assert!(is_model_mode("ORCHESTRATOR"));
        assert!(is_model_mode("  auto  "));
    }

    #[test]
    fn is_model_mode_rejects_concrete_models() {
        assert!(!is_model_mode("openai/gpt-4o"));
        assert!(!is_model_mode("anthropic/claude-sonnet-4-20250514"));
        assert!(!is_model_mode(""));
    }

    #[test]
    fn is_concrete_provider_model_requires_provider_slash_model() {
        assert!(is_concrete_provider_model("openai/gpt-4o"));
        assert!(is_concrete_provider_model("ollama/qwen3:8b"));
        assert!(!is_concrete_provider_model("orca-ata"));
        assert!(!is_concrete_provider_model("openai/"));
        assert!(!is_concrete_provider_model("/gpt-4o"));
    }

    // ── validate_subagent_contract ──────────────────────────────

    #[test]
    fn validate_contract_accepts_valid_subagent() {
        assert!(
            validate_subagent_contract(
                "subagent",
                "auto",
                &["geo".to_string(), "risk".to_string()],
                None
            )
            .is_ok()
        );
    }

    #[test]
    fn validate_contract_accepts_valid_model_proxy() {
        assert!(validate_subagent_contract("model-proxy", "openai/gpt-4o", &[], None).is_ok());
    }

    #[test]
    fn validate_contract_rejects_unknown_role() {
        let err = validate_subagent_contract("worker", "auto", &[], None).unwrap_err();
        assert!(err.1.contains("role must be"));
    }

    #[test]
    fn validate_contract_rejects_personality() {
        let personality = serde_json::json!({"tone": "formal"});
        let err =
            validate_subagent_contract("subagent", "auto", &[], Some(&personality)).unwrap_err();
        assert!(err.1.contains("personality"));
    }

    #[test]
    fn validate_contract_rejects_model_proxy_with_skills() {
        let err =
            validate_subagent_contract("model-proxy", "openai/gpt-4o", &["geo".to_string()], None)
                .unwrap_err();
        assert!(err.1.contains("cannot own skills"));
    }

    #[test]
    fn validate_contract_rejects_empty_model() {
        let err = validate_subagent_contract("subagent", "  ", &[], None).unwrap_err();
        assert!(err.1.contains("model cannot be empty"));
    }

    #[test]
    fn validate_contract_rejects_model_proxy_with_auto() {
        let err = validate_subagent_contract("model-proxy", "auto", &[], None).unwrap_err();
        assert!(err.1.contains("concrete provider/model"));
    }

    #[test]
    fn validate_contract_rejects_model_proxy_with_orchestrator() {
        let err = validate_subagent_contract("model-proxy", "orchestrator", &[], None).unwrap_err();
        assert!(err.1.contains("concrete provider/model"));
    }

    #[test]
    fn validate_contract_rejects_invalid_fixed_model_identifier() {
        let err = validate_subagent_contract("subagent", "orca-ata", &[], None).unwrap_err();
        assert!(err.1.contains("provider/model format"));
    }

    // ── runtime_state_label ─────────────────────────────────────

    #[test]
    fn runtime_state_label_maps_all_states() {
        use ironclad_agent::subagents::AgentRunState;
        assert_eq!(runtime_state_label(AgentRunState::Idle), "idle");
        assert_eq!(runtime_state_label(AgentRunState::Starting), "booting");
        assert_eq!(runtime_state_label(AgentRunState::Running), "running");
        assert_eq!(runtime_state_label(AgentRunState::Stopped), "stopped");
        assert_eq!(runtime_state_label(AgentRunState::Error), "error");
    }

    // ── CreateSubAgentRequest deserialization ────────────────────

    #[test]
    fn create_request_defaults() {
        let json = serde_json::json!({
            "name": "test-agent"
        });
        let req: CreateSubAgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name, "test-agent");
        assert_eq!(req.model, "auto");
        assert_eq!(req.role, "subagent");
        assert!(req.enabled);
        assert!(req.skills.is_empty());
        assert!(req.display_name.is_none());
        assert!(req.description.is_none());
        assert!(req.personality.is_none());
    }

    #[test]
    fn create_request_with_all_fields() {
        let json = serde_json::json!({
            "name": "geo-specialist",
            "display_name": "Geo Specialist",
            "model": "openai/gpt-4o",
            "role": "model-proxy",
            "description": "proxy for openai",
            "skills": [],
            "enabled": false
        });
        let req: CreateSubAgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(req.name, "geo-specialist");
        assert_eq!(req.display_name.as_deref(), Some("Geo Specialist"));
        assert_eq!(req.model, "openai/gpt-4o");
        assert_eq!(req.role, "model-proxy");
        assert!(!req.enabled);
    }

    // ── UpdateSubAgentRequest deserialization ────────────────────

    #[test]
    fn update_request_all_none() {
        let json = serde_json::json!({});
        let req: UpdateSubAgentRequest = serde_json::from_value(json).unwrap();
        assert!(req.display_name.is_none());
        assert!(req.model.is_none());
        assert!(req.role.is_none());
        assert!(req.description.is_none());
        assert!(req.skills.is_none());
        assert!(req.personality.is_none());
        assert!(req.enabled.is_none());
    }

    #[test]
    fn update_request_partial_fields() {
        let json = serde_json::json!({
            "model": "anthropic/claude-sonnet-4-20250514",
            "enabled": false
        });
        let req: UpdateSubAgentRequest = serde_json::from_value(json).unwrap();
        assert_eq!(
            req.model.as_deref(),
            Some("anthropic/claude-sonnet-4-20250514")
        );
        assert_eq!(req.enabled, Some(false));
        assert!(req.role.is_none());
    }
}
