use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde_json::Value;

use super::{AppState, internal_err};

pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::skills::list_skills(&state.db) {
        Ok(skills) => {
            let items: Vec<Value> = skills
                .into_iter()
                .map(|s| {
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "kind": s.kind,
                        "description": s.description,
                        "source_path": s.source_path,
                        "enabled": s.enabled,
                        "last_loaded_at": s.last_loaded_at,
                        "created_at": s.created_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "skills": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_skill(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match ironclad_db::skills::get_skill(&state.db, &id) {
        Ok(Some(s)) => Ok(axum::Json(serde_json::json!({
            "id": s.id,
            "name": s.name,
            "kind": s.kind,
            "description": s.description,
            "source_path": s.source_path,
            "content_hash": s.content_hash,
            "triggers_json": s.triggers_json,
            "tool_chain_json": s.tool_chain_json,
            "policy_overrides_json": s.policy_overrides_json,
            "script_path": s.script_path,
            "enabled": s.enabled,
            "last_loaded_at": s.last_loaded_at,
            "created_at": s.created_at,
        }))),
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn reload_skills(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    let config = state.config.read().await;
    let skills_dir = config.skills.skills_dir.clone();
    drop(config);

    let loaded = ironclad_agent::skills::SkillLoader::load_from_dir(&skills_dir)
        .map_err(|e| internal_err(&e))?;

    let mut added = 0u32;
    let mut updated = 0u32;

    for skill in &loaded {
        let name = skill.name();
        let hash = skill.hash();
        let kind = match skill {
            ironclad_agent::skills::LoadedSkill::Structured(_, _) => "structured",
            ironclad_agent::skills::LoadedSkill::Instruction(_, _) => "instruction",
        };
        let triggers = serde_json::to_string(skill.triggers()).ok();
        let source = skills_dir.join(name).to_string_lossy().to_string();

        let existing = ironclad_db::skills::list_skills(&state.db)
            .unwrap_or_default()
            .into_iter()
            .find(|s| s.name == name);

        if let Some(existing) = existing {
            if existing.content_hash != hash {
                if let Err(e) = ironclad_db::skills::update_skill(
                    &state.db,
                    &existing.id,
                    hash,
                    triggers.as_deref(),
                    None,
                ) {
                    tracing::warn!(error = %e, skill = name, "skill sync: update_skill failed");
                }
                updated += 1;
            }
        } else {
            if let Err(e) = ironclad_db::skills::register_skill(
                &state.db,
                name,
                kind,
                None,
                &source,
                hash,
                triggers.as_deref(),
                None,
                None,
                None,
            ) {
                tracing::warn!(error = %e, skill = name, "skill sync: register_skill failed");
            }
            added += 1;
        }
    }

    Ok(axum::Json(serde_json::json!({
        "reloaded": true,
        "scanned": loaded.len(),
        "added": added,
        "updated": updated,
    })))
}

pub async fn toggle_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::skills::toggle_skill_enabled(&state.db, &id) {
        Ok(Some(new_enabled)) => Ok(axum::Json(serde_json::json!({
            "id": id,
            "enabled": new_enabled,
        }))),
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}
