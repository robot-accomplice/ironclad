use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use ironclad_core::InputAuthority;

use super::AppState;

#[derive(Deserialize)]
pub struct CreateApprovalRequest {
    pub tool_name: String,
    pub tool_input: String,
    pub session_id: Option<String>,
}

#[derive(Deserialize)]
pub struct DecisionRequest {
    pub decided_by: String,
}

pub async fn list_approvals(State(state): State<AppState>) -> impl IntoResponse {
    let all = state.approvals.list_all();
    Json(serde_json::json!({ "approvals": all }))
}

pub async fn list_pending_approvals(State(state): State<AppState>) -> impl IntoResponse {
    let pending = state.approvals.list_pending();
    Json(serde_json::json!({ "approvals": pending }))
}

pub async fn create_approval(
    State(state): State<AppState>,
    Json(body): Json<CreateApprovalRequest>,
) -> impl IntoResponse {
    match state.approvals.request_approval(
        &body.tool_name,
        &body.tool_input,
        body.session_id.as_deref(),
        InputAuthority::Creator,
    ) {
        Ok(req) => {
            if let Err(e) = ironclad_db::approvals::record_approval_request(
                &state.db,
                &req.id,
                &req.tool_name,
                &req.tool_input,
                req.session_id.as_deref(),
                "pending",
                &req.timeout_at.to_rfc3339(),
            ) {
                return super::internal_err(&e).into_response();
            }
            match serde_json::to_value(req) {
                Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
                Err(e) => super::internal_err(&e).into_response(),
            }
        }
        Err(e) => super::internal_err(&e).into_response(),
    }
}

pub async fn get_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    match state.approvals.get_request(&id) {
        Some(req) => match serde_json::to_value(req) {
            Ok(v) => Json(v).into_response(),
            Err(e) => super::internal_err(&e).into_response(),
        },
        None => super::not_found("approval not found").into_response(),
    }
}

pub async fn approve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecisionRequest>,
) -> impl IntoResponse {
    match state.approvals.approve(&id, &body.decided_by) {
        Ok(req) => {
            if let Some(decided_at) = req.decided_at {
                if let Err(e) = ironclad_db::approvals::record_approval_decision(
                    &state.db,
                    &req.id,
                    "approved",
                    req.decided_by.as_deref().unwrap_or(&body.decided_by),
                    &decided_at.to_rfc3339(),
                ) {
                    return super::internal_err(&e).into_response();
                }
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
                    Ok(output) => {
                        replay_state.event_bus.publish(
                            json!({
                                "type": "approval_replay_succeeded",
                                "request_id": replay_req.id,
                                "tool": replay_req.tool_name,
                                "turn_id": replay_turn_id,
                                "output": output,
                            })
                            .to_string(),
                        );
                    }
                    Err(error) => {
                        replay_state.event_bus.publish(
                            json!({
                                "type": "approval_replay_failed",
                                "request_id": replay_req.id,
                                "tool": replay_req.tool_name,
                                "turn_id": replay_turn_id,
                                "error": error,
                            })
                            .to_string(),
                        );
                    }
                }
            });

            match serde_json::to_value(&req) {
                Ok(mut v) => {
                    if let Some(obj) = v.as_object_mut() {
                        obj.insert("replay_queued".to_string(), json!(true));
                    }
                    Json(v).into_response()
                }
                Err(e) => super::internal_err(&e).into_response(),
            }
        }
        Err(_) => super::not_found("approval not found").into_response(),
    }
}

pub async fn deny_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecisionRequest>,
) -> impl IntoResponse {
    match state.approvals.deny(&id, &body.decided_by) {
        Ok(req) => {
            if let Some(decided_at) = req.decided_at {
                if let Err(e) = ironclad_db::approvals::record_approval_decision(
                    &state.db,
                    &req.id,
                    "denied",
                    req.decided_by.as_deref().unwrap_or(&body.decided_by),
                    &decided_at.to_rfc3339(),
                ) {
                    return super::internal_err(&e).into_response();
                }
            }
            match serde_json::to_value(req) {
                Ok(v) => Json(v).into_response(),
                Err(e) => super::internal_err(&e).into_response(),
            }
        }
        Err(_) => super::not_found("approval not found").into_response(),
    }
}

pub async fn cleanup_approvals(State(state): State<AppState>) -> impl IntoResponse {
    let cleared = state.approvals.clear_decided();
    Json(serde_json::json!({ "cleared": cleared }))
}
