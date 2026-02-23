pub mod heartbeat;
pub mod scheduler;
pub mod tasks;

pub use heartbeat::run as run_heartbeat;
pub use heartbeat::{HeartbeatDaemon, TickContext};
pub use scheduler::DurableScheduler;
pub use tasks::{HeartbeatTask, TaskResult};

/// Cron worker loop: evaluates due jobs, acquires leases, executes, and records results.
pub async fn run_cron_worker(db: ironclad_db::Database, instance_id: String) {
    use std::time::Duration;

    let mut interval = tokio::time::interval(Duration::from_secs(60));
    tracing::info!("Cron worker started");

    loop {
        interval.tick().await;

        let jobs = match ironclad_db::cron::list_jobs(&db) {
            Ok(j) => j,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to list cron jobs");
                continue;
            }
        };

        let now = chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string();

        for job in &jobs {
            if !job.enabled {
                continue;
            }

            let due = match job.schedule_kind.as_str() {
                "cron" => job
                    .schedule_expr
                    .as_deref()
                    .map(|expr| {
                        DurableScheduler::evaluate_cron(expr, job.last_run_at.as_deref(), &now)
                    })
                    .unwrap_or(false),
                "every" => {
                    let interval_ms = job.schedule_every_ms.unwrap_or(60_000);
                    DurableScheduler::evaluate_interval(
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

            if !ironclad_db::cron::acquire_lease(&db, &job.id, &instance_id).unwrap_or(false) {
                continue;
            }

            tracing::debug!(job = %job.name, "Executing cron job");
            let start = std::time::Instant::now();

            let (result_status, error_msg) = execute_cron_job(job);
            let duration = start.elapsed().as_millis() as i64;

            ironclad_db::cron::record_run(
                &db,
                &job.id,
                result_status,
                Some(duration),
                error_msg.as_deref(),
            )
            .ok();
            ironclad_db::cron::release_lease(&db, &job.id).ok();
        }
    }
}

/// Execute a cron job based on its payload. Returns (status, optional error message).
fn execute_cron_job(job: &ironclad_db::cron::CronJob) -> (&'static str, Option<String>) {
    let payload: serde_json::Value = match serde_json::from_str(&job.payload_json) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(job = %job.name, error = %e, "invalid job payload JSON");
            return ("error", Some(format!("invalid payload: {e}")));
        }
    };

    let action = payload
        .get("action")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    match action {
        "log" => {
            let message = payload
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("cron heartbeat");
            tracing::info!(job = %job.name, message, "cron job executed");
            ("success", None)
        }
        "noop" => {
            tracing::debug!(job = %job.name, "noop cron job");
            ("success", None)
        }
        other => {
            tracing::warn!(job = %job.name, action = other, "unknown cron action");
            ("error", Some(format!("unknown action: {other}")))
        }
    }
}
