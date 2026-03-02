//! Decomposition gate, specialist proposal types, and delegation planning.

use std::collections::HashSet;

use serde_json::json;

use ironclad_agent::orchestration::{OrchestrationPattern, Orchestrator};

use super::AppState;

#[derive(Debug, Clone)]
pub(super) struct SpecialistProposal {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub skills: Vec<String>,
    pub model: String,
}

#[derive(Debug, Clone)]
pub(super) struct DelegationPlan {
    pub subtasks: Vec<String>,
    pub rationale: String,
    pub expected_utility_margin: f64,
}

#[derive(Debug, Clone)]
pub(super) enum DecompositionDecision {
    Centralized {
        rationale: String,
        expected_utility_margin: f64,
    },
    Delegated(DelegationPlan),
    RequiresSpecialistCreation {
        proposal: SpecialistProposal,
        rationale: String,
    },
}

#[derive(Debug, Clone, Default)]
pub(super) struct DelegationProvenance {
    pub subagent_task_started: bool,
    pub subagent_task_completed: bool,
    pub subagent_result_attached: bool,
}

pub(super) fn split_subtasks(input: &str) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for part in input
        .split(&['\n', ';'][..])
        .flat_map(|p| p.split(" then "))
        .flat_map(|p| p.split(" and "))
    {
        let trimmed = part.trim();
        if !trimmed.is_empty() && seen.insert(trimmed.to_string()) {
            out.push(trimmed.to_string());
        }
    }
    out
}

pub(super) fn capability_tokens(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 4)
        .map(|s| s.to_string())
        .collect()
}

pub(super) fn utility_margin_for_delegation(
    complexity_score: f64,
    subtask_count: usize,
    capability_fit_ratio: f64,
) -> f64 {
    let complexity_gain = complexity_score * 0.5;
    let parallel_gain = ((subtask_count.saturating_sub(1)) as f64) * 0.12;
    let fit_gain = capability_fit_ratio * 0.45;
    let orchestration_cost = 0.25 + ((subtask_count as f64) * 0.04);
    complexity_gain + parallel_gain + fit_gain - orchestration_cost
}

pub(super) fn proposal_to_json(
    proposal: &SpecialistProposal,
    rationale: &str,
) -> serde_json::Value {
    json!({
        "name": proposal.name,
        "display_name": proposal.display_name,
        "description": proposal.description,
        "skills": proposal.skills,
        "model": proposal.model,
        "rationale": rationale,
    })
}

pub(super) async fn evaluate_decomposition_gate(
    state: &AppState,
    user_content: &str,
    complexity_score: f64,
) -> DecompositionDecision {
    let cfg = state.config.read().await;
    if !cfg.agent.delegation_enabled {
        return DecompositionDecision::Centralized {
            rationale: "delegation disabled by configuration".to_string(),
            expected_utility_margin: -1.0,
        };
    }
    let min_complexity = cfg.agent.delegation_min_complexity;
    let min_margin = cfg.agent.delegation_min_utility_margin;
    drop(cfg);

    let subtasks = split_subtasks(user_content);
    if subtasks.len() <= 1 || complexity_score < min_complexity {
        return DecompositionDecision::Centralized {
            rationale: "task is single-step or below decomposition complexity threshold"
                .to_string(),
            expected_utility_margin: -0.1,
        };
    }

    let subagents = ironclad_db::agents::list_sub_agents(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to list sub-agents for decomposition"))
        .unwrap_or_default();
    let taskable: Vec<_> = subagents
        .into_iter()
        .filter(|a| !super::is_model_proxy_role(&a.role) && a.enabled)
        .collect();
    if taskable.is_empty() {
        return DecompositionDecision::Centralized {
            rationale: "no enabled taskable specialists available".to_string(),
            expected_utility_margin: -0.3,
        };
    }

    let required = capability_tokens(user_content);
    let mut fit_hits = 0usize;
    for agent in &taskable {
        let skills = super::parse_skills_json(agent.skills_json.as_deref());
        let skill_tokens: HashSet<String> = skills
            .iter()
            .flat_map(|s| capability_tokens(s))
            .collect::<Vec<_>>()
            .into_iter()
            .collect();
        if required.iter().any(|t| skill_tokens.contains(t)) {
            fit_hits += 1;
        }
    }
    let capability_fit_ratio = if taskable.is_empty() {
        0.0
    } else {
        fit_hits as f64 / taskable.len() as f64
    };
    let margin =
        utility_margin_for_delegation(complexity_score, subtasks.len(), capability_fit_ratio);
    if capability_fit_ratio < 0.2 {
        let proposal = SpecialistProposal {
            name: "proposed-specialist".to_string(),
            display_name: "Proposed Specialist".to_string(),
            description: "Auto-proposed specialist for uncovered capability gap".to_string(),
            skills: required.into_iter().take(8).collect(),
            model: "auto".to_string(),
        };
        return DecompositionDecision::RequiresSpecialistCreation {
            proposal,
            rationale:
                "existing specialists do not satisfy required capability fit; proposal required"
                    .to_string(),
        };
    }

    if margin < min_margin {
        return DecompositionDecision::Centralized {
            rationale: format!(
                "delegation utility margin {:.2} below threshold {:.2}",
                margin, min_margin
            ),
            expected_utility_margin: margin,
        };
    }

    DecompositionDecision::Delegated(DelegationPlan {
        subtasks,
        rationale: format!(
            "decomposed into subtasks with estimated delegation margin {:.2}",
            margin
        ),
        expected_utility_margin: margin,
    })
}

pub(super) async fn maybe_handle_specialist_creation_controls(
    state: &AppState,
    session_id: &str,
    user_content: &str,
) -> Option<String> {
    let lower = user_content.to_ascii_lowercase();
    if !(lower.contains("approve specialist")
        || lower.contains("review specialist config")
        || lower.contains("show specialist config")
        || lower.contains("deny specialist creation"))
    {
        return None;
    }

    let proposal = {
        let map = state.pending_specialist_proposals.read().await;
        map.get(session_id).cloned()
    }?;

    if lower.contains("review specialist config") || lower.contains("show specialist config") {
        return Some(format!(
            "Proposed specialist configuration preview:\n\n```json\n{}\n```\n\nReply with `approve specialist creation` to create it, or `deny specialist creation` to keep centralized execution.",
            serde_json::to_string_pretty(&proposal).unwrap_or_else(|_| "{}".to_string())
        ));
    }

    if lower.contains("approve specialist") {
        let name = proposal
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or("proposed-specialist")
            .to_string();
        let display_name = proposal
            .get("display_name")
            .and_then(|v| v.as_str())
            .unwrap_or("Proposed Specialist")
            .to_string();
        let description = proposal
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Auto-created specialist")
            .to_string();
        let skills: Vec<String> = proposal
            .get("skills")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let model = proposal
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("auto")
            .to_string();
        let row = ironclad_db::agents::SubAgentRow {
            id: uuid::Uuid::new_v4().to_string(),
            name: name.clone(),
            display_name: Some(display_name.clone()),
            model: model.clone(),
            role: "subagent".to_string(),
            description: Some(description),
            skills_json: Some(serde_json::to_string(&skills).unwrap_or_else(|_| "[]".to_string())),
            enabled: true,
            session_count: 0,
        };
        if let Err(e) = ironclad_db::agents::upsert_sub_agent(&state.db, &row) {
            return Some(format!("Failed to create specialist: {e}"));
        }
        let config = ironclad_agent::subagents::AgentInstanceConfig {
            id: name.clone(),
            name: display_name,
            model: row.model.clone(),
            skills,
            allowed_subagents: vec![],
            max_concurrent: 4,
        };
        if let Err(e) = state.registry.register(config).await {
            tracing::error!(agent = %name, error = %e, "failed to register specialist in runtime");
        }
        if let Err(e) = state.registry.start_agent(&name).await {
            tracing::error!(agent = %name, error = %e, "failed to start specialist in runtime");
        }
        {
            let mut map = state.pending_specialist_proposals.write().await;
            map.remove(session_id);
        }
        return Some(format!(
            "Approved. Created specialist `{name}`. I can now decompose and delegate this task."
        ));
    }

    if lower.contains("deny specialist creation") {
        {
            let mut map = state.pending_specialist_proposals.write().await;
            map.remove(session_id);
        }
        return Some(
            "Understood. I will keep execution centralized for this task and include rationale."
                .to_string(),
        );
    }

    None
}

/// Outcome of applying a decomposition gate decision.
pub(super) enum DecompositionOutcome {
    /// Task handled centrally -- no delegation.
    Centralized,
    /// A specialist creation proposal was stored; caller must relay `prompt` to the user
    /// and perform its own early-return (the return type differs between API and channel paths).
    SpecialistProposalPending { prompt: String },
    /// Task delegated via orchestrator; caller should thread `workflow_note` into context.
    Delegated { workflow_note: String },
}

/// Apply a decomposition gate decision: store proposals, orchestrate workflows, or log
/// centralized execution.  Returns an outcome the caller dispatches pathway-specifically.
pub(super) async fn apply_decomposition_decision(
    state: &AppState,
    gate_decision: &DecompositionDecision,
    session_id: &str,
    pathway_label: &str,
) -> DecompositionOutcome {
    match gate_decision {
        DecompositionDecision::RequiresSpecialistCreation {
            proposal,
            rationale,
        } => {
            let payload = proposal_to_json(proposal, rationale);
            {
                let mut pending = state.pending_specialist_proposals.write().await;
                pending.insert(session_id.to_string(), payload);
            }
            let prompt = format!(
                "I identified a capability gap and can create a new specialist with your approval.\n\n\
                 Proposed: `{}`\nRationale: {}\n\n\
                 Reply with:\n\
                 - `review specialist config` to inspect full config\n\
                 - `approve specialist creation` to create it\n\
                 - `deny specialist creation` to continue with main-agent execution",
                proposal.name, rationale
            );
            DecompositionOutcome::SpecialistProposalPending { prompt }
        }
        DecompositionDecision::Centralized {
            rationale,
            expected_utility_margin,
        } => {
            tracing::info!(
                decision = "centralized",
                pathway = %pathway_label,
                rationale = %rationale,
                expected_utility_margin = *expected_utility_margin,
                "decomposition gate decision"
            );
            DecompositionOutcome::Centralized
        }
        DecompositionDecision::Delegated(plan) => {
            let mut orch = Orchestrator::new();
            let wf_input = plan
                .subtasks
                .iter()
                .map(|s| (s.clone(), capability_tokens(s)))
                .collect::<Vec<_>>();
            let wf_id =
                orch.create_workflow(pathway_label, OrchestrationPattern::Parallel, wf_input);
            let available_agents = ironclad_db::agents::list_sub_agents(&state.db)
                .inspect_err(
                    |e| tracing::error!(error = %e, "failed to list sub-agents for workflow"),
                )
                .unwrap_or_default()
                .into_iter()
                .filter(|a| !super::is_model_proxy_role(&a.role) && a.enabled)
                .map(|a| (a.name, super::parse_skills_json(a.skills_json.as_deref())))
                .collect::<Vec<_>>();
            let matches = orch
                .match_capabilities(&wf_id, &available_agents)
                .unwrap_or_default();
            for (task_id, agent_id) in &matches {
                if let Err(e) = orch.assign_agent(&wf_id, task_id, agent_id) {
                    tracing::error!(
                        workflow = %wf_id,
                        task = %task_id,
                        agent = %agent_id,
                        error = %e,
                        "failed to assign agent to workflow task"
                    );
                }
            }
            let assignments = matches
                .iter()
                .map(|(task, agent)| format!("{task}->{agent}"))
                .collect::<Vec<_>>()
                .join(", ");
            let workflow_note = format!(
                "workflow_id={wf_id}; assignments={}",
                if assignments.is_empty() {
                    "none".to_string()
                } else {
                    assignments
                }
            );
            tracing::info!(
                decision = "delegated",
                pathway = %pathway_label,
                rationale = %plan.rationale,
                subtask_count = plan.subtasks.len(),
                expected_utility_margin = plan.expected_utility_margin,
                "decomposition gate decision"
            );
            DecompositionOutcome::Delegated { workflow_note }
        }
    }
}

/// Build a system note summarising the decomposition gate decision for the LLM context.
pub(super) fn build_gate_system_note(
    gate_decision: &DecompositionDecision,
    delegation_workflow_note: Option<&str>,
) -> String {
    match gate_decision {
        DecompositionDecision::Centralized {
            rationale,
            expected_utility_margin,
        } => format!(
            "Delegation decision: centralized. rationale='{}' expected_utility_margin={:.2}",
            rationale, expected_utility_margin
        ),
        DecompositionDecision::Delegated(plan) => {
            let subtask_lines = plan
                .subtasks
                .iter()
                .enumerate()
                .map(|(idx, s)| format!("{}. {}", idx + 1, s))
                .collect::<Vec<_>>()
                .join("\n");
            let mut note = format!(
                "Delegation decision: delegated.\nRationale: {}\nExpected utility margin: {:.2}\nSubtasks:\n{}",
                plan.rationale, plan.expected_utility_margin, subtask_lines
            );
            if let Some(wf_note) = delegation_workflow_note {
                note.push_str(&format!("\nWorkflow: {wf_note}"));
            }
            note.push_str(
                "\nExecution directive: perform real delegation by emitting a tool_call for \
                 `orchestrate-subagents` (or `assign-tasks`) with the delegated task payload. \
                 Do not simulate delegated output.",
            );
            note
        }
        DecompositionDecision::RequiresSpecialistCreation { .. } => {
            "Delegation decision: specialist creation required with user approval.".to_string()
        }
    }
}

/// Build tool definitions for virtual delegation and orchestration tools.
///
/// These tools are not in the ToolRegistry (they're handled by special-case dispatch
/// in `execute_tool_call`), so their schemas must be defined statically here.
/// Combined with registry-sourced definitions in `build_all_tool_definitions`.
pub(super) fn build_delegation_tool_definitions() -> Vec<ironclad_llm::format::ToolDefinition> {
    use ironclad_llm::format::ToolDefinition;

    vec![
        ToolDefinition {
            name: "orchestrate-subagents".into(),
            description: "Delegate one or more subtasks to existing specialist subagents. \
                Each subtask is assigned to the best-matching subagent based on its skills."
                .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "subtasks": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "task": { "type": "string", "description": "The subtask description" },
                                "subagent": { "type": "string", "description": "Optional: specific subagent name to assign to" }
                            },
                            "required": ["task"]
                        },
                        "description": "List of subtasks to delegate"
                    }
                },
                "required": ["subtasks"]
            }),
        },
        ToolDefinition {
            name: "compose-subagent".into(),
            description:
                "Create a new specialist subagent with a specific name, skills, and model. \
                Use when no existing subagent matches the required capability."
                    .into(),
            parameters: serde_json::json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "description": "Unique name for the new subagent" },
                    "display_name": { "type": "string", "description": "Human-readable display name" },
                    "description": { "type": "string", "description": "What this subagent specialises in" },
                    "skills": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "List of skill/capability keywords"
                    },
                    "model": { "type": "string", "description": "Optional: preferred model for this subagent" }
                },
                "required": ["name", "description", "skills"]
            }),
        },
    ]
}

/// Build tool definitions from the ToolRegistry (registered tools) plus virtual tools.
///
/// This is the single source of truth for all tool definitions sent to the LLM provider.
pub(super) fn build_all_tool_definitions(
    registry: &ironclad_agent::tools::ToolRegistry,
) -> Vec<ironclad_llm::format::ToolDefinition> {
    use ironclad_llm::format::ToolDefinition;

    let mut defs = build_delegation_tool_definitions();

    for tool in registry.list() {
        defs.push(ToolDefinition {
            name: tool.name().to_string(),
            description: tool.description().to_string(),
            parameters: tool.parameters_schema(),
        });
    }

    defs
}
