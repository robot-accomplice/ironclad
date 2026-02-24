//! # ironclad-schedule
//!
//! Unified cron/heartbeat scheduler for the Ironclad agent runtime. Jobs are
//! persisted in SQLite (`ironclad-db/cron.rs`) with lease-based mutual
//! exclusion to prevent duplicate execution across restarts.
//!
//! ## Key Types
//!
//! - [`HeartbeatDaemon`] -- Periodic tick loop driving registered heartbeat tasks
//! - [`DurableScheduler`] -- Cron expression and fixed-interval evaluation
//! - [`HeartbeatTask`] / [`TaskResult`] -- Pluggable task trait and outcome type
//!
//! ## Modules
//!
//! - `heartbeat` -- Heartbeat daemon loop with wallet and DB context
//! - `scheduler` -- Cron expression parsing (`evaluate_cron`) and interval checks
//! - `tasks` -- `HeartbeatTask` trait and built-in task implementations
//!
//! ## Entry Points
//!
//! - [`run_heartbeat()`] -- Start the heartbeat daemon
//! - [`run_cron_worker()`] -- Start the cron worker (lease, execute, record)

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

            let kind = normalize_schedule_kind(&job.schedule_kind);
            let due = match kind {
                "cron" => job
                    .schedule_expr
                    .as_deref()
                    .map(|expr| {
                        DurableScheduler::evaluate_cron(expr, job.last_run_at.as_deref(), &now)
                    })
                    .unwrap_or(false),
                "every" => {
                    let interval_ms = job
                        .schedule_every_ms
                        .or_else(|| {
                            job.schedule_expr
                                .as_deref()
                                .and_then(parse_interval_expr_to_ms)
                        })
                        .unwrap_or(60_000);
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

            let (result_status, error_msg) = execute_cron_job(&db, job);
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
fn execute_cron_job(
    db: &ironclad_db::Database,
    job: &ironclad_db::cron::CronJob,
) -> (&'static str, Option<String>) {
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
        "metric_snapshot" => {
            let snapshot = serde_json::json!({
                "job_id": job.id,
                "job_name": job.name,
                "schedule_kind": job.schedule_kind,
                "timestamp": chrono::Utc::now().to_rfc3339(),
            });
            match ironclad_db::metrics::record_metric_snapshot(db, &snapshot.to_string()) {
                Ok(_) => ("success", None),
                Err(e) => ("error", Some(format!("metric_snapshot failed: {e}"))),
            }
        }
        "expire_sessions" => {
            let ttl_seconds = payload
                .get("ttl_seconds")
                .and_then(|v| v.as_u64())
                .unwrap_or(86_400);
            match ironclad_db::sessions::expire_stale_sessions(db, ttl_seconds) {
                Ok(expired) => {
                    tracing::info!(job = %job.name, expired, ttl_seconds, "expired stale sessions");
                    ("success", None)
                }
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
                db,
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

fn normalize_schedule_kind(kind: &str) -> &str {
    match kind {
        "interval" => "every",
        "every" | "cron" => kind,
        _ => kind,
    }
}

fn parse_interval_expr_to_ms(expr: &str) -> Option<i64> {
    if expr.is_empty() {
        return None;
    }
    let unit = expr.chars().last()?;
    let qty = expr[..expr.len().saturating_sub(1)].parse::<i64>().ok()?;
    let ms = match unit {
        's' | 'S' => qty.saturating_mul(1_000),
        'm' | 'M' => qty.saturating_mul(60_000),
        'h' | 'H' => qty.saturating_mul(3_600_000),
        _ => return None,
    };
    if ms > 0 { Some(ms) } else { None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclad_db::Database;

    fn test_db() -> Database {
        Database::new(":memory:").expect("in-memory db")
    }

    fn job_with_payload(
        db: &Database,
        name: &str,
        payload_json: &str,
    ) -> ironclad_db::cron::CronJob {
        let job_id = ironclad_db::cron::create_job(
            db,
            name,
            "test-agent",
            "every",
            Some("5m"),
            payload_json,
        )
        .expect("create job");
        ironclad_db::cron::get_job(db, &job_id)
            .expect("get job")
            .expect("job exists")
    }

    #[test]
    fn normalize_schedule_kind_maps_interval_to_every() {
        assert_eq!(normalize_schedule_kind("interval"), "every");
        assert_eq!(normalize_schedule_kind("every"), "every");
        assert_eq!(normalize_schedule_kind("cron"), "cron");
        assert_eq!(normalize_schedule_kind("custom"), "custom");
    }

    #[test]
    fn parse_interval_expr_to_ms_parses_supported_units() {
        assert_eq!(parse_interval_expr_to_ms("5s"), Some(5_000));
        assert_eq!(parse_interval_expr_to_ms("2m"), Some(120_000));
        assert_eq!(parse_interval_expr_to_ms("3h"), Some(10_800_000));
        assert_eq!(parse_interval_expr_to_ms("7S"), Some(7_000));
        assert_eq!(parse_interval_expr_to_ms("1M"), Some(60_000));
    }

    #[test]
    fn parse_interval_expr_to_ms_rejects_invalid_values() {
        assert_eq!(parse_interval_expr_to_ms(""), None);
        assert_eq!(parse_interval_expr_to_ms("10"), None);
        assert_eq!(parse_interval_expr_to_ms("xs"), None);
        assert_eq!(parse_interval_expr_to_ms("0s"), None);
        assert_eq!(parse_interval_expr_to_ms("-5m"), None);
    }

    #[test]
    fn execute_cron_job_rejects_invalid_payload_json() {
        let db = test_db();
        let job = job_with_payload(&db, "bad-json", "{not-json}");
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "error");
        assert!(error.unwrap_or_default().contains("invalid payload"));
    }

    #[test]
    fn execute_cron_job_handles_log_and_noop_actions() {
        let db = test_db();
        let log_job = job_with_payload(&db, "log-job", r#"{"action":"log","message":"hello"}"#);
        let (status, error) = execute_cron_job(&db, &log_job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let noop_job = job_with_payload(&db, "noop-job", r#"{"action":"noop"}"#);
        let (status, error) = execute_cron_job(&db, &noop_job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    #[test]
    fn execute_cron_job_records_metric_snapshot() {
        let db = test_db();
        let job = job_with_payload(&db, "metrics-job", r#"{"action":"metric_snapshot"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM metric_snapshots", [], |row| {
                row.get(0)
            })
            .expect("count snapshots");
        assert_eq!(count, 1);
    }

    #[test]
    fn execute_cron_job_expires_stale_sessions() {
        let db = test_db();
        let session_id = ironclad_db::sessions::find_or_create(&db, "expire-agent", None)
            .expect("create session");
        db.conn()
            .execute(
                "UPDATE sessions SET updated_at = datetime('now', '-2 days') WHERE id = ?1",
                [&session_id],
            )
            .expect("age session");

        let job = job_with_payload(
            &db,
            "expire-job",
            r#"{"action":"expire_sessions","ttl_seconds":60}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let status: String = db
            .conn()
            .query_row(
                "SELECT status FROM sessions WHERE id = ?1",
                [&session_id],
                |row| row.get(0),
            )
            .expect("session status");
        assert_eq!(status, "expired");
    }

    #[test]
    fn execute_cron_job_records_transaction() {
        let db = test_db();
        let job = job_with_payload(
            &db,
            "tx-job",
            r#"{"action":"record_transaction","tx_type":"ops","amount":1.25,"currency":"USD","counterparty":"scheduler"}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let txs = ironclad_db::metrics::query_transactions(&db, 24).expect("query txs");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].tx_type, "ops");
        assert_eq!(txs[0].currency, "USD");
    }

    #[test]
    fn execute_cron_job_rejects_unknown_action() {
        let db = test_db();
        let job = job_with_payload(&db, "unknown-job", r#"{"action":"mystery"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "error");
        assert!(error.unwrap_or_default().contains("unknown action"));
    }
}
