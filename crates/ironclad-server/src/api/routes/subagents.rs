use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, internal_err};

const ROLE_SUBAGENT: &str = "subagent";
const ROLE_MODEL_PROXY: &str = "model-proxy";
const MODEL_MODE_AUTO: &str = "auto";
const MODEL_MODE_COMMANDER: &str = "commander";

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
    #[serde(default)]
    pub personality: Option<Value>,
    pub enabled: Option<bool>,
}

fn normalize_role(raw: &str) -> Option<&'static str> {
    let v = raw.trim().to_ascii_lowercase();
    match v.as_str() {
        ROLE_SUBAGENT | "specialist" => Some(ROLE_SUBAGENT),
        ROLE_MODEL_PROXY => Some(ROLE_MODEL_PROXY),
        _ => None,
    }
}

fn normalize_skills(skills: &[String]) -> Vec<String> {
    let mut out = std::collections::BTreeSet::new();
    for s in skills {
        let trimmed = s.trim();
        if !trimmed.is_empty() {
            out.insert(trimmed.to_string());
        }
    }
    out.into_iter().collect()
}

fn normalize_model_input(model: &str) -> String {
    model.trim().to_string()
}

fn is_model_mode(model: &str) -> bool {
    matches!(
        model.trim().to_ascii_lowercase().as_str(),
        MODEL_MODE_AUTO | MODEL_MODE_COMMANDER
    )
}

async fn resolve_taskable_subagent_runtime_model(
    state: &AppState,
    configured_model: &str,
) -> String {
    let model = configured_model.trim().to_ascii_lowercase();
    match model.as_str() {
        MODEL_MODE_AUTO => super::agent::select_routed_model(state, "").await,
        MODEL_MODE_COMMANDER => {
            let llm = state.llm.read().await;
            llm.router.select_model().to_string()
        }
        _ => configured_model.trim().to_string(),
    }
}

fn validate_subagent_contract(
    role: &str,
    model: &str,
    skills: &[String],
    personality: Option<&Value>,
) -> Result<(), (axum::http::StatusCode, String)> {
    let normalized = normalize_role(role).ok_or_else(|| {
        (
            axum::http::StatusCode::BAD_REQUEST,
            "role must be 'subagent' or 'model-proxy'".to_string(),
        )
    })?;
    if personality.is_some() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "personality is not supported for subagents; subagents must be personality-free"
                .to_string(),
        ));
    }
    if normalized == ROLE_MODEL_PROXY && !skills.is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "model-proxy entries cannot own skills; only taskable subagents may have fixed skills"
                .to_string(),
        ));
    }
    if model.trim().is_empty() {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "model cannot be empty; use a concrete provider/model, 'auto', or 'commander'"
                .to_string(),
        ));
    }
    if normalized == ROLE_MODEL_PROXY && is_model_mode(model) {
        return Err((
            axum::http::StatusCode::BAD_REQUEST,
            "model-proxy entries require a concrete provider/model, not 'auto' or 'commander'"
                .to_string(),
        ));
    }
    Ok(())
}

pub async fn list_sub_agents(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::agents::list_sub_agents(&state.db) {
        Ok(agents) => {
            let session_counts =
                ironclad_db::agents::list_session_counts_by_agent(&state.db).unwrap_or_default();
            let items: Vec<serde_json::Value> = agents
                .into_iter()
                .map(|a| {
                    let normalized_role = normalize_role(&a.role).unwrap_or(ROLE_SUBAGENT);
                    let skills = a
                        .skills_json
                        .as_deref()
                        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                        .unwrap_or_default();
                    let session_count = session_counts
                        .get(&a.name)
                        .copied()
                        .unwrap_or(a.session_count);
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "display_name": a.display_name,
                        "model": a.model,
                        "role": normalized_role,
                        "description": a.description,
                        "skills": skills,
                        "enabled": a.enabled,
                        "session_count": session_count,
                    })
                })
                .collect();
            let count = items.len();
            Ok(axum::Json(
                serde_json::json!({ "agents": items, "count": count }),
            ))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn create_sub_agent(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateSubAgentRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
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
                let _ = state.registry.register(config).await;
                let _ = state.registry.start_agent(&agent.name).await;
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
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
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
            let _ = state.registry.register(config).await;
        }
        let _ = state.registry.start_agent(&updated.name).await;
    } else {
        let _ = state.registry.stop_agent(&updated.name).await;
        let _ = state.registry.unregister(&updated.name).await;
    }

    Ok(axum::Json(serde_json::json!({
        "updated": true,
        "name": name,
    })))
}

pub async fn delete_sub_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::agents::delete_sub_agent(&state.db, &name) {
        Ok(true) => {
            let _ = state.registry.stop_agent(&name).await;
            let _ = state.registry.unregister(&name).await;
            Ok(axum::Json(
                serde_json::json!({ "deleted": true, "name": name }),
            ))
        }
        Ok(false) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("sub-agent '{name}' not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn toggle_sub_agent(
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
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
            let _ = state.registry.register(config).await;
        }
        let _ = state.registry.start_agent(&updated.name).await;
    } else {
        let _ = state.registry.stop_agent(&updated.name).await;
        let _ = state.registry.unregister(&updated.name).await;
    }

    Ok(axum::Json(serde_json::json!({
        "name": name,
        "enabled": new_enabled,
    })))
}
