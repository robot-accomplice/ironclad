pub async fn list_approvals(State(state): State<AppState>) -> impl IntoResponse {
    state.approvals.expire_timed_out();
    let pending = state.approvals.list_pending();
    let all = state.approvals.list_all();
    Json(json!({
        "pending": pending,
        "total": all.len(),
    }))
}

const MAX_DECIDED_BY_LEN: usize = 256;

#[derive(Deserialize)]
pub struct ApprovalDecisionRequest {
    #[serde(default = "default_decided_by")]
    pub decided_by: String,
}
fn default_decided_by() -> String {
    "api".into()
}

/// Sanitize the `decided_by` field: enforce max length and strip control characters.
fn sanitize_decided_by(raw: &str) -> Result<String, JsonError> {
    if raw.len() > MAX_DECIDED_BY_LEN {
        return Err(bad_request(format!(
            "decided_by exceeds max length of {MAX_DECIDED_BY_LEN} characters"
        )));
    }
    let sanitized: String = raw.chars().filter(|c| !c.is_control()).collect();
    Ok(sanitized)
}

pub async fn approve_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<axum::Json<ApprovalDecisionRequest>>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let decided_by = match body {
        Some(axum::Json(b)) => sanitize_decided_by(&b.decided_by)?,
        None => default_decided_by(),
    };
    match state.approvals.approve(&id, &decided_by) {
        Ok(req) => {
            if let Some(decided_at) = req.decided_at {
                ironclad_db::approvals::record_approval_decision(
                    &state.db,
                    &req.id,
                    "approved",
                    req.decided_by.as_deref().unwrap_or(&decided_by),
                    &decided_at.to_rfc3339(),
                )
                .inspect_err(|e| tracing::warn!(error = %e, "failed to persist approval decision"))
                .ok();
            }

            let replay_req = req.clone();
            let replay_state = state.clone();
            tokio::spawn(async move {
                let params: serde_json::Value = serde_json::from_str(&replay_req.tool_input)
                    .unwrap_or_else(|_| json!({ "raw_input": replay_req.tool_input }));
                let replay_turn_id = replay_req
                    .session_id
                    .clone()
                    .unwrap_or_else(|| replay_req.id.clone());

                replay_state.event_bus.publish(
                    json!({
                        "type": "approval_replay_started",
                        "request_id": replay_req.id,
                        "tool": replay_req.tool_name,
                        "turn_id": replay_turn_id,
                    })
                    .to_string(),
                );

                let replay_result = super::agent::execute_tool_call_after_approval(
                    &replay_state,
                    &replay_req.tool_name,
                    &params,
                    &replay_turn_id,
                    replay_req.requested_authority,
                    None,
                )
                .await;

                match replay_result {
                    Ok(output) => replay_state.event_bus.publish(
                        json!({
                            "type": "approval_replay_succeeded",
                            "request_id": replay_req.id,
                            "tool": replay_req.tool_name,
                            "turn_id": replay_turn_id,
                            "output": output,
                        })
                        .to_string(),
                    ),
                    Err(error) => replay_state.event_bus.publish(
                        json!({
                            "type": "approval_replay_failed",
                            "request_id": replay_req.id,
                            "tool": replay_req.tool_name,
                            "turn_id": replay_turn_id,
                            "error": error,
                        })
                        .to_string(),
                    ),
                }
            });

            Ok(Json(json!({
                "approval": req,
                "replay_queued": true,
            })))
        }
        Err(e) => Err(not_found(e.to_string())),
    }
}

pub async fn deny_request(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Option<axum::Json<ApprovalDecisionRequest>>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let decided_by = match body {
        Some(axum::Json(b)) => sanitize_decided_by(&b.decided_by)?,
        None => default_decided_by(),
    };
    match state.approvals.deny(&id, &decided_by) {
        Ok(req) => Ok(Json(json!(req))),
        Err(e) => Err(not_found(e.to_string())),
    }
}

// ── Audit trail routes ───────────────────────────────────────

pub async fn get_policy_audit(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let decisions =
        ironclad_db::policy::get_decisions_for_turn(&state.db, &turn_id).map_err(|e| {
            tracing::error!(error = %e, "failed to fetch policy audit");
            JsonError(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal server error".into(),
            )
        })?;
    Ok(Json(json!({
        "turn_id": turn_id,
        "decisions": decisions.iter().map(|d| json!({
            "id": d.id,
            "tool_name": d.tool_name,
            "decision": d.decision,
            "rule_name": d.rule_name,
            "reason": d.reason,
            "created_at": d.created_at,
        })).collect::<Vec<_>>(),
    })))
}

pub async fn get_tool_audit(
    State(state): State<AppState>,
    Path(turn_id): Path<String>,
) -> std::result::Result<impl IntoResponse, JsonError> {
    let calls = ironclad_db::tools::get_tool_calls_for_turn(&state.db, &turn_id).map_err(|e| {
        tracing::error!(error = %e, "failed to fetch tool audit");
        JsonError(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal server error".into(),
        )
    })?;
    Ok(Json(json!({
        "turn_id": turn_id,
        "tool_calls": calls.iter().map(|c| json!({
            "id": c.id,
            "tool_name": c.tool_name,
            "skill_id": c.skill_id,
            "skill_name": c.skill_name,
            "skill_hash": c.skill_hash,
            "input": c.input,
            "output": c.output,
            "status": c.status,
            "duration_ms": c.duration_ms,
            "created_at": c.created_at,
        })).collect::<Vec<_>>(),
    })))
}
