use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;

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
    ) {
        Ok(req) => match serde_json::to_value(req) {
            Ok(v) => (StatusCode::CREATED, Json(v)).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("serialization error: {e}")).into_response(),
        },
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
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("serialization error: {e}")).into_response(),
        },
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn approve_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecisionRequest>,
) -> impl IntoResponse {
    match state.approvals.approve(&id, &body.decided_by) {
        Ok(req) => match serde_json::to_value(req) {
            Ok(v) => Json(v).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("serialization error: {e}")).into_response(),
        },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn deny_approval(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(body): Json<DecisionRequest>,
) -> impl IntoResponse {
    match state.approvals.deny(&id, &body.decided_by) {
        Ok(req) => match serde_json::to_value(req) {
            Ok(v) => Json(v).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("serialization error: {e}")).into_response(),
        },
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

pub async fn cleanup_approvals(State(state): State<AppState>) -> impl IntoResponse {
    let cleared = state.approvals.clear_decided();
    Json(serde_json::json!({ "cleared": cleared }))
}
