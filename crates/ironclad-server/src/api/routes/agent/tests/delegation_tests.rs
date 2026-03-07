#[tokio::test]
async fn delegation_repairs_hollow_subagent_before_selection() {
    let state = crate::api::routes::tests::test_state();
    let row = ironclad_db::agents::SubAgentRow {
        id: uuid::Uuid::new_v4().to_string(),
        name: "moltbook-monitor".to_string(),
        display_name: Some("Moltbook Monitor".to_string()),
        model: "auto".to_string(),
        fallback_models_json: Some("[]".to_string()),
        role: "subagent".to_string(),
        description: Some("Monitors the moltbook feed and reports changes".to_string()),
        skills_json: Some("[]".to_string()),
        enabled: true,
        session_count: 0,
    };
    ironclad_db::agents::upsert_sub_agent(&state.db, &row).unwrap();

    let sid = ironclad_db::sessions::find_or_create(&state.db, "test-turn-agent", None).unwrap();
    let turn_id =
        ironclad_db::sessions::create_turn(&state.db, &sid, None, None, None, None).unwrap();

    let out = super::execute_virtual_subagent_tool_call(
        &state,
        "select-subagent-model",
        &serde_json::json!({"specialist":"moltbook-monitor","task":"check the moltbook feed and summarize updates"}),
        &turn_id,
        ironclad_core::InputAuthority::Creator,
        ironclad_core::SurvivalTier::Normal,
    )
    .await
    .unwrap();

    assert!(out.contains("selected_subagent=moltbook-monitor"));
    let repaired = ironclad_db::agents::list_sub_agents(&state.db)
        .unwrap()
        .into_iter()
        .find(|a| a.name == "moltbook-monitor")
        .unwrap();
    let repaired_skills = super::parse_skills_json(repaired.skills_json.as_deref());
    assert!(
        !repaired_skills.is_empty(),
        "hollow subagent should be repaired with real skill names"
    );
    assert!(
        repaired_skills
            .iter()
            .all(|skill| matches!(skill.as_str(), "context-continuity" | "self-diagnostics")),
        "repair should persist actual known skills, not guessed capability tokens"
    );
    let runtime = state.registry.get_agent("moltbook-monitor").await.unwrap();
    assert_eq!(
        runtime.state,
        ironclad_agent::subagents::AgentRunState::Running
    );
    let session_counts = ironclad_db::agents::list_session_counts_by_agent(&state.db).unwrap();
    assert!(session_counts.get("moltbook-monitor").copied().unwrap_or(0) > 0);
}
