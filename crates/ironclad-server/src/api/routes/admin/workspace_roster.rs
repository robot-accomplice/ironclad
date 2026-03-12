pub async fn roster(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let agents_in_registry = state.registry.list_agents().await;

    let workspace = std::path::Path::new(&config.agent.workspace);
    let os = ironclad_core::personality::load_os(workspace);
    let firmware = ironclad_core::personality::load_firmware(workspace);
    let directives = ironclad_core::personality::load_directives(workspace);

    let skills = ironclad_db::skills::list_skills(&state.db)
        .inspect_err(|e| tracing::warn!(error = %e, "failed to load skills for roster"))
        .unwrap_or_default();
    let enabled_skills: Vec<&str> = skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.as_str())
        .collect();
    let skill_kinds: std::collections::HashMap<&str, Vec<&str>> = {
        let mut map: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
        for s in &skills {
            if s.enabled {
                map.entry(s.kind.as_str())
                    .or_default()
                    .push(s.name.as_str());
            }
        }
        map
    };

    let voice = os.as_ref().map(|o| {
        json!({
            "formality": o.voice.formality,
            "proactiveness": o.voice.proactiveness,
            "verbosity": o.voice.verbosity,
            "humor": o.voice.humor,
            "domain": o.voice.domain,
        })
    });

    let missions: Vec<Value> = directives
        .as_ref()
        .map(|d| {
            d.missions
                .iter()
                .map(|m| {
                    json!({
                        "name": m.name,
                        "timeframe": m.timeframe,
                        "priority": m.priority,
                        "description": m.description,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let firmware_rules: Vec<Value> = firmware
        .as_ref()
        .map(|f| {
            f.rules
                .iter()
                .map(|r| {
                    json!({
                        "type": r.rule_type,
                        "rule": r.rule,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    let sub_agents = ironclad_db::agents::list_sub_agents(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to load sub-agents for roster"))
        .unwrap_or_default();
    let session_counts = ironclad_db::agents::list_session_counts_by_agent(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to load session counts for roster"))
        .unwrap_or_default();
    let taskable_sub_agents: Vec<&ironclad_db::agents::SubAgentRow> = sub_agents
        .iter()
        .filter(|sa| !sa.role.eq_ignore_ascii_case(ROLE_MODEL_PROXY))
        .collect();
    let model_proxies: Vec<&ironclad_db::agents::SubAgentRow> = sub_agents
        .iter()
        .filter(|sa| sa.role.eq_ignore_ascii_case(ROLE_MODEL_PROXY))
        .collect();

    let running_count = agents_in_registry
        .iter()
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .filter(|a| {
            taskable_sub_agents
                .iter()
                .any(|sa| sa.name.eq_ignore_ascii_case(&a.id))
        })
        .count();
    let stats = json!({
        "subordinate_count": taskable_sub_agents.len(),
        "running_subordinates": running_count,
        "total_skills": skills.len(),
        "enabled_skills": enabled_skills.len(),
    });

    let main_agent = json!({
        "id": config.agent.id,
        "name": config.agent.name,
        "display_name": config.agent.name,
        "role": "orchestrator",
        "model": config.models.primary,
        "enabled": true,
        "color": WORKSPACE_PALETTE[0],
        "session_count": null,
        "description": os.as_ref().map(|o| {
            let first_line = o.prompt_text.lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("Autonomous agent");
            first_line.to_string()
        }),
        "voice": voice,
        "missions": missions,
        "firmware_rules": firmware_rules,
        "skills": &enabled_skills,
        "capabilities": [
            "orchestrate-subagents",
            "assign-tasks",
            "select-subagent-model"
        ],
        "skill_breakdown": skill_kinds,
        "subordinates": taskable_sub_agents.iter().map(|a| a.name.clone()).collect::<Vec<_>>(),
        "stats": stats,
    });

    let specialist_cards: Vec<Value> = taskable_sub_agents.iter().enumerate().map(|(i, sa)| {
        let runtime = agents_in_registry.iter().find(|a| a.id == sa.name);
        let state_str = runtime.map(|r| format!("{:?}", r.state)).unwrap_or_else(|| {
            if sa.enabled { "Idle".into() } else { "Disabled".into() }
        });
        let model_mode = match sa.model.trim().to_ascii_lowercase().as_str() {
            "auto" => "auto",
            "orchestrator" => "orchestrator",
            _ => "fixed",
        };
        let color = WORKSPACE_PALETTE[(i + 1) % WORKSPACE_PALETTE.len()];
        let fallback_models =
            crate::api::routes::subagents::parse_fallback_models_json(sa.fallback_models_json.as_deref());
        let fixed_skills: Vec<String> = sa.skills_json.as_ref().map(|s| {
            serde_json::from_str::<Vec<String>>(s).unwrap_or_else(|e| {
                tracing::warn!(agent = %sa.name, error = %e, "corrupt skills_json, defaulting to []");
                Vec::new()
            })
        }).unwrap_or_default();
        json!({
            "id": sa.id,
            "name": sa.name,
            "display_name": sa.display_name,
            "role": ROLE_SUBAGENT,
            "model": sa.model,
            "fallback_models": fallback_models,
            "model_mode": model_mode,
            "resolved_model": runtime.map(|r| r.model.clone()),
            "enabled": sa.enabled,
            "color": color,
            "state": state_str,
            "session_count": session_counts.get(&sa.name).copied().unwrap_or(sa.session_count),
            "description": sa.description,
            "skills": fixed_skills.clone(),
            "fixed_skills": fixed_skills,
            "shared_skills": enabled_skills.clone(),
            "supervisor": config.agent.id,
        })
    }).collect();

    let mut roster = vec![main_agent];
    roster.extend(specialist_cards);

    let proxies: Vec<Value> = model_proxies
        .iter()
        .map(|sa| {
            json!({
                "id": sa.id,
                "name": sa.name,
                "display_name": sa.display_name,
                "role": ROLE_MODEL_PROXY,
                "model": sa.model,
                "enabled": sa.enabled
            })
        })
        .collect();

    Json(json!({
        "roster": roster,
        "count": roster.len(),
        "taskable_subagent_count": taskable_sub_agents.len(),
        "model_proxy_count": proxies.len(),
        "model_proxies": proxies
    }))
}
