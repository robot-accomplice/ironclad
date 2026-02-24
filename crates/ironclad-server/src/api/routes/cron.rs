use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{AppState, internal_err};

#[derive(Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub agent_id: Option<String>,
    pub schedule_kind: String,
    pub schedule_expr: Option<String>,
    pub payload_json: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CronRunsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub job_id: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CronRunItem {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    pub status: String,
    pub duration_ms: Option<i64>,
    pub error: Option<String>,
    pub created_at: String,
    pub day: String,
}

#[derive(Deserialize)]
pub struct UpdateCronJobRequest {
    pub name: Option<String>,
    pub schedule_kind: Option<String>,
    pub schedule_expr: Option<String>,
    pub enabled: Option<bool>,
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
    let agent_id = body.agent_id.as_deref().unwrap_or("ironclad");
    match ironclad_db::cron::create_job(
        &state.db,
        &body.name,
        agent_id,
        &body.schedule_kind,
        body.schedule_expr.as_deref(),
        payload,
    ) {
        Ok(id) => Ok(axum::Json(serde_json::json!({ "job_id": id }))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn list_cron_runs(
    State(state): State<AppState>,
    Query(params): Query<CronRunsQuery>,
) -> impl IntoResponse {
    let limit = params.limit.unwrap_or(1000).clamp(1, 5000);
    let jobs = match ironclad_db::cron::list_jobs(&state.db) {
        Ok(j) => j,
        Err(e) => return Err(internal_err(&e)),
    };
    let job_name_by_id: std::collections::HashMap<String, String> =
        jobs.into_iter().map(|j| (j.id, j.name)).collect();
    match ironclad_db::cron::list_runs(
        &state.db,
        params.from.as_deref(),
        params.to.as_deref(),
        params.job_id.as_deref(),
        limit,
    ) {
        Ok(runs) => {
            let items: Vec<CronRunItem> = runs
                .into_iter()
                .map(|r| {
                    let day = r.created_at.split(' ').next().unwrap_or("").to_string();
                    CronRunItem {
                        id: r.id,
                        job_id: r.job_id.clone(),
                        job_name: job_name_by_id
                            .get(&r.job_id)
                            .cloned()
                            .unwrap_or_else(|| "unknown".to_string()),
                        status: r.status,
                        duration_ms: r.duration_ms,
                        error: r.error,
                        created_at: r.created_at,
                        day,
                    }
                })
                .collect();
            Ok(axum::Json(serde_json::json!({ "runs": items })))
        }
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

pub async fn update_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<UpdateCronJobRequest>,
) -> Result<impl IntoResponse, (axum::http::StatusCode, String)> {
    match ironclad_db::cron::update_job(
        &state.db,
        &id,
        body.name.as_deref(),
        body.schedule_kind.as_deref(),
        body.schedule_expr.as_deref(),
        body.enabled,
    ) {
        Ok(true) => Ok(axum::Json(serde_json::json!({ "updated": true, "id": id }))),
        Ok(false) => Err((
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
