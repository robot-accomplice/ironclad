use std::time::Duration;

use serde_json::Value;

use crate::api::{AppState, execute_scheduled_agent_task, subagent_integrity};

pub(crate) async fn run_cron_worker(state: AppState, instance_id: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    tracing::info!("Server cron worker started");

    loop {
        interval.tick().await;
        let jobs = match ironclad_db::cron::list_jobs(&state.db) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to list cron jobs");
                continue;
            }
        };
        let now = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        for job in &jobs {
            if !job.enabled {
                continue;
            }
            let kind = match job.schedule_kind.as_str() {
                "interval" => "every",
                other => other,
            };
            let due = match kind {
                "cron" => job
                    .schedule_expr
                    .as_deref()
                    .map(|expr| {
                        ironclad_schedule::DurableScheduler::evaluate_cron(
                            expr,
                            job.last_run_at.as_deref(),
                            &now,
                        )
                    })
                    .unwrap_or(false),
                "every" => {
                    let interval_ms = job
                        .schedule_every_ms
                        .or_else(|| {
                            parse_interval_expr_to_ms(job.schedule_expr.as_deref().unwrap_or(""))
                        })
                        .unwrap_or(60_000);
                    ironclad_schedule::DurableScheduler::evaluate_interval(
                        job.last_run_at.as_deref(),
                        interval_ms,
                        &now,
                    )
                }
                _ => false,
            };
            if !due {
                continue;
            }
            if !ironclad_db::cron::acquire_lease(&state.db, &job.id, &instance_id).unwrap_or(false)
            {
                continue;
            }
            let start = std::time::Instant::now();
            let (status, error) = execute_cron_job_once(&state, job).await;
            let duration = start.elapsed().as_millis() as i64;
            let _ = ironclad_db::cron::record_run(
                &state.db,
                &job.id,
                status,
                Some(duration),
                error.as_deref(),
            );
            let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
            let next = ironclad_schedule::DurableScheduler::calculate_next_run(
                kind,
                job.schedule_expr.as_deref(),
                job.schedule_every_ms,
                &now_str,
            );
            let _ = ironclad_db::cron::update_next_run_at(&state.db, &job.id, next.as_deref());
            let _ = ironclad_db::cron::release_lease(&state.db, &job.id, &instance_id);
        }
    }
}

pub(crate) async fn execute_cron_job_once(
    state: &AppState,
    job: &ironclad_db::cron::CronJob,
) -> (&'static str, Option<String>) {
    let payload: Value = match serde_json::from_str(&job.payload_json) {
        Ok(v) => v,
        Err(e) => return ("error", Some(format!("invalid payload: {e}"))),
    };
    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");
    match action {
        "agent_task" => execute_agent_task_for_job(state, job, &payload).await,
        "log" => {
            if let Some(task) = implied_agent_task(job, &payload) {
                execute_named_agent_task(state, &job.agent_id, &task).await
            } else {
                tracing::info!(job = %job.name, message = payload.get("message").and_then(|v| v.as_str()).unwrap_or("cron heartbeat"), "cron job executed");
                ("success", None)
            }
        }
        "metric_snapshot" => {
            let snapshot = serde_json::json!({"job_id": job.id, "job_name": job.name, "schedule_kind": job.schedule_kind, "timestamp": chrono::Utc::now().to_rfc3339()});
            match ironclad_db::metrics::record_metric_snapshot(&state.db, &snapshot.to_string()) {
                Ok(_) => ("success", None),
                Err(e) => ("error", Some(format!("metric_snapshot failed: {e}"))),
            }
        }
        "expire_sessions" => {
            let ttl_seconds = payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(86_400);
            match ironclad_db::sessions::expire_stale_sessions(&state.db, ttl_seconds) {
                Ok(_) => ("success", None),
                Err(e) => ("error", Some(format!("expire_sessions failed: {e}"))),
            }
        }
        "record_transaction" => {
            let tx_type = payload
                .get("tx_type")
                .and_then(|v| v.as_str())
                .unwrap_or("cron");
            let amount = payload
                .get("amount")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let currency = payload
                .get("currency")
                .and_then(|v| v.as_str())
                .unwrap_or("USD");
            let counterparty = payload.get("counterparty").and_then(|v| v.as_str());
            let tx_hash = payload.get("tx_hash").and_then(|v| v.as_str());
            match ironclad_db::metrics::record_transaction(
                &state.db,
                tx_type,
                amount,
                currency,
                counterparty,
                tx_hash,
            ) {
                Ok(_) => ("success", None),
                Err(e) => ("error", Some(format!("record_transaction failed: {e}"))),
            }
        }
        "noop" => ("success", None),
        other => ("error", Some(format!("unknown action: {other}"))),
    }
}

async fn execute_agent_task_for_job(
    state: &AppState,
    job: &ironclad_db::cron::CronJob,
    payload: &Value,
) -> (&'static str, Option<String>) {
    let task = payload
        .get("task")
        .and_then(|v| v.as_str())
        .or_else(|| payload.get("prompt").and_then(|v| v.as_str()))
        .or_else(|| payload.get("message").and_then(|v| v.as_str()))
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .or(job
            .description
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty()));
    let Some(task) = task else {
        return (
            "error",
            Some("agent_task payload missing task/prompt/message".to_string()),
        );
    };
    execute_named_agent_task(state, &job.agent_id, task).await
}

async fn execute_named_agent_task(
    state: &AppState,
    agent_id: &str,
    task: &str,
) -> (&'static str, Option<String>) {
    if let Ok(subagents) = ironclad_db::agents::list_sub_agents(&state.db)
        && let Some(sa) = subagents
            .into_iter()
            .find(|sa| sa.name.eq_ignore_ascii_case(agent_id) && sa.enabled)
    {
        if let Err(err) = subagent_integrity::ensure_taskable_subagent_ready(state, &sa).await {
            return (
                "error",
                Some(format!("subagent integrity repair failed: {err}")),
            );
        }
    }
    match execute_scheduled_agent_task(state, agent_id, task).await {
        Ok(_) => ("success", None),
        Err(err) => ("error", Some(err)),
    }
}

fn implied_agent_task(job: &ironclad_db::cron::CronJob, payload: &Value) -> Option<String> {
    let description = job
        .description
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    let message = payload
        .get("message")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .unwrap_or("");
    if message.eq_ignore_ascii_case(description) || message.starts_with("scheduled job:") {
        return Some(description.to_string());
    }
    None
}

fn parse_interval_expr_to_ms(expr: &str) -> Option<i64> {
    if expr.is_empty() {
        return None;
    }
    let (unit_byte_offset, unit) = expr.char_indices().last()?;
    let qty = expr[..unit_byte_offset].parse::<i64>().ok()?;
    Some(match unit {
        's' | 'S' => qty.saturating_mul(1_000),
        'm' | 'M' => qty.saturating_mul(60_000),
        'h' | 'H' => qty.saturating_mul(3_600_000),
        _ => return None,
    })
}
