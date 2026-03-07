use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::sync::Semaphore;

use crate::api::{AppState, execute_scheduled_agent_task, subagent_integrity};

pub(crate) async fn run_cron_worker(state: AppState, instance_id: String) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let concurrency = Arc::new(Semaphore::new(4));
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
            let Ok(permit) = concurrency.clone().try_acquire_owned() else {
                let _ = ironclad_db::cron::release_lease(&state.db, &job.id, &instance_id);
                tracing::warn!(job=%job.name, "Cron worker saturated; deferring leased job to next tick");
                continue;
            };
            let state_clone = state.clone();
            let job_clone = job.clone();
            let instance_id_clone = instance_id.clone();
            let kind = kind.to_string();
            tokio::spawn(async move {
                let _permit = permit;
                let start = std::time::Instant::now();
                let result = execute_cron_job_once(&state_clone, &job_clone).await;
                let duration = start.elapsed().as_millis() as i64;
                let _ = ironclad_db::cron::record_run(
                    &state_clone.db,
                    &job_clone.id,
                    result.status,
                    Some(duration),
                    result.error.as_deref(),
                    result.output.as_deref(),
                );
                let now_str = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();
                let next = ironclad_schedule::DurableScheduler::calculate_next_run(
                    &kind,
                    job_clone.schedule_expr.as_deref(),
                    job_clone.schedule_every_ms,
                    &now_str,
                );
                let _ = ironclad_db::cron::update_next_run_at(
                    &state_clone.db,
                    &job_clone.id,
                    next.as_deref(),
                );
                let _ = ironclad_db::cron::release_lease(
                    &state_clone.db,
                    &job_clone.id,
                    &instance_id_clone,
                );
            });
        }
    }
}

pub(crate) struct CronExecutionResult {
    pub status: &'static str,
    pub error: Option<String>,
    pub output: Option<String>,
}

pub(crate) async fn execute_cron_job_once(
    state: &AppState,
    job: &ironclad_db::cron::CronJob,
) -> CronExecutionResult {
    let payload: Value = match serde_json::from_str(&job.payload_json) {
        Ok(v) => v,
        Err(e) => {
            return CronExecutionResult {
                status: "error",
                error: Some(format!("invalid payload: {e}")),
                output: None,
            };
        }
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
                let message = payload
                    .get("message")
                    .and_then(|v| v.as_str())
                    .unwrap_or("cron heartbeat");
                tracing::info!(job = %job.name, message, "cron job executed");
                CronExecutionResult {
                    status: "success",
                    error: None,
                    output: Some(message.to_string()),
                }
            }
        }
        "metric_snapshot" => {
            let snapshot = serde_json::json!({"job_id": job.id, "job_name": job.name, "schedule_kind": job.schedule_kind, "timestamp": chrono::Utc::now().to_rfc3339()});
            match ironclad_db::metrics::record_metric_snapshot(&state.db, &snapshot.to_string()) {
                Ok(_) => CronExecutionResult {
                    status: "success",
                    error: None,
                    output: Some("metric snapshot recorded".to_string()),
                },
                Err(e) => CronExecutionResult {
                    status: "error",
                    error: Some(format!("metric_snapshot failed: {e}")),
                    output: None,
                },
            }
        }
        "expire_sessions" => {
            let ttl_seconds = payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(86_400);
            match ironclad_db::sessions::expire_stale_sessions(&state.db, ttl_seconds) {
                Ok(expired) => CronExecutionResult {
                    status: "success",
                    error: None,
                    output: Some(format!("expired {expired} stale sessions")),
                },
                Err(e) => CronExecutionResult {
                    status: "error",
                    error: Some(format!("expire_sessions failed: {e}")),
                    output: None,
                },
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
                Ok(_) => CronExecutionResult {
                    status: "success",
                    error: None,
                    output: Some(format!("transaction recorded: {amount} {currency}")),
                },
                Err(e) => CronExecutionResult {
                    status: "error",
                    error: Some(format!("record_transaction failed: {e}")),
                    output: None,
                },
            }
        }
        "noop" => CronExecutionResult {
            status: "success",
            error: None,
            output: None,
        },
        other => CronExecutionResult {
            status: "error",
            error: Some(format!("unknown action: {other}")),
            output: None,
        },
    }
}

async fn execute_agent_task_for_job(
    state: &AppState,
    job: &ironclad_db::cron::CronJob,
    payload: &Value,
) -> CronExecutionResult {
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
        return CronExecutionResult {
            status: "error",
            error: Some("agent_task payload missing task/prompt/message".to_string()),
            output: None,
        };
    };
    execute_named_agent_task(state, &job.agent_id, task).await
}

async fn execute_named_agent_task(
    state: &AppState,
    agent_id: &str,
    task: &str,
) -> CronExecutionResult {
    match ironclad_db::agents::list_sub_agents(&state.db) {
        Ok(subagents) => {
            if let Some(sa) = subagents
                .into_iter()
                .find(|sa| sa.name.eq_ignore_ascii_case(agent_id) && sa.enabled)
                && let Err(err) =
                    subagent_integrity::ensure_taskable_subagent_ready(state, &sa).await
            {
                return CronExecutionResult {
                    status: "error",
                    error: Some(format!("subagent integrity repair failed: {err}")),
                    output: None,
                };
            }
        }
        Err(e) => {
            tracing::warn!(agent_id, error = %e, "failed to list sub-agents for cron task; proceeding without integrity check");
        }
    }
    match execute_scheduled_agent_task(state, agent_id, task).await {
        Ok(output) => CronExecutionResult {
            status: "success",
            error: None,
            output: Some(output),
        },
        Err(err) => CronExecutionResult {
            status: "error",
            error: Some(err),
            output: None,
        },
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
    if message.eq_ignore_ascii_case(description)
        || message.to_ascii_lowercase().starts_with("scheduled job:")
    {
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
