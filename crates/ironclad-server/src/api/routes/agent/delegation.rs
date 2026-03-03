//! Virtual subagent delegation tool execution.

use std::collections::{HashMap, HashSet};

use super::super::JsonError;
use super::AppState;

pub(super) fn is_virtual_delegation_tool(tool_name: &str) -> bool {
    matches!(
        tool_name.trim().to_ascii_lowercase().as_str(),
        "orchestrate-subagents"
            | "orchestrate_subagents"
            | "assign-tasks"
            | "assign_tasks"
            | "delegate-subagent"
            | "delegate_subagent"
            | "select-subagent-model"
            | "select_subagent_model"
    )
}

async fn resolve_subagent_runtime_model(
    state: &AppState,
    subagent: &ironclad_db::agents::SubAgentRow,
    task: &str,
) -> String {
    let configured = subagent.model.trim();
    if configured.eq_ignore_ascii_case("auto") {
        return super::select_routed_model(state, task).await;
    }
    if configured.eq_ignore_ascii_case("orchestrator") {
        let llm = state.llm.read().await;
        return llm.router.select_model().to_string();
    }
    if configured.is_empty() {
        return super::select_routed_model(state, task).await;
    }
    configured.to_string()
}

fn pick_running_subagent<'a>(
    task: &str,
    specialist_hint: Option<&str>,
    taskable_subagents: &'a [ironclad_db::agents::SubAgentRow],
    runtime_by_name: &HashMap<String, ironclad_agent::subagents::AgentInstance>,
) -> Option<&'a ironclad_db::agents::SubAgentRow> {
    let running: Vec<&ironclad_db::agents::SubAgentRow> = taskable_subagents
        .iter()
        .filter(|sa| {
            runtime_by_name
                .get(&sa.name.to_ascii_lowercase())
                .is_some_and(|inst| inst.state == ironclad_agent::subagents::AgentRunState::Running)
        })
        .collect();
    if running.is_empty() {
        return None;
    }

    if let Some(hint_raw) = specialist_hint {
        let hint = hint_raw.trim().to_ascii_lowercase();
        if !hint.is_empty()
            && let Some(chosen) = running.iter().find(|sa| {
                sa.name.eq_ignore_ascii_case(&hint)
                    || sa
                        .display_name
                        .as_deref()
                        .is_some_and(|d| d.to_ascii_lowercase().contains(&hint))
            })
        {
            return Some(chosen);
        }
    }

    let required = super::capability_tokens(task);
    let mut scored: Vec<(&ironclad_db::agents::SubAgentRow, usize)> = running
        .iter()
        .map(|sa| {
            let skills = super::parse_skills_json(sa.skills_json.as_deref());
            let skill_tokens: HashSet<String> = skills
                .iter()
                .flat_map(|s| super::capability_tokens(s))
                .collect();
            let overlap = required
                .iter()
                .filter(|tok| skill_tokens.contains(*tok))
                .count();
            (*sa, overlap)
        })
        .collect();
    scored.sort_by_key(|(_, overlap)| std::cmp::Reverse(*overlap));
    scored
        .first()
        .map(|(sa, _)| *sa)
        .or_else(|| running.first().copied())
}

fn timeout_like_error(err: &str) -> bool {
    let e = err.to_ascii_lowercase();
    e.contains("timeout")
        || e.contains("timed out")
        || e.contains("no route to host")
        || e.contains("error sending request")
        || e.contains("connection refused")
}

async fn provider_breaker_blocked(state: &AppState, model: &str) -> bool {
    let provider_prefix = model.split('/').next().unwrap_or("unknown");
    let llm = state.llm.read().await;
    llm.breakers.is_blocked(provider_prefix)
}

async fn select_cloud_rescue_model(
    state: &AppState,
    current_model: &str,
    preferred_fallbacks: &[String],
) -> Option<String> {
    let config = state.config.read().await;
    let mut candidates = Vec::new();
    for m in preferred_fallbacks {
        if !m.trim().is_empty() && !candidates.iter().any(|c: &String| c == m) {
            candidates.push(m.clone());
        }
    }
    for m in &config.models.fallbacks {
        if !m.trim().is_empty() && !candidates.iter().any(|c: &String| c == m) {
            candidates.push(m.clone());
        }
    }
    if !config.models.primary.trim().is_empty()
        && !candidates
            .iter()
            .any(|c: &String| c == &config.models.primary)
    {
        candidates.push(config.models.primary.clone());
    }
    drop(config);

    let llm = state.llm.read().await;
    for candidate in candidates {
        if candidate.eq_ignore_ascii_case(current_model) {
            continue;
        }
        let Some(provider) = llm.providers.get_by_model(&candidate) else {
            continue;
        };
        if provider.is_local {
            continue;
        }
        if llm
            .breakers
            .is_blocked(candidate.split('/').next().unwrap_or("unknown"))
        {
            continue;
        }
        return Some(candidate);
    }
    None
}

pub(super) async fn execute_virtual_subagent_tool_call(
    state: &AppState,
    tool_name: &str,
    params: &serde_json::Value,
    turn_id: &str,
    authority: ironclad_core::InputAuthority,
    tier: ironclad_core::SurvivalTier,
) -> Result<String, String> {
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
        Err(JsonError(_status, msg)) => (
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
    if let Err(JsonError(_status, msg)) = policy_result {
        return Err(format!("Policy denied: {msg}"));
    }

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

    let action = tool_name.trim().to_ascii_lowercase();
    let mut task = params
        .get("task")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("query").and_then(|v| v.as_str()))
        .or_else(|| params.get("prompt").and_then(|v| v.as_str()))
        .unwrap_or("")
        .trim()
        .to_string();
    let specialist_hint = params
        .get("specialist")
        .and_then(|v| v.as_str())
        .or_else(|| params.get("subagent").and_then(|v| v.as_str()));

    let subtasks: Vec<String> = match params.get("subtasks") {
        Some(v) => match v.as_array() {
            Some(arr) => arr
                .iter()
                .filter_map(|v| v.as_str().map(|s| s.trim().to_string()))
                .filter(|s| !s.is_empty())
                .take(6)
                .collect(),
            None => {
                tracing::warn!("delegation 'subtasks' param is not an array, ignoring");
                vec![]
            }
        },
        None => vec![],
    };
    if task.is_empty() && !subtasks.is_empty() {
        task = subtasks.join("; ");
    }
    if task.is_empty() {
        return Err("delegation tool requires `task` (or `subtasks`)".to_string());
    }

    let all_subagents = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| format!("failed to query sub-agents: {e}"))?;
    let taskable_subagents: Vec<ironclad_db::agents::SubAgentRow> = all_subagents
        .into_iter()
        .filter(|sa| !super::is_model_proxy_role(&sa.role) && sa.enabled)
        .collect();
    if taskable_subagents.is_empty() {
        return Err("no enabled taskable subagents are configured".to_string());
    }

    let runtime_by_name: HashMap<String, ironclad_agent::subagents::AgentInstance> = state
        .registry
        .list_agents()
        .await
        .into_iter()
        .map(|a| (a.id.to_ascii_lowercase(), a))
        .collect();

    let booting_count = runtime_by_name
        .values()
        .filter(|a| {
            matches!(
                a.state,
                ironclad_agent::subagents::AgentRunState::Starting
                    | ironclad_agent::subagents::AgentRunState::Idle
            )
        })
        .count();
    let running_count = runtime_by_name
        .values()
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .count();

    if action == "select-subagent-model" || action == "select_subagent_model" {
        let chosen = pick_running_subagent(
            &task,
            specialist_hint,
            &taskable_subagents,
            &runtime_by_name,
        )
        .or_else(|| taskable_subagents.first())
        .ok_or_else(|| "no candidate subagent found for model selection".to_string())?;
        let model = resolve_subagent_runtime_model(state, chosen, &task).await;
        return Ok(format!(
            "selected_subagent={} resolved_model={} running={} booting={}",
            chosen.name, model, running_count, booting_count
        ));
    }

    let chosen = pick_running_subagent(
        &task,
        specialist_hint,
        &taskable_subagents,
        &runtime_by_name,
    )
    .ok_or_else(|| {
        format!(
            "no running taskable subagents are available (running={}, booting={})",
            running_count, booting_count
        )
    })?;
    let model = resolve_subagent_runtime_model(state, chosen, &task).await;
    let preferred_fallbacks = crate::api::routes::subagents::parse_fallback_models_json(
        chosen.fallback_models_json.as_deref(),
    );
    let configured_is_fixed = {
        let raw = chosen.model.trim().to_ascii_lowercase();
        !raw.is_empty() && raw != "auto" && raw != "orchestrator"
    };
    let mut effective_model = model.clone();
    let mut delegation_notes: Vec<String> = Vec::new();
    if configured_is_fixed
        && provider_breaker_blocked(state, &effective_model).await
        && let Some(rescue) =
            select_cloud_rescue_model(state, &effective_model, &preferred_fallbacks).await
    {
        delegation_notes.push(format!(
            "breaker-open guardrail rerouted fixed model from {} to {}",
            effective_model, rescue
        ));
        effective_model = rescue;
    }

    let task_list = if subtasks.is_empty() {
        vec![task.clone()]
    } else {
        subtasks
    };
    let mut outputs = Vec::new();
    for (idx, subtask) in task_list.iter().enumerate() {
        let skills = super::parse_skills_json(chosen.skills_json.as_deref());
        let system_prompt = format!(
            "You are specialist subagent `{}`. Skills: {}.\nYou report to the orchestrator. Complete only the assigned task and return concise factual output plus caveats.",
            chosen.name,
            if skills.is_empty() {
                "(none)".to_string()
            } else {
                skills.join(", ")
            }
        );
        let model_for_api = effective_model
            .split_once('/')
            .map(|(_, m)| m)
            .unwrap_or(&effective_model)
            .to_string();
        let req = ironclad_llm::format::UnifiedRequest {
            model: model_for_api,
            messages: vec![
                ironclad_llm::format::UnifiedMessage {
                    role: "system".into(),
                    content: system_prompt,
                    parts: None,
                },
                ironclad_llm::format::UnifiedMessage {
                    role: "user".into(),
                    content: subtask.clone(),
                    parts: None,
                },
            ],
            max_tokens: Some(1200),
            temperature: None,
            system: None,
            quality_target: None,
            tools: vec![],
        };
        let result = match super::infer_with_fallback_with_budget_and_preferred(
            state,
            &req,
            &effective_model,
            super::DELEGATED_INFERENCE_BUDGET,
            &preferred_fallbacks,
        )
        .await
        {
            Ok(r) => r,
            Err(err) => {
                let retry_target = if timeout_like_error(&err) {
                    select_cloud_rescue_model(state, &effective_model, &preferred_fallbacks).await
                } else {
                    None
                };
                if let Some(rescue_model) = retry_target {
                    tracing::warn!(
                        subagent = %chosen.name,
                        from_model = %effective_model,
                        to_model = %rescue_model,
                        error = %err,
                        "delegation timeout guardrail rerouting to cloud model"
                    );
                    delegation_notes.push(format!(
                        "timeout guardrail rerouted delegated subtask from {} to {}",
                        effective_model, rescue_model
                    ));
                    effective_model = rescue_model.clone();
                    let retry_model_for_api = rescue_model
                        .split_once('/')
                        .map(|(_, m)| m)
                        .unwrap_or(&rescue_model)
                        .to_string();
                    let retry_req = ironclad_llm::format::UnifiedRequest {
                        model: retry_model_for_api,
                        messages: req.messages.clone(),
                        max_tokens: req.max_tokens,
                        temperature: req.temperature,
                        system: req.system.clone(),
                        quality_target: req.quality_target,
                        tools: req.tools.clone(),
                    };
                    super::infer_with_fallback_with_budget_and_preferred(
                        state,
                        &retry_req,
                        &rescue_model,
                        super::DELEGATED_INFERENCE_BUDGET,
                        &preferred_fallbacks,
                    )
                    .await?
                } else {
                    return Err(err);
                }
            }
        };
        outputs.push(format!(
            "subtask {} -> {}\n{}",
            idx + 1,
            chosen.name,
            result.content.trim()
        ));
        if action == "assign-tasks" || action == "assign_tasks" {
            // assign-tasks executes one delegated unit per call.
            break;
        }
    }

    Ok(format!(
        "delegated_subagent={} model={} fallback_models={}{}\n{}",
        chosen.name,
        effective_model,
        serde_json::to_string(&preferred_fallbacks).unwrap_or_else(|_| "[]".to_string()),
        if delegation_notes.is_empty() {
            String::new()
        } else {
            format!("\nnotes={}", delegation_notes.join(" | "))
        },
        outputs.join("\n\n")
    ))
}
