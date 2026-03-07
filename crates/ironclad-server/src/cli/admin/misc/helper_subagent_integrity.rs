#[derive(Debug, Default, Clone, Copy)]
pub(super) struct SubagentIntegrityProbe {
    hollow_subagents: u64,
    repaired_skills: u64,
    repaired_sessions: u64,
}

pub(super) async fn probe_subagent_integrity_via_gateway(
    base_url: &str,
    repair: bool,
) -> Result<SubagentIntegrityProbe, Box<dyn std::error::Error>> {
    let client = super::http_client()?;
    let resp = client.get(format!("{base_url}/api/subagents")).send().await?;
    if !resp.status().is_success() {
        return Ok(SubagentIntegrityProbe::default());
    }
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let agents = body
        .get("agents")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let mut probe = SubagentIntegrityProbe::default();
    for agent in agents {
        let name = agent.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let enabled = agent
            .get("enabled")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !enabled || name.is_empty() {
            continue;
        }
        let integrity = agent.get("integrity").cloned().unwrap_or_default();
        let hollow = integrity
            .get("hollow")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let repairable = integrity
            .get("repairable")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if !hollow || !repairable {
            continue;
        }
        probe.hollow_subagents += 1;
        if !repair {
            continue;
        }

        let inferred_skills: Vec<String> = integrity
            .get("inferred_skills")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        if !inferred_skills.is_empty() {
            let payload = serde_json::json!({ "skills": inferred_skills });
            let repair_resp = client
                .put(format!("{base_url}/api/subagents/{name}"))
                .json(&payload)
                .send()
                .await?;
            if repair_resp.status().is_success() {
                probe.repaired_skills += 1;
            }
        }
        let missing_session = integrity
            .get("missing_session")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if missing_session {
            let session_resp = client
                .post(format!("{base_url}/api/sessions"))
                .json(&serde_json::json!({ "agent_id": name }))
                .send()
                .await?;
            if session_resp.status().is_success() {
                probe.repaired_sessions += 1;
            }
        }
    }
    Ok(probe)
}
