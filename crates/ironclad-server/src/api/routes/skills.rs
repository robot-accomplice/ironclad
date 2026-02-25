use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde_json::Value;
use std::collections::HashSet;
use std::path::Path as FsPath;

use super::{AppState, internal_err};

struct BuiltinSkillDef {
    name: &'static str,
    description: &'static str,
}

const BUILTIN_SKILLS: &[BuiltinSkillDef] = &[
    BuiltinSkillDef {
        name: "context-continuity",
        description: "Preserve continuity across sessions and long-running workflows.",
    },
    BuiltinSkillDef {
        name: "conway-security",
        description: "Security guardrails for high-impact infrastructure workflows.",
    },
    BuiltinSkillDef {
        name: "ethereum-funding",
        description: "Operational treasury and Ethereum funding workflows.",
    },
    BuiltinSkillDef {
        name: "himalaya-email",
        description: "CLI-based email operations through a local mail bridge.",
    },
    BuiltinSkillDef {
        name: "knowledge-management",
        description: "Knowledge capture, curation, and retrieval conventions.",
    },
    BuiltinSkillDef {
        name: "local-subagents",
        description: "Subagent orchestration for parallelized task execution.",
    },
    BuiltinSkillDef {
        name: "model-management",
        description: "Model routing and fallback strategy management.",
    },
    BuiltinSkillDef {
        name: "obsidian-vault",
        description: "Obsidian-backed knowledge workflows and synchronization.",
    },
    BuiltinSkillDef {
        name: "scope-cli",
        description: "Scope and boundary management for CLI-driven workflows.",
    },
    BuiltinSkillDef {
        name: "search-management",
        description: "Search and retrieval strategy management for investigations.",
    },
    BuiltinSkillDef {
        name: "self-diagnostics",
        description: "Runtime diagnostics and self-healing operational checks.",
    },
    BuiltinSkillDef {
        name: "self-funding",
        description: "Autonomous funding and sustainability operational workflows.",
    },
    BuiltinSkillDef {
        name: "session-bloat-prevention",
        description: "Context-budget controls to prevent session bloat.",
    },
    BuiltinSkillDef {
        name: "supervisor-protocol",
        description: "Supervisor and delegation protocol for specialist execution.",
    },
];

fn is_builtin_skill_name(name: &str) -> bool {
    BUILTIN_SKILLS
        .iter()
        .any(|skill| skill.name.eq_ignore_ascii_case(name))
}

fn is_builtin_skill(s: &ironclad_db::skills::SkillRecord) -> bool {
    s.kind.eq_ignore_ascii_case("builtin") || is_builtin_skill_name(&s.name)
}

pub async fn list_skills(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::skills::list_skills(&state.db) {
        Ok(skills) => {
            let mut items: Vec<Value> = skills
                .into_iter()
                .map(|s| {
                    let built_in = is_builtin_skill(&s);
                    serde_json::json!({
                        "id": s.id,
                        "name": s.name,
                        "kind": s.kind,
                        "description": s.description,
                        "source_path": s.source_path,
                        "risk_level": s.risk_level,
                        "enabled": s.enabled || built_in,
                        "built_in": built_in,
                        "last_loaded_at": s.last_loaded_at,
                        "created_at": s.created_at,
                    })
                })
                .collect();
            let seen: HashSet<String> = items
                .iter()
                .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                .map(|name| name.to_ascii_lowercase())
                .collect();
            for built_in in BUILTIN_SKILLS {
                if seen.contains(&built_in.name.to_ascii_lowercase()) {
                    continue;
                }
                items.push(serde_json::json!({
                    "id": format!("builtin:{}", built_in.name),
                    "name": built_in.name,
                    "kind": "builtin",
                    "description": built_in.description,
                    "source_path": Value::Null,
                    "risk_level": "Caution",
                    "enabled": true,
                    "built_in": true,
                    "last_loaded_at": Value::Null,
                    "created_at": Value::Null,
                }));
            }
            Ok(axum::Json(serde_json::json!({ "skills": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_skill(State(state): State<AppState>, Path(id): Path<String>) -> impl IntoResponse {
    match ironclad_db::skills::get_skill(&state.db, &id) {
        Ok(Some(s)) => {
            let built_in = is_builtin_skill(&s);
            Ok(axum::Json(serde_json::json!({
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
                "risk_level": s.risk_level,
                "enabled": s.enabled || built_in,
                "built_in": built_in,
                "last_loaded_at": s.last_loaded_at,
                "created_at": s.created_at,
            })))
        }
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
    fn risk_level_str(r: ironclad_core::RiskLevel) -> &'static str {
        match r {
            ironclad_core::RiskLevel::Safe => "Safe",
            ironclad_core::RiskLevel::Caution => "Caution",
            ironclad_core::RiskLevel::Dangerous => "Dangerous",
            ironclad_core::RiskLevel::Forbidden => "Forbidden",
        }
    }
    fn normalize_script_path(base: &FsPath, raw: &FsPath) -> String {
        let candidate = if raw.is_absolute() {
            raw.to_path_buf()
        } else {
            base.join(raw)
        };
        std::fs::canonicalize(&candidate)
            .unwrap_or(candidate)
            .to_string_lossy()
            .to_string()
    }

    let config = state.config.read().await;
    let skills_dir = config.skills.skills_dir.clone();
    drop(config);

    let loaded = ironclad_agent::skills::SkillLoader::load_from_dir(&skills_dir)
        .map_err(|e| internal_err(&e))?;

    let mut added = 0u32;
    let mut updated = 0u32;

    let existing_by_name: std::collections::HashMap<String, ironclad_db::skills::SkillRecord> =
        ironclad_db::skills::list_skills(&state.db)
            .unwrap_or_default()
            .into_iter()
            .map(|s| (s.name.clone(), s))
            .collect();

    for skill in &loaded {
        let name = skill.name();
        let hash = skill.hash();
        let kind = match skill {
            ironclad_agent::skills::LoadedSkill::Structured(_, _, _) => "structured",
            ironclad_agent::skills::LoadedSkill::Instruction(_, _, _) => "instruction",
        };
        let triggers = serde_json::to_string(skill.triggers()).ok();
        let source = skill.source_path().to_string_lossy().to_string();
        let desc = skill.description();
        let (tool_chain_json, policy_overrides_json, script_path, risk_level) =
            if let Some(manifest) = skill.structured_manifest() {
                let tool_chain_json = manifest
                    .tool_chain
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok());
                let policy_overrides_json = manifest
                    .policy_overrides
                    .as_ref()
                    .and_then(|v| serde_json::to_string(v).ok());
                let base = skill.source_path().parent().unwrap_or(skills_dir.as_path());
                let script_path = manifest
                    .script_path
                    .as_ref()
                    .map(|p| normalize_script_path(base, p));
                (
                    tool_chain_json,
                    policy_overrides_json,
                    script_path,
                    risk_level_str(manifest.risk_level).to_string(),
                )
            } else {
                (None, None, None, "Caution".to_string())
            };

        let existing = existing_by_name.get(name);

        if let Some(existing) = existing {
            if existing.content_hash != hash
                || existing.triggers_json.as_deref() != triggers.as_deref()
                || existing.tool_chain_json.as_deref() != tool_chain_json.as_deref()
                || existing.policy_overrides_json.as_deref() != policy_overrides_json.as_deref()
                || existing.script_path.as_deref() != script_path.as_deref()
                || existing.source_path != source
                || existing.risk_level != risk_level
            {
                if let Err(e) = ironclad_db::skills::update_skill_full(
                    &state.db,
                    &existing.id,
                    hash,
                    triggers.as_deref(),
                    tool_chain_json.as_deref(),
                    policy_overrides_json.as_deref(),
                    script_path.as_deref(),
                    &source,
                    &risk_level,
                ) {
                    tracing::warn!(error = %e, skill = name, "skill sync: update_skill failed");
                }
                updated += 1;
            }
        } else {
            if let Err(e) = ironclad_db::skills::register_skill_full(
                &state.db,
                name,
                kind,
                desc,
                &source,
                hash,
                triggers.as_deref(),
                tool_chain_json.as_deref(),
                policy_overrides_json.as_deref(),
                script_path.as_deref(),
                &risk_level,
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
    let existing = ironclad_db::skills::get_skill(&state.db, &id).map_err(|e| internal_err(&e))?;
    if let Some(s) = existing.as_ref()
        && is_builtin_skill(s)
    {
        return Err((
            axum::http::StatusCode::FORBIDDEN,
            format!("skill {} is built-in and cannot be disabled", s.name),
        ));
    }
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

pub async fn delete_skill(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::skills::get_skill(&state.db, &id) {
        Ok(Some(skill)) => {
            if is_builtin_skill(&skill) {
                return Err((
                    axum::http::StatusCode::FORBIDDEN,
                    format!("skill {} is built-in and cannot be deleted", skill.name),
                ));
            }
            ironclad_db::skills::delete_skill(&state.db, &id).map_err(|e| internal_err(&e))?;
            Ok(axum::Json(serde_json::json!({
                "id": id,
                "name": skill.name,
                "deleted": true,
            })))
        }
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("skill {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}
