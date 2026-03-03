//! Virtual orchestration tools — enable the orchestrator agent to create,
//! configure, and manage subagents at runtime via tool calls.
//!
//! These tools mirror the subagent CRUD API (`/api/subagents`) but are callable
//! from within the agent inference loop, allowing the primary agent to
//! autonomously compose its own team of specialists.

use super::AppState;

/// Returns `true` for tool names that are orchestration management tools.
pub(super) fn is_virtual_orchestration_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.trim().to_ascii_lowercase().as_str(),
        "compose-subagent"
            | "compose_subagent"
            | "update-subagent-skills"
            | "update_subagent_skills"
            | "list-subagent-roster"
            | "list_subagent_roster"
            | "list-available-skills"
            | "list_available_skills"
            | "remove-subagent"
            | "remove_subagent"
    )
}

/// Execute a virtual orchestration tool call.
///
/// All tools apply the same policy + approval checks as delegation tools,
/// then dispatch to the appropriate subagent management operation.
///
/// Orchestration tools are sensitive roster-management operations.
/// They run under the caller's resolved authority and must never be
/// privilege-escalated implicitly.
pub(super) async fn execute_virtual_orchestration_tool(
    state: &AppState,
    tool_name: &str,
    params: &serde_json::Value,
    turn_id: &str,
    authority: ironclad_core::InputAuthority,
    tier: ironclad_core::SurvivalTier,
) -> Result<String, String> {
    // ── policy gate ──────────────────────────────────────────────
    let policy_result = super::check_tool_policy(
        &state.policy_engine,
        tool_name,
        params,
        authority,
        tier,
        ironclad_core::RiskLevel::Caution,
    );
    let (decision_str, rule_name, reason) = match &policy_result {
        Ok(()) => ("allow".to_string(), None, None),
        Err(super::super::JsonError(_status, msg)) => (
            "deny".to_string(),
            Some("policy_engine"),
            Some(msg.as_str()),
        ),
    };
    ironclad_db::policy::record_policy_decision(
        &state.db,
        Some(turn_id),
        tool_name,
        &decision_str,
        rule_name,
        reason,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record policy decision"))
    .ok();
    if let Err(super::super::JsonError(_status, msg)) = policy_result {
        return Err(format!("Policy denied: {msg}"));
    }

    // ── approval gate ────────────────────────────────────────────
    match state.approvals.check_tool(tool_name) {
        Ok(ironclad_agent::approvals::ToolClassification::Gated) => {
            let request = state
                .approvals
                .request_approval(tool_name, &params.to_string(), Some(turn_id))
                .map_err(|e| format!("Approval error: {e}"))?;
            ironclad_db::approvals::record_approval_request(
                &state.db,
                &request.id,
                &request.tool_name,
                &request.tool_input,
                request.session_id.as_deref(),
                "pending",
                &request.timeout_at.to_rfc3339(),
            )
            .inspect_err(|e| tracing::warn!(error = %e, "failed to persist approval request"))
            .ok();
            return Err(format!(
                "Tool '{tool_name}' requires approval (request: {})",
                request.id
            ));
        }
        Err(e) => return Err(format!("Tool blocked: {e}")),
        Ok(_) => {}
    }

    // ── dispatch ─────────────────────────────────────────────────
    let action = tool_name.trim().to_ascii_lowercase();
    match action.as_str() {
        "compose-subagent" | "compose_subagent" => compose_subagent(state, params).await,
        "update-subagent-skills" | "update_subagent_skills" => {
            update_subagent_skills(state, params).await
        }
        "list-subagent-roster" | "list_subagent_roster" => list_subagent_roster(state).await,
        "list-available-skills" | "list_available_skills" => {
            list_available_skills(state, params).await
        }
        "remove-subagent" | "remove_subagent" => remove_subagent(state, params).await,
        _ => Err(format!("unrecognized orchestration tool: {tool_name}")),
    }
}

// ── compose-subagent ─────────────────────────────────────────────

async fn compose_subagent(state: &AppState, params: &serde_json::Value) -> Result<String, String> {
    use crate::api::routes::subagents::{
        ROLE_SUBAGENT, normalize_fallback_models, normalize_model_input, normalize_role,
        normalize_skills, resolve_taskable_subagent_runtime_model, validate_subagent_contract,
        validate_subagent_name,
    };

    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            "compose-subagent requires `name` (alphanumeric, hyphens, underscores)".to_string()
        })?;

    validate_subagent_name(&name).map_err(|e| format!("invalid subagent name: {}", e.1))?;

    // Check for existing agent with same name.
    let existing = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| format!("failed to query sub-agents: {e}"))?;
    if existing.iter().any(|a| a.name.eq_ignore_ascii_case(&name)) {
        return Err(format!(
            "subagent '{name}' already exists; use update-subagent-skills to modify it"
        ));
    }

    let model_raw = params
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("auto");
    let model = normalize_model_input(model_raw);

    let description = params
        .get("description")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let skills_raw: Vec<String> = match params.get("skills") {
        Some(v) if v.is_array() => v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };
    let skills = normalize_skills(&skills_raw);
    let fallback_models_raw: Vec<String> = match params.get("fallback_models") {
        Some(v) if v.is_array() => v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => vec![],
    };
    let fallback_models = normalize_fallback_models(&fallback_models_raw, &model);

    let role = params
        .get("role")
        .and_then(|v| v.as_str())
        .unwrap_or(ROLE_SUBAGENT);
    let normalized_role = normalize_role(role)
        .ok_or_else(|| "role must be 'subagent' or 'model-proxy'".to_string())?;

    validate_subagent_contract(normalized_role, &model, &skills, None)
        .map_err(|e| format!("contract violation: {}", e.1))?;

    // Generate display name from hyphenated name.
    let display_name = params
        .get("display_name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            name.split('-')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        None => String::new(),
                        Some(f) => f.to_uppercase().to_string() + c.as_str(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        });

    let agent = ironclad_db::agents::SubAgentRow {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.clone(),
        display_name: Some(display_name.clone()),
        model: model.clone(),
        fallback_models_json: Some(
            serde_json::to_string(&fallback_models).unwrap_or_else(|_| "[]".to_string()),
        ),
        role: normalized_role.to_string(),
        description: description.clone(),
        skills_json: Some(serde_json::to_string(&skills).unwrap_or_else(|_| "[]".to_string())),
        enabled: true,
        session_count: 0,
    };

    ironclad_db::agents::upsert_sub_agent(&state.db, &agent)
        .map_err(|e| format!("failed to persist subagent: {e}"))?;

    // Register and start in runtime.
    if normalized_role == ROLE_SUBAGENT {
        let config = ironclad_agent::subagents::AgentInstanceConfig {
            id: name.clone(),
            name: display_name.clone(),
            model: resolve_taskable_subagent_runtime_model(state, &model).await,
            skills: skills.clone(),
            allowed_subagents: vec![],
            max_concurrent: 4,
        };
        if let Err(e) = state.registry.register(config).await {
            tracing::error!(agent = %name, error = %e, "orchestration: failed to register sub-agent");
        }
        if let Err(e) = state.registry.start_agent(&name).await {
            tracing::error!(agent = %name, error = %e, "orchestration: failed to start sub-agent");
        }
    }

    let skills_label = if skills.is_empty() {
        "(none)".to_string()
    } else {
        skills.join(", ")
    };
    Ok(format!(
        "created subagent '{name}' (display: {display_name}) model={model} fallback_models={} role={normalized_role} skills=[{skills_label}] enabled=true",
        serde_json::to_string(&fallback_models).unwrap_or_else(|_| "[]".to_string())
    ))
}

// ── update-subagent-skills ───────────────────────────────────────

async fn update_subagent_skills(
    state: &AppState,
    params: &serde_json::Value,
) -> Result<String, String> {
    use crate::api::routes::subagents::normalize_skills;

    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "update-subagent-skills requires `name`".to_string())?;

    let agents = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| format!("failed to query sub-agents: {e}"))?;
    let existing = agents
        .iter()
        .find(|a| a.name.eq_ignore_ascii_case(&name))
        .ok_or_else(|| format!("subagent '{name}' not found"))?;

    let new_skills_raw: Vec<String> = match params.get("skills") {
        Some(v) if v.is_array() => v
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
        _ => return Err("update-subagent-skills requires `skills` array".to_string()),
    };

    let mode = params
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("replace");

    let merged_skills = match mode {
        "append" => {
            let mut current: Vec<String> = existing
                .skills_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok())
                .unwrap_or_default();
            current.extend(new_skills_raw);
            normalize_skills(&current)
        }
        _ => normalize_skills(&new_skills_raw), // "replace" or default
    };

    let mut updated = existing.clone();
    updated.skills_json =
        Some(serde_json::to_string(&merged_skills).unwrap_or_else(|_| "[]".to_string()));

    ironclad_db::agents::upsert_sub_agent(&state.db, &updated)
        .map_err(|e| format!("failed to update subagent skills: {e}"))?;

    let skills_label = if merged_skills.is_empty() {
        "(none)".to_string()
    } else {
        merged_skills.join(", ")
    };
    Ok(format!(
        "updated subagent '{name}' skills=[{skills_label}] (mode={mode})"
    ))
}

// ── list-subagent-roster ─────────────────────────────────────────

async fn list_subagent_roster(state: &AppState) -> Result<String, String> {
    use crate::api::routes::subagents::{ROLE_MODEL_PROXY, parse_fallback_models_json};

    let agents = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| format!("failed to query sub-agents: {e}"))?;

    if agents.is_empty() {
        return Ok("no subagents configured".to_string());
    }

    let runtime = state.registry.list_agents().await;
    let runtime_by_name: std::collections::HashMap<
        String,
        ironclad_agent::subagents::AgentInstance,
    > = runtime
        .into_iter()
        .map(|a| (a.id.to_ascii_lowercase(), a))
        .collect();

    let mut lines = Vec::new();
    let mut taskable_count = 0usize;
    let mut proxy_count = 0usize;

    for a in &agents {
        let is_proxy = a.role.eq_ignore_ascii_case(ROLE_MODEL_PROXY);
        if is_proxy {
            proxy_count += 1;
        }
        let skills: Vec<String> = a
            .skills_json
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or_default();

        let runtime_state = if is_proxy {
            "n/a".to_string()
        } else if let Some(inst) = runtime_by_name.get(&a.name.to_ascii_lowercase()) {
            format!("{:?}", inst.state).to_ascii_lowercase()
        } else if a.enabled {
            "booting".to_string()
        } else {
            "stopped".to_string()
        };

        let taskable = a.enabled && runtime_state == "running" && !is_proxy;
        if taskable {
            taskable_count += 1;
        }

        let skills_label = if skills.is_empty() {
            "(none)".to_string()
        } else {
            skills.join(", ")
        };
        let fallback_models = parse_fallback_models_json(a.fallback_models_json.as_deref());
        let fallback_label = if fallback_models.is_empty() {
            "[]".to_string()
        } else {
            format!("[{}]", fallback_models.join(", "))
        };

        lines.push(format!(
            "- {} [{}] model={} fallbacks={} skills=[{}] enabled={} runtime={}{}",
            a.name,
            a.role,
            a.model,
            fallback_label,
            skills_label,
            a.enabled,
            runtime_state,
            if taskable { " ★taskable" } else { "" },
        ));
    }

    Ok(format!(
        "subagent roster ({} total, {} taskable, {} proxies):\n{}",
        agents.len(),
        taskable_count,
        proxy_count,
        lines.join("\n"),
    ))
}

// ── list-available-skills ────────────────────────────────────────

async fn list_available_skills(
    state: &AppState,
    params: &serde_json::Value,
) -> Result<String, String> {
    let keyword = params
        .get("keyword")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());

    let skills = if let Some(ref kw) = keyword {
        ironclad_db::skills::find_by_trigger(&state.db, kw)
            .map_err(|e| format!("failed to search skills: {e}"))?
    } else {
        ironclad_db::skills::list_skills(&state.db)
            .map_err(|e| format!("failed to list skills: {e}"))?
    };

    if skills.is_empty() {
        return Ok(if let Some(kw) = keyword {
            format!("no skills match keyword '{kw}'")
        } else {
            "no skills registered in workspace catalog".to_string()
        });
    }

    let mut lines: Vec<String> = skills
        .iter()
        .map(|s| {
            let desc = s.description.as_deref().unwrap_or("(no description)");
            let status = if s.enabled { "enabled" } else { "disabled" };
            format!(
                "- {} [{}] risk={} {}: {}",
                s.name, s.kind, s.risk_level, status, desc
            )
        })
        .collect();

    lines.insert(
        0,
        format!("workspace skill catalog ({} skills):", skills.len()),
    );

    Ok(lines.join("\n"))
}

// ── remove-subagent ──────────────────────────────────────────────

async fn remove_subagent(state: &AppState, params: &serde_json::Value) -> Result<String, String> {
    let name = params
        .get("name")
        .and_then(|v| v.as_str())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "remove-subagent requires `name`".to_string())?;

    let deleted = ironclad_db::agents::delete_sub_agent(&state.db, &name)
        .map_err(|e| format!("failed to delete subagent: {e}"))?;

    if !deleted {
        return Err(format!("subagent '{name}' not found"));
    }

    // Tear down from runtime.
    if let Err(e) = state.registry.stop_agent(&name).await {
        tracing::warn!(agent = %name, error = %e, "orchestration: failed to stop agent during removal");
    }
    state.registry.unregister(&name).await;

    Ok(format!("removed subagent '{name}'"))
}
