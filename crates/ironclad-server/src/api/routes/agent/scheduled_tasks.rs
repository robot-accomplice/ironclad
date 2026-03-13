use serde_json::json;

use super::AppState;
use super::decomposition::{
    DecompositionOutcome, DelegationProvenance, apply_decomposition_decision,
    build_gate_system_note, evaluate_decomposition_gate,
};

pub(crate) async fn execute_scheduled_agent_task(
    state: &AppState,
    agent_id: &str,
    task: &str,
) -> Result<String, String> {
    let config = state.config.read().await;
    let root_agent_id = config.agent.id.clone();
    if !agent_id.eq_ignore_ascii_case(&root_agent_id) {
        let params = json!({"task": task, "subagent": agent_id});
        return super::delegation::execute_virtual_subagent_tool_call(
            state,
            "delegate-subagent",
            &params,
            &uuid::Uuid::new_v4().to_string(),
            ironclad_core::InputAuthority::SelfGenerated,
            ironclad_core::SurvivalTier::Normal,
        )
        .await;
    }

    let session_id = ironclad_db::sessions::find_or_create(
        &state.db,
        agent_id,
        Some(&ironclad_db::sessions::SessionScope::Agent),
    )
    .map_err(|e| format!("failed to create scheduled-task session: {e}"))?;

    let _ = ironclad_db::sessions::append_message(&state.db, &session_id, "user", task)
        .map_err(|e| format!("failed to store scheduled-task prompt: {e}"))?;

    let turn_id = uuid::Uuid::new_v4().to_string();
    if let Err(e) = ironclad_db::sessions::create_turn_with_id(
        &state.db,
        &turn_id,
        &session_id,
        None,
        None,
        None,
        None,
    ) {
        tracing::warn!(error = %e, "failed to pre-create scheduled-task turn");
    }

    let personality = state.personality.read().await;
    let os_text = personality.os_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let features = ironclad_llm::extract_features(task, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let gate_decision = evaluate_decomposition_gate(state, task, complexity).await;
    let outcome = apply_decomposition_decision(state, &gate_decision, &session_id, "cron").await;
    let delegation_workflow_note = match outcome {
        DecompositionOutcome::SpecialistProposalPending { prompt } => {
            return Err(format!(
                "scheduled task requires specialist creation before execution: {prompt}"
            ));
        }
        DecompositionOutcome::Centralized => None,
        DecompositionOutcome::Delegated { workflow_note } => Some(workflow_note),
    };
    let gate_system_note =
        build_gate_system_note(&gate_decision, delegation_workflow_note.as_deref());

    let input = super::core::InferenceInput {
        state,
        session_id: &session_id,
        user_content: task,
        turn_id: &turn_id,
        channel_label: "cron",
        agent_name: config.agent.name.clone(),
        agent_id: root_agent_id,
        os_text,
        firmware_text,
        primary_model: config.models.primary.clone(),
        tier_adapt: config.tier_adapt.clone(),
        delegation_workflow_note,
        inject_diagnostics: false,
        gate_system_note: Some(gate_system_note),
        delegated_execution_note: None,
        is_correction_turn: false,
    };
    drop(config);

    let prepared = super::core::prepare_inference(&input).await?;
    let mut provenance = DelegationProvenance::default();
    let result = super::core::execute_inference_pipeline(
        state,
        &prepared,
        &session_id,
        task,
        &turn_id,
        ironclad_core::InputAuthority::SelfGenerated,
        Some("cron"),
        &mut provenance,
    )
    .await?;

    Ok(result.content)
}
