use axum::{
    extract::{Path, Query, State},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AppState, JsonError, bad_request, internal_err, not_found, validate_long, validate_short,
};

#[derive(Deserialize)]
pub struct CreateCronJobRequest {
    pub name: String,
    pub description: Option<String>,
    pub agent_id: Option<String>,
    pub schedule_kind: String,
    #[serde(alias = "schedule")]
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
    pub description: Option<String>,
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
                        "payload_json": j.payload_json,
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
    validate_short("name", &body.name)?;
    if let Some(ref d) = body.description {
        validate_long("description", d)?;
    }
    if let Some(ref a) = body.agent_id {
        validate_short("agent_id", a)?;
    }
    // BUG-013: Validate payload_json is valid JSON before storing.
    // Default to a valid executable payload so newly created jobs do useful work
    // instead of entering unknown-action failure loops.
    let payload = match body.payload_json.as_deref() {
        Some(raw) if !raw.trim().is_empty() => {
            if serde_json::from_str::<serde_json::Value>(raw).is_err() {
                return Err(bad_request("payload_json must be valid JSON"));
            }
            raw.to_string()
        }
        _ => serde_json::json!({
            "action": "log",
            "message": format!("scheduled job: {}", body.name)
        })
        .to_string(),
    };
    // BUG-012: Validate schedule_kind is a known value.
    let schedule_kind = normalize_schedule_kind(&body.schedule_kind);
    if !matches!(schedule_kind, "cron" | "every" | "once") {
        return Err(bad_request(format!(
            "invalid schedule_kind '{}': must be one of cron, every, once, interval",
            body.schedule_kind
        )));
    }
    let schedule_kind = schedule_kind.to_string();
    let schedule_expr = normalize_schedule_expr(&schedule_kind, body.schedule_expr.as_deref());
    // BUG-011: Validate cron expressions have the right number of fields.
    if schedule_kind == "cron" {
        match schedule_expr.as_deref() {
            None | Some("") => {
                return Err(bad_request(
                    "schedule_expr is required for cron schedule_kind",
                ));
            }
            Some(expr) => {
                let fields: Vec<&str> = expr.split_whitespace().collect();
                if fields.len() < 5 || fields.len() > 6 {
                    return Err(bad_request(format!(
                        "invalid cron expression: expected 5 or 6 fields, got {}",
                        fields.len()
                    )));
                }
            }
        }
    }
    let default_agent_id = {
        let cfg = state.config.read().await;
        cfg.agent.id.clone()
    };
    let agent_id = body
        .agent_id
        .as_deref()
        .unwrap_or(default_agent_id.as_str());
    match ironclad_db::cron::create_job(
        &state.db,
        &body.name,
        agent_id,
        &schedule_kind,
        schedule_expr.as_deref(),
        &payload,
    ) {
        Ok(id) => {
            let desc = body.description.as_deref().map(str::trim);
            if let Some(d) = desc {
                if !d.is_empty() {
                    let _ = ironclad_db::cron::update_job_description(&state.db, &id, Some(d));
                }
            }
            Ok(axum::Json(serde_json::json!({ "job_id": id })))
        }
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
) -> Result<impl IntoResponse, JsonError> {
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
        Ok(None) => Err(not_found(format!("cron job {id} not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn update_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<UpdateCronJobRequest>,
) -> Result<impl IntoResponse, JsonError> {
    let schedule_kind = body
        .schedule_kind
        .as_deref()
        .map(normalize_schedule_kind)
        .map(str::to_string);
    if let Some(ref d) = body.description {
        validate_long("description", d)?;
    }
    let schedule_expr = normalize_schedule_expr(
        schedule_kind
            .as_deref()
            .or(body.schedule_kind.as_deref())
            .unwrap_or("cron"),
        body.schedule_expr.as_deref(),
    );
    match ironclad_db::cron::update_job(
        &state.db,
        &id,
        body.name.as_deref(),
        schedule_kind.as_deref(),
        schedule_expr.as_deref(),
        body.enabled,
    ) {
        Ok(base_updated) => {
            let mut updated = base_updated;
            if body.description.is_some() {
                let desc = body.description.as_deref().map(str::trim);
                let changed = ironclad_db::cron::update_job_description(
                    &state.db,
                    &id,
                    desc.filter(|d| !d.is_empty()),
                )
                .map_err(|e| internal_err(&e))?;
                updated = updated || changed;
            }
            if updated {
                Ok(axum::Json(serde_json::json!({ "updated": true, "id": id })))
            } else {
                Err(not_found(format!("cron job {id} not found")))
            }
        }
        Err(e) => Err(internal_err(&e)),
    }
}

pub async fn delete_cron_job(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, JsonError> {
    match ironclad_db::cron::delete_job(&state.db, &id) {
        Ok(true) => Ok(axum::Json(serde_json::json!({ "deleted": true, "id": id }))),
        Ok(false) => Err(not_found(format!("cron job {id} not found"))),
        Err(e) => Err(internal_err(&e)),
    }
}

fn normalize_schedule_kind(kind: &str) -> &str {
    match kind.trim().to_ascii_lowercase().as_str() {
        "interval" | "every" => "every",
        "cron" => "cron",
        "once" => "once",
        _ => kind,
    }
}

fn normalize_schedule_expr(kind: &str, expr: Option<&str>) -> Option<String> {
    let expr = expr?.trim();
    if expr.is_empty() {
        return None;
    }
    if kind == "every" || kind == "interval" {
        if expr.ends_with('s') || expr.ends_with('m') || expr.ends_with('h') {
            return Some(expr.to_string());
        }
        if let Ok(n) = expr.parse::<u64>() {
            return Some(format!("{n}s"));
        }
    }
    Some(expr.to_string())
}
