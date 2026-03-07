fn workspace_files_snapshot(workspace_root: &std::path::Path) -> Value {
    let mut entries: Vec<Value> = Vec::new();
    if let Ok(read_dir) = std::fs::read_dir(workspace_root) {
        for entry in read_dir.flatten().take(200) {
            let path = entry.path();
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            if name.is_empty() || name.starts_with('.') {
                continue;
            }
            let kind = if path.is_dir() { "dir" } else { "file" };
            entries.push(json!({ "name": name, "kind": kind }));
        }
    }
    entries.sort_by(|a, b| {
        a["name"]
            .as_str()
            .unwrap_or_default()
            .cmp(b["name"].as_str().unwrap_or_default())
    });
    let entry_count = entries.len();
    json!({
        "workspace_root": workspace_root.display().to_string(),
        "top_level_entries": entries,
        "entry_count": entry_count,
    })
}

fn parse_db_timestamp_utc(ts: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(ts) {
        return Some(dt.with_timezone(&chrono::Utc));
    }
    if let Ok(ndt) = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S") {
        return Some(chrono::DateTime::from_naive_utc_and_offset(
            ndt,
            chrono::Utc,
        ));
    }
    None
}

fn is_recent_activity(ts: &str, now: chrono::DateTime<chrono::Utc>) -> bool {
    parse_db_timestamp_utc(ts)
        .map(|t| (now - t).num_seconds() <= WORKSPACE_ACTIVITY_WINDOW_SECS)
        .unwrap_or(false)
}

fn has_tool_token(tool_name_lower: &str, token: &str) -> bool {
    tool_name_lower
        .split(|c: char| !c.is_ascii_alphanumeric())
        .any(|part| part == token)
}

fn workstation_for_tool(tool_name: &str) -> (&'static str, &'static str) {
    let t = tool_name.to_lowercase();
    // Classify local file/search tooling before broad "search" web matching.
    if t.contains("read")
        || t.contains("write")
        || t.contains("file")
        || t.contains("glob")
        || has_tool_token(&t, "rg")
        || t.contains("patch")
        || t.contains("edit")
    {
        return ("files", "tool_execution");
    }
    if t.contains("web") || t.contains("http") || t.contains("fetch") || t.contains("search") {
        return ("web", "tool_execution");
    }
    if t.contains("memory") {
        return ("memory", "working");
    }
    if t.contains("wallet")
        || t.contains("chain")
        || t.contains("block")
        || t.contains("contract")
        || t.contains("token")
    {
        return ("blockchain", "tool_execution");
    }
    ("exec", "tool_execution")
}

fn derive_workspace_activity(
    db: &ironclad_db::Database,
    agent_id: &str,
    running: bool,
    now: chrono::DateTime<chrono::Utc>,
) -> (Option<&'static str>, &'static str, Option<String>) {
    if !running {
        return (Some("standby"), "idle", None);
    }

    let conn = db.conn();

    let latest_tool: Option<(String, String)> = conn
        .query_row(
            "SELECT tc.tool_name, tc.created_at
             FROM tool_calls tc
             INNER JOIN turns t ON t.id = tc.turn_id
             INNER JOIN sessions s ON s.id = t.session_id
             WHERE s.agent_id = ?1
             ORDER BY tc.created_at DESC
             LIMIT 1",
            [agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .inspect_err(
            |e| tracing::debug!(error = %e, "failed to query latest tool call for agent status"),
        )
        .ok();

    if let Some((tool_name, created_at)) = latest_tool
        && is_recent_activity(&created_at, now)
    {
        let (workstation, activity) = workstation_for_tool(&tool_name);
        return (Some(workstation), activity, Some(tool_name));
    }

    // Subagents execute through delegated orchestration calls recorded under the
    // orchestrator session. Attribute recent delegated tool activity back to the
    // selected subagent so workspace animation reflects real delegated execution.
    let latest_delegated: Option<(String, String)> = conn
        .query_row(
            "SELECT tc.tool_name, tc.created_at
             FROM tool_calls tc
             WHERE tc.output LIKE ('%delegated_subagent=' || ?1 || '%')
             ORDER BY tc.created_at DESC
             LIMIT 1",
            [agent_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .inspect_err(|e| {
            tracing::debug!(
                error = %e,
                subagent = %agent_id,
                "failed to query delegated tool activity for workspace state"
            )
        })
        .ok();
    if let Some((tool_name, created_at)) = latest_delegated
        && is_recent_activity(&created_at, now)
    {
        let (workstation, activity) = workstation_for_tool(&tool_name);
        return (Some(workstation), activity, Some(tool_name));
    }

    let latest_turn_created: Option<String> = conn
        .query_row(
            "SELECT t.created_at
             FROM turns t
             INNER JOIN sessions s ON s.id = t.session_id
             WHERE s.agent_id = ?1
             ORDER BY t.created_at DESC
             LIMIT 1",
            [agent_id],
            |row| row.get(0),
        )
        .inspect_err(
            |e| tracing::debug!(error = %e, "failed to query latest turn for agent status"),
        )
        .ok();

    if let Some(created_at) = latest_turn_created
        && is_recent_activity(&created_at, now)
    {
        return (Some("llm"), "inference", None);
    }

    (Some("standby"), "idle", None)
}

pub async fn workspace_state(State(state): State<AppState>) -> impl IntoResponse {
    let agents = state.registry.list_agents().await;
    let config = state.config.read().await;
    let now = chrono::Utc::now();
    let workspace_root = std::path::Path::new(&config.agent.workspace);
    let files = workspace_files_snapshot(workspace_root);

    let systems: Vec<Value> = vec![
        json!({ "id": "llm",        "name": "LLM Inference",   "kind": "Inference",   "x": 0.18, "y": 0.22 }),
        json!({ "id": "memory",     "name": "Memory",          "kind": "Storage",     "x": 0.82, "y": 0.22 }),
        json!({ "id": "exec",       "name": "Code Execution",  "kind": "Execution",   "x": 0.18, "y": 0.78 }),
        json!({ "id": "blockchain", "name": "Blockchain",      "kind": "Blockchain",  "x": 0.82, "y": 0.78 }),
        json!({ "id": "web",        "name": "Web / APIs",      "kind": "Tool",        "x": 0.50, "y": 0.12 }),
        json!({ "id": "files",      "name": "File System",     "kind": "Tool",        "x": 0.50, "y": 0.88 }),
        json!({ "id": "shelter",    "name": "Shelter",         "kind": "Shelter",     "x": 0.035, "y": 0.50 }),
    ];

    let skills = ironclad_db::skills::list_skills(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to load skills for workspace state"))
        .unwrap_or_default();
    let enabled_skills: Vec<String> = skills
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.name.clone())
        .collect();

    let agent_list: Vec<Value> = agents
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let color = WORKSPACE_PALETTE[(i + 1) % WORKSPACE_PALETTE.len()];
            let running = format!("{:?}", a.state).to_lowercase() == "running";
            let (workstation, activity, active_skill) =
                derive_workspace_activity(&state.db, &a.id, running, now);
            json!({
                "id": a.id,
                "name": a.name,
                "role": ROLE_SUBAGENT,
                "state": a.state,
                "color": color,
                "model": a.model,
                "current_workstation": workstation,
                "activity": activity,
                "active_skill": active_skill,
                "updated_at": chrono::Utc::now().to_rfc3339(),
                "subordinates": [],
                "supervisor": config.agent.id,
            })
        })
        .collect();

    let (main_workstation, main_activity, main_active_skill) =
        derive_workspace_activity(&state.db, &config.agent.id, true, now);

    let main_agent = json!({
        "id": config.agent.id,
        "name": config.agent.name,
        "role": "agent",
        "state": "Running",
        "color": WORKSPACE_PALETTE[0],
        "model": config.models.primary,
        "current_workstation": main_workstation,
        "activity": main_activity,
        "active_skill": main_active_skill.or_else(|| enabled_skills.first().cloned()),
        "skills": enabled_skills,
        "updated_at": chrono::Utc::now().to_rfc3339(),
        "subordinates": agent_list.iter()
            .filter(|a| a["role"] == ROLE_SUBAGENT)
            .map(|a| a["id"].clone())
            .collect::<Vec<_>>(),
        "supervisor": null,
    });

    let mut all_agents = vec![main_agent];
    all_agents.extend(agent_list);

    Json(json!({
        "agents": all_agents,
        "systems": systems,
        "files": files,
        "interactions": [],
    }))
}
