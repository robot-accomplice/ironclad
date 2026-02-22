use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;

use super::{AppState, internal_err};

#[derive(Deserialize)]
pub struct CreateSubAgentRequest {
    pub name: String,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub model: String,
    #[serde(default = "default_role")]
    pub role: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_role() -> String {
    "specialist".into()
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
    pub enabled: Option<bool>,
}

pub async fn list_sub_agents(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::agents::list_sub_agents(&state.db) {
        Ok(agents) => {
            let items: Vec<serde_json::Value> = agents
                .into_iter()
                .map(|a| {
                    serde_json::json!({
                        "id": a.id,
                        "name": a.name,
                        "display_name": a.display_name,
                        "model": a.model,
                        "role": a.role,
                        "description": a.description,
                        "enabled": a.enabled,
                        "session_count": a.session_count,
                    })
                })
                .collect();
            let count = items.len();
            Ok(axum::Json(serde_json::json!({ "agents": items, "count": count })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn create_sub_agent(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateSubAgentRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
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
        model: body.model,
        role: body.role,
        description: body.description,
        skills_json: None,
        enabled: body.enabled,
        session_count: 0,
    };

    match ironclad_db::agents::upsert_sub_agent(&state.db, &agent) {
        Ok(()) => {
            if agent.enabled {
                let config = ironclad_agent::subagents::AgentInstanceConfig {
                    id: agent.name.clone(),
                    name: agent.display_name.clone().unwrap_or_else(|| agent.name.clone()),
                    model: agent.model.clone(),
                    skills: vec![],
                    allowed_subagents: vec![],
                    max_concurrent: 4,
                };
                let _ = state.registry.register(config).await;
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
    let agents = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| internal_err(&e))?;

    let existing = agents.iter().find(|a| a.name == name).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("sub-agent '{name}' not found"),
        )
    })?;

    let updated = ironclad_db::agents::SubAgentRow {
        id: existing.id.clone(),
        name: existing.name.clone(),
        display_name: body.display_name.or(existing.display_name.clone()),
        model: body.model.unwrap_or_else(|| existing.model.clone()),
        role: body.role.unwrap_or_else(|| existing.role.clone()),
        description: body.description.or(existing.description.clone()),
        skills_json: existing.skills_json.clone(),
        enabled: body.enabled.unwrap_or(existing.enabled),
        session_count: existing.session_count,
    };

    ironclad_db::agents::upsert_sub_agent(&state.db, &updated)
        .map_err(|e| internal_err(&e))?;

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
        Ok(true) => Ok(axum::Json(
            serde_json::json!({ "deleted": true, "name": name }),
        )),
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
    let agents = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| internal_err(&e))?;

    let existing = agents.iter().find(|a| a.name == name).ok_or_else(|| {
        (
            axum::http::StatusCode::NOT_FOUND,
            format!("sub-agent '{name}' not found"),
        )
    })?;

    let new_enabled = !existing.enabled;
    let mut updated = existing.clone();
    updated.enabled = new_enabled;

    ironclad_db::agents::upsert_sub_agent(&state.db, &updated)
        .map_err(|e| internal_err(&e))?;

    Ok(axum::Json(serde_json::json!({
        "name": name,
        "enabled": new_enabled,
    })))
}
