//! Scheduled (cron) task execution via the unified pipeline.
//!
//! ## Security fix
//!
//! Injection defense was completely absent from this entry point.
//! Now routed through `PipelineConfig::cron()` which enables injection
//! defense (block at >0.7, sanitize at 0.3-0.7).

use serde_json::json;

use super::AppState;
use super::decomposition::{
    DecompositionOutcome, DelegationProvenance, apply_decomposition_decision,
    build_gate_system_note, evaluate_decomposition_gate,
};
use super::pipeline::{PipelineConfig, UnifiedPipelineInput};

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
    drop(config);

    // ── SECURITY FIX: injection defense (was completely missing) ─────
    let pipeline_config = PipelineConfig::cron();
    let task_content = if pipeline_config.injection_defense {
        let threat = ironclad_agent::injection::check_injection(task);
        if threat.is_blocked() {
            return Err(format!(
                "scheduled task blocked: injection detected (score={:.2})",
                threat.value()
            ));
        }
        if threat.is_caution() {
            tracing::info!(
                score = threat.value(),
                "Sanitizing caution-level cron task input"
            );
            ironclad_agent::injection::sanitize(task)
        } else {
            task.to_string()
        }
    } else {
        task.to_string()
    };

    // ── Session resolution (Dedicated: agent-scoped) ────────────────
    let session_id = ironclad_db::sessions::find_or_create(
        &state.db,
        agent_id,
        Some(&ironclad_db::sessions::SessionScope::Agent),
    )
    .map_err(|e| format!("failed to create scheduled-task session: {e}"))?;

    let _ = ironclad_db::sessions::append_message(&state.db, &session_id, "user", &task_content)
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

    // ── Decomposition gate ──────────────────────────────────────────
    let features = ironclad_llm::extract_features(&task_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let gate_decision = evaluate_decomposition_gate(state, &task_content, complexity).await;
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

    // ── Unified pipeline (replaces manual InferenceInput + prepare + execute) ──
    let input = UnifiedPipelineInput {
        state,
        config: &pipeline_config,
        session_id: &session_id,
        user_content: &task_content,
        turn_id: &turn_id,
        is_correction_turn: false,
        delegation_workflow_note,
        gate_system_note: Some(gate_system_note),
        delegated_execution_note: None,
        delegation_provenance: DelegationProvenance::default(),
    };

    let result = super::pipeline::execute_unified_pipeline(input).await?;
    Ok(result.content)
}
