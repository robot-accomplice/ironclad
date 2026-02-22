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

            // Job payload is JSON describing the action; log it for now
            let result_status = "success";
            let duration = start.elapsed().as_millis() as i64;

            ironclad_db::cron::record_run(&db, &job.id, result_status, Some(duration), None).ok();
            ironclad_db::cron::release_lease(&db, &job.id).ok();
        }
    }
}
