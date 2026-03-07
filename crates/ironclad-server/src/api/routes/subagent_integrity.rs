use std::collections::BTreeSet;

use ironclad_agent::subagents::{AgentInstance, AgentInstanceConfig, AgentRunState};

use super::AppState;
use super::subagents::{ROLE_SUBAGENT, normalize_role, resolve_taskable_subagent_runtime_model};

const BUILTIN_SKILLS_JSON: &str = include_str!(concat!(env!("OUT_DIR"), "/builtin-skills.json"));
const INTERNALIZED_SKILLS: &[&str] = &[
    "update-and-rollback",
    "workflow-design",
    "skill-creation",
    "session-operator",
    "claims-auditor",
    "efficacy-assessment",
    "fast-cache",
    "model-routing-tuner",
];

fn parse_skills_json(skills_json: Option<&str>) -> Vec<String> {
    skills_json
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

fn capability_tokens(text: &str) -> Vec<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|t| t.len() >= 4)
        .map(|s| s.to_string())
        .collect()
}

#[derive(Debug, Clone)]
pub(crate) struct SubagentIntegrity {
    pub inferred_skills: Vec<String>,
    pub has_fixed_skills: bool,
    pub missing_session: bool,
    pub runtime_registered: bool,
    pub runtime_running: bool,
    pub runtime_state: String,
    pub repairable: bool,
}

#[derive(Debug, serde::Deserialize)]
struct BuiltinSkillCatalogEntry {
    name: String,
}

fn skill_registry_names(state: &AppState) -> BTreeSet<String> {
    let mut out: BTreeSet<String> =
        serde_json::from_str::<Vec<BuiltinSkillCatalogEntry>>(BUILTIN_SKILLS_JSON)
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.name.to_ascii_lowercase())
            .collect();
    out.extend(INTERNALIZED_SKILLS.iter().map(|s| s.to_ascii_lowercase()));
    if let Ok(db_skills) = ironclad_db::skills::list_skills(&state.db) {
        out.extend(
            db_skills
                .into_iter()
                .filter(|s| s.enabled)
                .map(|s| s.name.to_ascii_lowercase()),
        );
    }
    out
}

fn inferred_skill_tokens(agent: &ironclad_db::agents::SubAgentRow) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for token in capability_tokens(&agent.name.replace(['-', '_'], " ")) {
        out.insert(token);
    }
    if let Some(display) = agent.display_name.as_deref() {
        for token in capability_tokens(display) {
            out.insert(token);
        }
    }
    if let Some(description) = agent.description.as_deref() {
        for token in capability_tokens(description) {
            out.insert(token);
        }
    }
    out
}

fn matched_repair_skills(
    state: &AppState,
    agent: &ironclad_db::agents::SubAgentRow,
) -> Vec<String> {
    let registry = skill_registry_names(state);
    let tokens = inferred_skill_tokens(agent);
    let mut scored: Vec<(String, usize)> = registry
        .into_iter()
        .map(|skill| {
            let overlap = capability_tokens(&skill.replace('-', " "))
                .into_iter()
                .filter(|tok| tokens.contains(tok))
                .count();
            (skill, overlap)
        })
        .filter(|(_, overlap)| *overlap > 0)
        .collect();
    scored.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut matched: Vec<String> = scored.into_iter().map(|(skill, _)| skill).take(4).collect();
    if matched.is_empty() {
        matched = vec![
            "context-continuity".to_string(),
            "self-diagnostics".to_string(),
        ];
    }
    matched
}

pub(crate) fn assess_subagent_integrity(
    agent: &ironclad_db::agents::SubAgentRow,
    runtime: Option<&AgentInstance>,
    session_count: i64,
) -> SubagentIntegrity {
    let fixed_skills = parse_skills_json(agent.skills_json.as_deref());
    let inferred_skills = inferred_skill_tokens(agent).into_iter().take(8).collect();
    let runtime_state = runtime
        .map(|inst| match inst.state {
            AgentRunState::Idle => "idle",
            AgentRunState::Starting => "booting",
            AgentRunState::Running => "running",
            AgentRunState::Stopped => "stopped",
            AgentRunState::Error => "error",
        })
        .unwrap_or("missing")
        .to_string();
    let runtime_registered = runtime.is_some();
    let runtime_running = runtime.is_some_and(|inst| inst.state == AgentRunState::Running);
    let missing_session = session_count <= 0;
    let has_fixed_skills = !fixed_skills.is_empty();
    let repairable = normalize_role(&agent.role) == Some(ROLE_SUBAGENT) && agent.enabled;

    SubagentIntegrity {
        inferred_skills,
        has_fixed_skills,
        missing_session,
        runtime_registered,
        runtime_running,
        runtime_state,
        repairable,
    }
}

pub(crate) async fn ensure_taskable_subagent_ready(
    state: &AppState,
    agent: &ironclad_db::agents::SubAgentRow,
) -> Result<ironclad_db::agents::SubAgentRow, String> {
    if normalize_role(&agent.role) != Some(ROLE_SUBAGENT) {
        return Err(format!(
            "subagent '{}' is not taskable (role={})",
            agent.name, agent.role
        ));
    }
    if !agent.enabled {
        return Err(format!("subagent '{}' is disabled", agent.name));
    }

    let runtime = state.registry.get_agent(&agent.name).await;
    let session_count = ironclad_db::agents::list_session_counts_by_agent(&state.db)
        .map_err(|e| format!("failed to read subagent session counts: {e}"))?
        .get(&agent.name)
        .copied()
        .unwrap_or(agent.session_count);
    let integrity = assess_subagent_integrity(agent, runtime.as_ref(), session_count);
    let repair_skills = matched_repair_skills(state, agent);

    if !integrity.repairable {
        return Err(format!(
            "subagent '{}' is hollow and not repairable from current metadata",
            agent.name
        ));
    }

    let mut updated = agent.clone();
    let mut changed = false;
    if !integrity.has_fixed_skills && !repair_skills.is_empty() {
        updated.skills_json =
            Some(serde_json::to_string(&repair_skills).unwrap_or_else(|_| "[]".to_string()));
        changed = true;
    }
    if changed {
        ironclad_db::agents::upsert_sub_agent(&state.db, &updated)
            .map_err(|e| format!("failed to persist repaired subagent '{}': {e}", agent.name))?;
    }

    let live_skills = parse_skills_json(updated.skills_json.as_deref());
    ironclad_db::sessions::find_or_create(&state.db, &updated.name, None).map_err(|e| {
        format!(
            "failed to ensure session for subagent '{}': {e}",
            updated.name
        )
    })?;

    if state.registry.get_agent(&updated.name).await.is_none() {
        let config = AgentInstanceConfig {
            id: updated.name.clone(),
            name: updated
                .display_name
                .clone()
                .unwrap_or_else(|| updated.name.clone()),
            model: resolve_taskable_subagent_runtime_model(state, &updated.model).await,
            skills: live_skills,
            allowed_subagents: vec![],
            max_concurrent: 4,
        };
        state.registry.register(config).await.map_err(|e| {
            format!(
                "failed to register repaired subagent '{}': {e}",
                updated.name
            )
        })?;
    }
    state
        .registry
        .start_agent(&updated.name)
        .await
        .map_err(|e| format!("failed to start repaired subagent '{}': {e}", updated.name))?;

    let refreshed = ironclad_db::agents::list_sub_agents(&state.db)
        .map_err(|e| format!("failed to reload subagent '{}': {e}", updated.name))?
        .into_iter()
        .find(|row| row.name == updated.name)
        .unwrap_or(updated);
    Ok(refreshed)
}
