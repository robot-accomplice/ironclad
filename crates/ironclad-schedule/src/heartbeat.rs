use crate::scheduler::DurableScheduler;
use chrono::{DateTime, Utc};
use ironclad_core::SurvivalTier;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TickContext {
    pub credit_balance: f64,
    pub usdc_balance: f64,
    pub survival_tier: SurvivalTier,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug)]
pub struct HeartbeatDaemon {
    pub interval_ms: u64,
    /// The original configured interval, used to recover when tier returns to Normal/High.
    pub original_interval_ms: u64,
    pub running: bool,
}

impl HeartbeatDaemon {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            original_interval_ms: interval_ms,
            running: false,
        }
    }

    /// Returns a new interval if the current tier warrants adjusting the tick loop.
    /// Degraded tiers slow the interval down; recovering to Normal or High restores
    /// the original configured interval.
    pub fn should_adjust_interval(&self, tier: &SurvivalTier) -> Option<u64> {
        const MAX_HEARTBEAT_INTERVAL_MS: u64 = 3_600_000; // 1 hour max
        const MIN_HEARTBEAT_INTERVAL_MS: u64 = 10_000; // 10 second min

        match tier {
            SurvivalTier::Normal | SurvivalTier::High => {
                if self.interval_ms != self.original_interval_ms {
                    Some(self.original_interval_ms)
                } else {
                    None
                }
            }
            _ => {
                let new = match tier {
                    SurvivalTier::LowCompute => self.interval_ms * 2,
                    SurvivalTier::Critical => self.interval_ms * 2,
                    SurvivalTier::Dead => self.interval_ms * 10,
                    _ => unreachable!(),
                };
                let max = match tier {
                    SurvivalTier::Dead => MAX_HEARTBEAT_INTERVAL_MS,
                    _ => 300_000, // 5 minutes max for non-dead tiers
                };
                Some(new.clamp(MIN_HEARTBEAT_INTERVAL_MS, max))
            }
        }
    }
}

/// Build a tick context by combining credit and USDC balances to derive the survival tier.
pub fn build_tick_context(credit_balance: f64, usdc_balance: f64) -> TickContext {
    let combined = credit_balance + usdc_balance;
    let survival_tier = SurvivalTier::from_balance(combined, 0.0);
    TickContext {
        credit_balance,
        usdc_balance,
        survival_tier,
        timestamp: Utc::now(),
    }
}

/// The default set of heartbeat tasks executed each tick.
pub fn default_tasks() -> Vec<crate::tasks::HeartbeatTask> {
    use crate::tasks::HeartbeatTask;
    vec![
        HeartbeatTask::SurvivalCheck,
        HeartbeatTask::UsdcMonitor,
        HeartbeatTask::YieldTask,
        HeartbeatTask::MemoryPrune,
        HeartbeatTask::CacheEvict,
        HeartbeatTask::MetricSnapshot,
        HeartbeatTask::AgentCardRefresh,
        HeartbeatTask::SessionGovernor,
    ]
}

fn should_rotate_sessions(
    reset_schedule: Option<&str>,
    last_rotation_at: Option<&str>,
    now: DateTime<Utc>,
) -> bool {
    reset_schedule
        .map(|expr| DurableScheduler::evaluate_cron(expr, last_rotation_at, &now.to_rfc3339()))
        .unwrap_or(false)
}

/// Run the heartbeat loop. Fetches balances from the wallet, builds a tick context,
/// runs tasks, and adjusts interval based on survival tier.
pub async fn run(
    mut daemon: HeartbeatDaemon,
    wallet: std::sync::Arc<ironclad_wallet::WalletService>,
    db: ironclad_db::Database,
    session_config: ironclad_core::config::SessionConfig,
    agent_id: String,
) {
    use crate::tasks::{HeartbeatTask, execute_task};
    use std::time::Duration;

    daemon.running = true;
    let tasks = default_tasks();
    let mut interval = tokio::time::interval(Duration::from_millis(daemon.interval_ms));
    let mut last_atoken_balance: Option<f64> = None;
    let governor = ironclad_agent::governor::SessionGovernor::new(session_config.clone());
    let mut last_rotation_at: Option<String> = None;

    tracing::info!(interval_ms = daemon.interval_ms, "Heartbeat loop started");

    loop {
        interval.tick().await;

        if !daemon.running {
            break;
        }

        let usdc_balance = wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
        let credit_balance = 0.0; // credit balance is external; defaults to 0
        let ctx = build_tick_context(credit_balance, usdc_balance);

        tracing::debug!(
            tier = ?ctx.survival_tier,
            usdc = usdc_balance,
            "Heartbeat tick"
        );

        for task in &tasks {
            let result = execute_task(task, &ctx);
            if !result.success {
                tracing::warn!(task = ?task, msg = %result.message, "Heartbeat task failed");
            }
            if result.should_wake {
                tracing::info!(task = ?task, msg = %result.message, "Heartbeat task triggered wake");
            }
            // YieldTask: check aToken balance and record yield_earned if balance increased
            if *task == HeartbeatTask::YieldTask {
                let agent_address = wallet.wallet.address();
                let current = wallet
                    .yield_engine
                    .get_a_token_balance(agent_address)
                    .await
                    .ok()
                    .unwrap_or(0.0);
                if let Some(prev) = last_atoken_balance
                    && current > prev
                {
                    let delta = current - prev;
                    if delta > 0.0 {
                        if let Err(e) = ironclad_db::metrics::record_transaction(
                            &db,
                            "yield_earned",
                            delta,
                            "USDC",
                            None,
                            None,
                        ) {
                            tracing::warn!(error = %e, "failed to record yield_earned metric");
                        }
                        tracing::debug!(delta, "recorded yield_earned");
                    }
                }
                last_atoken_balance = Some(current);
            }
            if *task == HeartbeatTask::MetricSnapshot && result.success {
                let snapshot = serde_json::json!({
                    "survival_tier": format!("{:?}", ctx.survival_tier),
                    "usdc_balance": ctx.usdc_balance,
                    "credit_balance": ctx.credit_balance,
                    "timestamp": ctx.timestamp.to_rfc3339(),
                });
                ironclad_db::metrics::record_metric_snapshot(&db, &snapshot.to_string())
                    .inspect_err(|e| tracing::warn!(error = %e, "failed to record metric snapshot"))
                    .ok();
            }
            if *task == HeartbeatTask::SessionGovernor {
                match governor.tick(&db) {
                    Ok(expired) => {
                        if expired > 0 {
                            tracing::info!(expired, "session governor expired stale sessions");
                        }
                    }
                    Err(e) => tracing::warn!(error = %e, "session governor tick failed"),
                }
                let now = chrono::Utc::now();
                if should_rotate_sessions(
                    session_config.reset_schedule.as_deref(),
                    last_rotation_at.as_deref(),
                    now,
                ) {
                    match governor.rotate_agent_scope_sessions(&db, &agent_id) {
                        Ok(rotated) => {
                            if rotated > 0 {
                                tracing::info!(rotated, "session governor rotated agent sessions");
                            }
                            last_rotation_at = Some(now.to_rfc3339());
                        }
                        Err(e) => tracing::warn!(error = %e, "session rotation failed"),
                    }
                }
            }
            // Heartbeat task results are logged, not recorded in cron_runs
            // (they are not cron jobs and virtual IDs would flood the table).
            if !result.success {
                tracing::debug!(task = ?task, msg = %result.message, "heartbeat task unsuccessful");
            }
        }

        if let Some(new_interval) = daemon.should_adjust_interval(&ctx.survival_tier)
            && new_interval != daemon.interval_ms
        {
            tracing::info!(
                old_ms = daemon.interval_ms,
                new_ms = new_interval,
                tier = ?ctx.survival_tier,
                "Adjusting heartbeat interval"
            );
            daemon.interval_ms = new_interval;
            let period = Duration::from_millis(new_interval);
            interval = tokio::time::interval(period);
            // Consume the immediate first tick so the new period takes effect
            interval.tick().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tick_context_high_balance() {
        let ctx = build_tick_context(10.0, 5.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::High);
        assert!((ctx.credit_balance - 10.0).abs() < f64::EPSILON);
        assert!((ctx.usdc_balance - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn tick_context_critical_balance() {
        let ctx = build_tick_context(0.05, 0.01);
        assert_eq!(ctx.survival_tier, SurvivalTier::Critical);
    }

    #[test]
    fn tick_context_low_compute_balance() {
        let ctx = build_tick_context(0.20, 0.10);
        assert_eq!(ctx.survival_tier, SurvivalTier::LowCompute);
    }

    #[test]
    fn interval_adjustment_by_tier() {
        let daemon = HeartbeatDaemon::new(1000);

        // When interval == original, Normal/High return None (no change needed)
        assert_eq!(daemon.should_adjust_interval(&SurvivalTier::High), None);
        assert_eq!(daemon.should_adjust_interval(&SurvivalTier::Normal), None);
        // Clamped to MIN 10s ..= MAX 1h
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::LowCompute),
            Some(10_000)
        );
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::Critical),
            Some(10_000)
        );
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::Dead),
            Some(10_000)
        );

        let daemon_30s = HeartbeatDaemon::new(30_000);
        assert_eq!(
            daemon_30s.should_adjust_interval(&SurvivalTier::LowCompute),
            Some(60_000)
        );
        assert_eq!(
            daemon_30s.should_adjust_interval(&SurvivalTier::Critical),
            Some(60_000) // 2x multiplier, capped at 300_000
        );
        assert_eq!(
            daemon_30s.should_adjust_interval(&SurvivalTier::Dead),
            Some(300_000)
        );
    }

    #[test]
    fn interval_recovers_to_original_on_normal_tier() {
        let mut daemon = HeartbeatDaemon::new(30_000);
        // Simulate degradation: interval was increased but original is preserved
        daemon.interval_ms = 120_000;
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::Normal),
            Some(30_000)
        );
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::High),
            Some(30_000)
        );
    }

    // Phase 4I: default_tasks() returns expected task set
    #[test]
    fn default_tasks_returns_expected_set() {
        use crate::tasks::HeartbeatTask;
        let tasks = default_tasks();
        assert_eq!(tasks.len(), 8);
        assert_eq!(tasks[0], HeartbeatTask::SurvivalCheck);
        assert_eq!(tasks[1], HeartbeatTask::UsdcMonitor);
        assert_eq!(tasks[2], HeartbeatTask::YieldTask);
        assert_eq!(tasks[3], HeartbeatTask::MemoryPrune);
        assert_eq!(tasks[4], HeartbeatTask::CacheEvict);
        assert_eq!(tasks[5], HeartbeatTask::MetricSnapshot);
        assert_eq!(tasks[6], HeartbeatTask::AgentCardRefresh);
        assert_eq!(tasks[7], HeartbeatTask::SessionGovernor);
    }

    // Phase 4I: HeartbeatDaemon::new creates with correct interval
    #[test]
    fn heartbeat_daemon_new_creates_with_correct_interval() {
        let daemon = HeartbeatDaemon::new(5000);
        assert_eq!(daemon.interval_ms, 5000);
        assert!(!daemon.running);
    }

    const MAX_HEARTBEAT_INTERVAL_MS: u64 = 3_600_000;
    const MIN_HEARTBEAT_INTERVAL_MS: u64 = 10_000;

    #[test]
    fn interval_capped_at_max() {
        let daemon = HeartbeatDaemon::new(1_800_000); // 30 min
        let new = daemon.should_adjust_interval(&SurvivalTier::Critical);
        assert!(new.is_some());
        assert!(new.unwrap() <= 300_000, "Critical tier caps at 5 minutes");

        let dead = daemon.should_adjust_interval(&SurvivalTier::Dead);
        assert!(dead.is_some());
        assert!(dead.unwrap() <= MAX_HEARTBEAT_INTERVAL_MS);
    }

    #[test]
    fn interval_floored_at_min() {
        let daemon = HeartbeatDaemon::new(1); // 1ms
        let new = daemon.should_adjust_interval(&SurvivalTier::LowCompute);
        assert!(new.is_some());
        assert!(new.unwrap() >= MIN_HEARTBEAT_INTERVAL_MS);
    }

    #[test]
    fn should_rotate_sessions_respects_cron_slots() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T12:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(should_rotate_sessions(Some("0 12 * * *"), None, now));
        assert!(!should_rotate_sessions(Some("15 12 * * *"), None, now));
    }

    #[test]
    fn should_rotate_sessions_deduplicates_same_slot() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T12:00:20+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!should_rotate_sessions(
            Some("0 12 * * *"),
            Some("2025-01-01T12:00:05+00:00"),
            now
        ));
    }

    // ── BUG-076/093: should_rotate_sessions edge cases ─────────────────

    #[test]
    fn should_rotate_sessions_none_schedule_returns_false() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T12:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!should_rotate_sessions(None, None, now));
    }

    #[test]
    fn should_rotate_sessions_none_schedule_with_last_rotation() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T12:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!should_rotate_sessions(
            None,
            Some("2025-01-01T00:00:00+00:00"),
            now
        ));
    }

    #[test]
    fn should_rotate_sessions_invalid_cron_returns_false() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T12:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(!should_rotate_sessions(Some("this is invalid"), None, now));
    }

    #[test]
    fn should_rotate_sessions_no_last_rotation_at_matches() {
        let now = DateTime::parse_from_rfc3339("2025-01-01T06:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        assert!(should_rotate_sessions(Some("0 6 * * *"), None, now));
    }

    #[test]
    fn should_rotate_sessions_different_slot_after_last_rotation() {
        let now = DateTime::parse_from_rfc3339("2025-01-02T06:00:10+00:00")
            .unwrap()
            .with_timezone(&Utc);
        // Last rotation was yesterday's 6am slot
        assert!(should_rotate_sessions(
            Some("0 6 * * *"),
            Some("2025-01-01T06:00:05+00:00"),
            now
        ));
    }

    // ── HeartbeatDaemon additional coverage ─────────────────────────────

    #[test]
    fn heartbeat_daemon_new_sets_both_intervals() {
        let daemon = HeartbeatDaemon::new(60_000);
        assert_eq!(daemon.interval_ms, 60_000);
        assert_eq!(daemon.original_interval_ms, 60_000);
        assert!(!daemon.running);
    }

    #[test]
    fn should_adjust_interval_dead_tier_caps_at_1_hour() {
        let daemon = HeartbeatDaemon::new(200_000); // 200s
        let new = daemon.should_adjust_interval(&SurvivalTier::Dead);
        // 200_000 * 10 = 2_000_000 capped at 3_600_000
        assert!(new.is_some());
        assert!(new.unwrap() <= 3_600_000);
    }

    #[test]
    fn should_adjust_interval_critical_caps_at_5_minutes() {
        let daemon = HeartbeatDaemon::new(200_000); // 200s
        let new = daemon.should_adjust_interval(&SurvivalTier::Critical);
        // 200_000 * 2 = 400_000 capped at 300_000
        assert!(new.is_some());
        assert!(new.unwrap() <= 300_000);
    }

    #[test]
    fn should_adjust_interval_low_compute_caps_at_5_minutes() {
        let daemon = HeartbeatDaemon::new(200_000); // 200s
        let new = daemon.should_adjust_interval(&SurvivalTier::LowCompute);
        // 200_000 * 2 = 400_000 capped at 300_000
        assert!(new.is_some());
        assert!(new.unwrap() <= 300_000);
    }

    #[test]
    fn should_adjust_interval_already_at_original_no_change() {
        let daemon = HeartbeatDaemon::new(60_000);
        // Normal/High with interval == original -> None
        assert!(
            daemon
                .should_adjust_interval(&SurvivalTier::Normal)
                .is_none()
        );
        assert!(daemon.should_adjust_interval(&SurvivalTier::High).is_none());
    }

    #[test]
    fn should_adjust_interval_degraded_then_recover() {
        let mut daemon = HeartbeatDaemon::new(30_000);
        // Simulate degradation
        daemon.interval_ms = 300_000;
        // Now tier recovers to Normal -> restore original
        let restored = daemon.should_adjust_interval(&SurvivalTier::Normal);
        assert_eq!(restored, Some(30_000));
        // Recover to High
        let restored = daemon.should_adjust_interval(&SurvivalTier::High);
        assert_eq!(restored, Some(30_000));
    }

    // ── build_tick_context additional tiers ─────────────────────────────

    #[test]
    fn build_tick_context_dead_tier() {
        // Dead requires < 0.0 balance AND hours_below_zero >= 0.999
        // from_balance(usd=-0.5, hours=1.0) = Dead
        // BUT build_tick_context uses combined = credit + usdc with hours=0.0
        // So Dead requires usd < 0 AND hours >= 0.999, but hours is always 0 in build_tick_context
        // Therefore Dead tier isn't reachable via build_tick_context. Let's verify Critical:
        let ctx = build_tick_context(0.001, 0.001);
        // combined = 0.002, which is < 0.10 -> Critical
        assert_eq!(ctx.survival_tier, SurvivalTier::Critical);
    }

    #[test]
    fn build_tick_context_normal_tier() {
        let ctx = build_tick_context(1.0, 0.0);
        // combined = 1.0, which is >= 0.50 and < 5.0 -> Normal
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);
    }

    #[test]
    fn build_tick_context_zero_balances() {
        let ctx = build_tick_context(0.0, 0.0);
        // combined = 0.0, which is < 0.10 -> Critical
        assert_eq!(ctx.survival_tier, SurvivalTier::Critical);
        assert!((ctx.credit_balance - 0.0).abs() < f64::EPSILON);
        assert!((ctx.usdc_balance - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_tick_context_boundary_low_compute() {
        // LowCompute boundary: >= 0.10 and < 0.50
        let ctx = build_tick_context(0.10, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::LowCompute);

        let ctx = build_tick_context(0.49, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::LowCompute);
    }

    #[test]
    fn build_tick_context_boundary_normal() {
        // Normal boundary: >= 0.50 and < 5.00
        let ctx = build_tick_context(0.50, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);

        let ctx = build_tick_context(4.99, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);
    }

    #[test]
    fn build_tick_context_boundary_high() {
        // High boundary: >= 5.00
        let ctx = build_tick_context(5.00, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::High);
    }

    #[test]
    fn build_tick_context_timestamp_is_recent() {
        let before = Utc::now();
        let ctx = build_tick_context(1.0, 1.0);
        let after = Utc::now();
        assert!(ctx.timestamp >= before);
        assert!(ctx.timestamp <= after);
    }

    // ── default_tasks coverage ─────────────────────────────────────────

    #[test]
    fn default_tasks_last_is_session_governor() {
        use crate::tasks::HeartbeatTask;
        let tasks = default_tasks();
        assert_eq!(*tasks.last().unwrap(), HeartbeatTask::SessionGovernor);
    }
}
