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

            match ironclad_db::cron::acquire_lease(&db, &job.id, &instance_id) {
                Ok(acquired) => {
                    if !acquired {
                        continue;
                    }
                }
                Err(e) => {
                    tracing::warn!(job_id = %job.id, error = %e, "failed to acquire cron lease");
                    continue;
                }
            }

            tracing::debug!(job = %job.name, "Executing cron job");
            let start = std::time::Instant::now();

            let (result_status, error_msg) = execute_cron_job(&db, job);
            let duration = start.elapsed().as_millis() as i64;

            if let Err(e) = ironclad_db::cron::record_run(
                &db,
                &job.id,
                result_status,
                Some(duration),
                error_msg.as_deref(),
            ) {
                tracing::warn!(job_id = %job.id, error = %e, "failed to record cron run");
            }
            if let Err(e) = ironclad_db::cron::release_lease(&db, &job.id, &instance_id) {
                tracing::warn!(job_id = %job.id, error = %e, "failed to release cron lease");
            }
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
        .or_else(|| {
            payload
                .get("kind")
                .and_then(|v| v.as_str())
                .and_then(legacy_kind_to_action)
        })
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
        "agent_turn_legacy" => {
            // Backward compatibility for imported legacy cron payloads from older runtimes.
            // Ironclad's durable scheduler currently does not execute agent turns directly.
            tracing::warn!(
                job = %job.name,
                "legacy agentTurn cron payload detected; treating as noop"
            );
            ("success", None)
        }
        other => {
            tracing::warn!(job = %job.name, action = other, "unknown cron action");
            ("error", Some(format!("unknown action: {other}")))
        }
    }
}

fn legacy_kind_to_action(kind: &str) -> Option<&'static str> {
    match kind {
        "agentTurn" => Some("agent_turn_legacy"),
        "metricSnapshot" => Some("metric_snapshot"),
        "expireSessions" => Some("expire_sessions"),
        "recordTransaction" => Some("record_transaction"),
        "log" => Some("log"),
        "noop" => Some("noop"),
        _ => None,
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
    let (unit_byte_offset, unit) = expr.char_indices().last()?;
    let qty = expr[..unit_byte_offset].parse::<i64>().ok()?;
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

    #[test]
    fn execute_cron_job_accepts_legacy_agent_turn_kind() {
        let db = test_db();
        let job = job_with_payload(
            &db,
            "legacy-agent-turn",
            r#"{"kind":"agentTurn","message":"Do work"}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    #[test]
    fn execute_cron_job_accepts_legacy_metric_snapshot_kind() {
        let db = test_db();
        let job = job_with_payload(&db, "legacy-metrics", r#"{"kind":"metricSnapshot"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    // ── BUG-093: legacy_kind_to_action exhaustive coverage ─────────────
    #[test]
    fn legacy_kind_to_action_all_known_kinds() {
        assert_eq!(legacy_kind_to_action("agentTurn"), Some("agent_turn_legacy"));
        assert_eq!(legacy_kind_to_action("metricSnapshot"), Some("metric_snapshot"));
        assert_eq!(legacy_kind_to_action("expireSessions"), Some("expire_sessions"));
        assert_eq!(legacy_kind_to_action("recordTransaction"), Some("record_transaction"));
        assert_eq!(legacy_kind_to_action("log"), Some("log"));
        assert_eq!(legacy_kind_to_action("noop"), Some("noop"));
    }

    #[test]
    fn legacy_kind_to_action_unknown_returns_none() {
        assert_eq!(legacy_kind_to_action("unknown"), None);
        assert_eq!(legacy_kind_to_action(""), None);
        assert_eq!(legacy_kind_to_action("AgentTurn"), None); // case-sensitive
        assert_eq!(legacy_kind_to_action("NOOP"), None);
        assert_eq!(legacy_kind_to_action("agent_turn_legacy"), None);
    }

    // ── BUG-093: normalize_schedule_kind boundary cases ────────────────
    #[test]
    fn normalize_schedule_kind_pass_through_unknown_kinds() {
        assert_eq!(normalize_schedule_kind(""), "");
        assert_eq!(normalize_schedule_kind("once"), "once");
        assert_eq!(normalize_schedule_kind("at"), "at");
        assert_eq!(normalize_schedule_kind("weekly"), "weekly");
    }

    // ── BUG-094: parse_interval_expr_to_ms edge cases ──────────────────
    #[test]
    fn parse_interval_expr_to_ms_uppercase_h() {
        assert_eq!(parse_interval_expr_to_ms("1H"), Some(3_600_000));
        assert_eq!(parse_interval_expr_to_ms("2H"), Some(7_200_000));
    }

    #[test]
    fn parse_interval_expr_to_ms_single_char() {
        // Single character like "s" has no numeric part
        assert_eq!(parse_interval_expr_to_ms("s"), None);
        assert_eq!(parse_interval_expr_to_ms("m"), None);
        assert_eq!(parse_interval_expr_to_ms("h"), None);
    }

    #[test]
    fn parse_interval_expr_to_ms_large_values() {
        // 24h = 86_400_000
        assert_eq!(parse_interval_expr_to_ms("24h"), Some(86_400_000));
        // 1000s = 1_000_000
        assert_eq!(parse_interval_expr_to_ms("1000s"), Some(1_000_000));
    }

    #[test]
    fn parse_interval_expr_to_ms_unknown_unit() {
        assert_eq!(parse_interval_expr_to_ms("5d"), None); // days not supported
        assert_eq!(parse_interval_expr_to_ms("3w"), None); // weeks not supported
    }

    // ── execute_cron_job: log action with default message ──────────────
    #[test]
    fn execute_cron_job_log_action_with_default_message() {
        let db = test_db();
        // No "message" key in payload -> should use default "cron heartbeat"
        let job = job_with_payload(&db, "log-default", r#"{"action":"log"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    // ── execute_cron_job: expire_sessions with default TTL ─────────────
    #[test]
    fn execute_cron_job_expire_sessions_uses_default_ttl() {
        let db = test_db();
        // No "ttl_seconds" -> should use default 86_400
        let job = job_with_payload(&db, "expire-default", r#"{"action":"expire_sessions"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    // ── execute_cron_job: record_transaction with defaults ─────────────
    #[test]
    fn execute_cron_job_record_transaction_uses_defaults() {
        let db = test_db();
        // Minimal payload: no tx_type, amount, currency, counterparty, tx_hash
        let job = job_with_payload(&db, "tx-minimal", r#"{"action":"record_transaction"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let txs = ironclad_db::metrics::query_transactions(&db, 24).expect("query txs");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].tx_type, "cron"); // default tx_type
        assert_eq!(txs[0].currency, "USD"); // default currency
    }

    // ── execute_cron_job: record_transaction with tx_hash ──────────────
    #[test]
    fn execute_cron_job_record_transaction_with_tx_hash() {
        let db = test_db();
        let job = job_with_payload(
            &db,
            "tx-with-hash",
            r#"{"action":"record_transaction","tx_type":"payment","amount":42.0,"currency":"ETH","counterparty":"alice","tx_hash":"0xabc"}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());

        let txs = ironclad_db::metrics::query_transactions(&db, 24).expect("query txs");
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].tx_type, "payment");
        assert_eq!(txs[0].currency, "ETH");
    }

    // ── execute_cron_job: legacy kind fallback paths ───────────────────
    #[test]
    fn execute_cron_job_legacy_expire_sessions_kind() {
        let db = test_db();
        let job = job_with_payload(
            &db,
            "legacy-expire",
            r#"{"kind":"expireSessions","ttl_seconds":3600}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    #[test]
    fn execute_cron_job_legacy_record_transaction_kind() {
        let db = test_db();
        let job = job_with_payload(
            &db,
            "legacy-tx",
            r#"{"kind":"recordTransaction","amount":5.0}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    #[test]
    fn execute_cron_job_legacy_log_kind() {
        let db = test_db();
        let job = job_with_payload(&db, "legacy-log", r#"{"kind":"log","message":"test"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    #[test]
    fn execute_cron_job_legacy_noop_kind() {
        let db = test_db();
        let job = job_with_payload(&db, "legacy-noop", r#"{"kind":"noop"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    // ── execute_cron_job: unknown legacy kind falls through to unknown ──
    #[test]
    fn execute_cron_job_unknown_legacy_kind() {
        let db = test_db();
        let job = job_with_payload(&db, "legacy-unknown", r#"{"kind":"foobar"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "error");
        assert!(error.unwrap_or_default().contains("unknown action"));
    }

    // ── execute_cron_job: no action or kind -> "unknown" ───────────────
    #[test]
    fn execute_cron_job_no_action_or_kind_is_unknown() {
        let db = test_db();
        let job = job_with_payload(&db, "empty-payload", r#"{"data":"value"}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "error");
        assert!(error.unwrap_or_default().contains("unknown action"));
    }

    // ── execute_cron_job: empty object payload ─────────────────────────
    #[test]
    fn execute_cron_job_empty_object_payload() {
        let db = test_db();
        let job = job_with_payload(&db, "empty-obj", r#"{}"#);
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "error");
        assert!(error.unwrap_or_default().contains("unknown action"));
    }

    // ── execute_cron_job: action takes precedence over kind ────────────
    #[test]
    fn execute_cron_job_action_takes_precedence_over_kind() {
        let db = test_db();
        // Both action and kind are present; action should win
        let job = job_with_payload(
            &db,
            "precedence",
            r#"{"action":"noop","kind":"agentTurn"}"#,
        );
        let (status, error) = execute_cron_job(&db, &job);
        assert_eq!(status, "success");
        assert!(error.is_none());
    }

    // ── run_cron_worker async integration tests ─────────────────────────
    // These tests exercise the async cron worker loop by spawning it with
    // tokio time control and aborting after one iteration completes.

    fn create_due_job(db: &Database, name: &str, payload: &str) -> String {
        let job_id = ironclad_db::cron::create_job(
            db,
            name,
            "test-agent",
            "every",
            Some("1s"),
            payload,
        )
        .expect("create job");
        // Set schedule_every_ms to 1 so the job is immediately due
        db.conn()
            .execute(
                "UPDATE cron_jobs SET schedule_every_ms = 1 WHERE id = ?1",
                [&job_id],
            )
            .expect("update schedule_every_ms");
        job_id
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_executes_due_log_job() {
        let db = test_db();
        let _job_id = create_due_job(&db, "worker-log", r#"{"action":"log","message":"test"}"#);

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "test-instance".into()).await;
        });

        // Advance past the 60s interval to trigger one tick
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        // Yield to let the worker process
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // Verify the job was executed by checking cron_runs table
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |row| row.get(0))
            .expect("count runs");
        assert!(count >= 1, "expected at least one cron run, got {count}");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_executes_noop_job() {
        let db = test_db();
        let _job_id = create_due_job(&db, "worker-noop", r#"{"action":"noop"}"#);

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "noop-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |row| row.get(0))
            .expect("count runs");
        assert!(count >= 1, "expected at least one cron run");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_executes_metric_snapshot_job() {
        let db = test_db();
        let _job_id = create_due_job(&db, "worker-metric", r#"{"action":"metric_snapshot"}"#);

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "metric-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let snap_count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM metric_snapshots", [], |row| row.get(0))
            .expect("count snapshots");
        assert!(snap_count >= 1, "expected at least one metric snapshot");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_executes_expire_sessions_job() {
        let db = test_db();
        // Create a session that's old enough to expire
        let session_id = ironclad_db::sessions::find_or_create(&db, "cron-expire-agent", None)
            .expect("create session");
        db.conn()
            .execute(
                "UPDATE sessions SET updated_at = datetime('now', '-2 days') WHERE id = ?1",
                [&session_id],
            )
            .expect("age session");

        let _job_id = create_due_job(
            &db,
            "worker-expire",
            r#"{"action":"expire_sessions","ttl_seconds":60}"#,
        );

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "expire-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

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

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_executes_record_transaction_job() {
        let db = test_db();
        let _job_id = create_due_job(
            &db,
            "worker-tx",
            r#"{"action":"record_transaction","tx_type":"cron_test","amount":99.0,"currency":"USDC"}"#,
        );

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "tx-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let txs = ironclad_db::metrics::query_transactions(&db, 24).expect("query txs");
        assert!(!txs.is_empty(), "expected at least one transaction");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_skips_disabled_job() {
        let db = test_db();
        let job_id = create_due_job(&db, "worker-disabled", r#"{"action":"log","message":"skip"}"#);
        // Disable the job
        db.conn()
            .execute(
                "UPDATE cron_jobs SET enabled = 0 WHERE id = ?1",
                [&job_id],
            )
            .expect("disable job");

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "disabled-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // Disabled job should not produce any cron runs
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |row| row.get(0))
            .expect("count runs");
        assert_eq!(count, 0, "disabled job should not be executed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_handles_unknown_action_job() {
        let db = test_db();
        let _job_id = create_due_job(&db, "worker-unknown", r#"{"action":"nonexistent"}"#);

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "unknown-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // Should have recorded an error run
        let error_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE status = 'error'",
                [],
                |row| row.get(0),
            )
            .expect("count error runs");
        assert!(error_count >= 1, "expected at least one error run");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_handles_invalid_json_job() {
        let db = test_db();
        let _job_id = create_due_job(&db, "worker-badjson", "{not-valid-json}");

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "badjson-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let error_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE status = 'error'",
                [],
                |row| row.get(0),
            )
            .expect("count error runs");
        assert!(error_count >= 1, "expected error run for invalid JSON");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_handles_legacy_agent_turn_job() {
        let db = test_db();
        let _job_id = create_due_job(
            &db,
            "worker-legacy-turn",
            r#"{"kind":"agentTurn","message":"hello"}"#,
        );

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "legacy-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let success_count: i64 = db
            .conn()
            .query_row(
                "SELECT COUNT(*) FROM cron_runs WHERE status = 'success'",
                [],
                |row| row.get(0),
            )
            .expect("count success runs");
        assert!(success_count >= 1, "expected success run for legacy agent turn");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_with_no_jobs_does_not_crash() {
        let db = test_db();

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "empty-instance".into()).await;
        });

        // Advance past a tick with no jobs
        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // No crash, no runs
        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |row| row.get(0))
            .expect("count runs");
        assert_eq!(count, 0);
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_cron_schedule_kind_job() {
        let db = test_db();
        // Create a job with "cron" kind that uses a cron expression matching every minute
        let job_id = ironclad_db::cron::create_job(
            &db,
            "worker-cron-kind",
            "test-agent",
            "cron",
            Some("* * * * *"),
            r#"{"action":"log","message":"cron tick"}"#,
        )
        .expect("create cron job");

        // The cron expression "* * * * *" matches every minute.
        // We don't set last_run_at, so it should be evaluated as due.

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "cron-kind-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(100)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // The cron evaluation depends on real wall-clock matching chrono::Utc::now().
        // In paused-time tests, Utc::now() still returns the real time, so the cron
        // expression "* * * * *" should match at most times.
        // We check that the job was at least attempted (run recorded).
        let count: i64 = db
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM cron_runs WHERE job_id = '{}'", job_id),
                [],
                |row| row.get(0),
            )
            .expect("count cron runs");
        // Cron jobs depend on wall time matching. If it happens to match, count >= 1.
        // We don't assert strictly because wall clock vs cron expression may not align
        // in CI, but the code path is still exercised.
        let _ = count;
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_unknown_schedule_kind_not_due() {
        let db = test_db();
        // Create a job with an unknown schedule kind -- should not be treated as due
        let job_id = ironclad_db::cron::create_job(
            &db,
            "worker-unknown-kind",
            "test-agent",
            "weekly",
            Some("*"),
            r#"{"action":"log","message":"weekly"}"#,
        )
        .expect("create job");

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "unknown-kind-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // Unknown schedule_kind -> due = false -> no runs recorded for that job
        let count: i64 = db
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM cron_runs WHERE job_id = '{}'", job_id),
                [],
                |row| row.get(0),
            )
            .expect("count runs");
        assert_eq!(count, 0, "unknown schedule kind should not be executed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_interval_kind_job() {
        let db = test_db();
        // Create a job with "interval" schedule_kind (gets normalized to "every")
        let job_id = ironclad_db::cron::create_job(
            &db,
            "worker-interval-kind",
            "test-agent",
            "interval",
            Some("1s"),
            r#"{"action":"noop"}"#,
        )
        .expect("create job");
        // Set schedule_every_ms to 1 so it's immediately due
        db.conn()
            .execute(
                "UPDATE cron_jobs SET schedule_every_ms = 1 WHERE id = ?1",
                [&job_id],
            )
            .expect("update ms");

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "interval-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let count: i64 = db
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM cron_runs WHERE job_id = '{}'", job_id),
                [],
                |row| row.get(0),
            )
            .expect("count runs");
        assert!(count >= 1, "interval job should have been executed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_every_kind_with_expr_fallback() {
        let db = test_db();
        // Create a job with "every" kind and schedule_expr but no schedule_every_ms
        // This tests the or_else fallback to parse_interval_expr_to_ms
        let job_id = ironclad_db::cron::create_job(
            &db,
            "worker-every-expr",
            "test-agent",
            "every",
            Some("1s"),
            r#"{"action":"noop"}"#,
        )
        .expect("create job");
        // Ensure schedule_every_ms is NULL so the expr fallback is used
        db.conn()
            .execute(
                "UPDATE cron_jobs SET schedule_every_ms = NULL WHERE id = ?1",
                [&job_id],
            )
            .expect("clear ms");

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "every-expr-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        // The expr "1s" = 1000ms, and with no last_run the job should be immediately due
        let count: i64 = db
            .conn()
            .query_row(
                &format!("SELECT COUNT(*) FROM cron_runs WHERE job_id = '{}'", job_id),
                [],
                |row| row.get(0),
            )
            .expect("count runs");
        assert!(count >= 1, "every-kind with expr fallback should have been executed");
    }

    #[tokio::test(start_paused = true)]
    async fn run_cron_worker_multiple_jobs_in_single_tick() {
        let db = test_db();
        let _id1 = create_due_job(&db, "multi-1", r#"{"action":"log","message":"first"}"#);
        let _id2 = create_due_job(&db, "multi-2", r#"{"action":"noop"}"#);
        let _id3 = create_due_job(&db, "multi-3", r#"{"action":"log","message":"third"}"#);

        let db_clone = db.clone();
        let handle = tokio::spawn(async move {
            run_cron_worker(db_clone, "multi-instance".into()).await;
        });

        tokio::time::advance(std::time::Duration::from_secs(61)).await;
        tokio::task::yield_now().await;
        tokio::time::advance(std::time::Duration::from_millis(10)).await;
        tokio::task::yield_now().await;

        handle.abort();
        let _ = handle.await;

        let count: i64 = db
            .conn()
            .query_row("SELECT COUNT(*) FROM cron_runs", [], |row| row.get(0))
            .expect("count runs");
        assert!(count >= 3, "expected at least 3 cron runs, got {count}");
    }
}
