use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use serde::Deserialize;
use serde_json::Value;

use super::{AppState, internal_err};

#[derive(Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub agent_id: String,
    pub schedule_kind: String,
    pub schedule_expr: Option<String>,
    pub payload_json: Option<String>,
}

pub async fn list_cron_jobs(State(state): State<AppState>) -> impl IntoResponse {
    match ironclad_db::cron::list_jobs(&state.db) {
        Ok(jobs) => {
            let items: Vec<Value> = jobs
                .into_iter()
                .map(|j| {
                    serde_json::json!({
                        "id": j.id,
                        "name": j.name,
                        "description": j.description,
                        "enabled": j.enabled,
                        "schedule_kind": j.schedule_kind,
                        "schedule_expr": j.schedule_expr,
                        "agent_id": j.agent_id,
                        "last_run_at": j.last_run_at,
                        "last_status": j.last_status,
                        "consecutive_errors": j.consecutive_errors,
                        "next_run_at": j.next_run_at,
                    })
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "jobs": items })))
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn create_cron_job(
    State(state): State<AppState>,
    axum::Json(body): axum::Json<CreateCronJobRequest>,
) -> impl IntoResponse {
    let payload = body.payload_json.as_deref().unwrap_or("{}");
    match ironclad_db::cron::create_job(
        &state.db,
        &body.name,
        &body.agent_id,
        &body.schedule_kind,
        body.schedule_expr.as_deref(),
        payload,
    ) {
        Ok(id) => Ok(axum::Json(serde_json::json!({ "job_id": id }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn get_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::cron::get_job(&state.db, &id) {
        Ok(Some(job)) => Ok(axum::Json(serde_json::json!({
            "id": job.id,
            "name": job.name,
            "description": job.description,
            "enabled": job.enabled,
            "schedule_kind": job.schedule_kind,
            "schedule_expr": job.schedule_expr,
            "schedule_every_ms": job.schedule_every_ms,
            "schedule_tz": job.schedule_tz,
            "agent_id": job.agent_id,
            "session_target": job.session_target,
            "payload_json": job.payload_json,
            "delivery_mode": job.delivery_mode,
            "delivery_channel": job.delivery_channel,
            "last_run_at": job.last_run_at,
            "last_status": job.last_status,
            "last_duration_ms": job.last_duration_ms,
            "consecutive_errors": job.consecutive_errors,
            "next_run_at": job.next_run_at,
            "last_error": job.last_error,
        }))),
        Ok(None) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("cron job {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn delete_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::cron::delete_job(&state.db, &id) {
        Ok(true) => Ok(axum::Json(serde_json::json!({ "deleted": true, "id": id }))),
        Ok(false) => Err((
            axum::http::StatusCode::NOT_FOUND,
            format!("cron job {id} not found"),
        )),
        Err(e) => Err(internal_err(&e)),
    }
}
