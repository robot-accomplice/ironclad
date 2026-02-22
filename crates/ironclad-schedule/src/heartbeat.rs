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
    pub running: bool,
}

impl HeartbeatDaemon {
    pub fn new(interval_ms: u64) -> Self {
        Self {
            interval_ms,
            running: false,
        }
    }

    /// Returns a new interval if the current tier warrants slowing down the tick loop.
    pub fn should_adjust_interval(&self, tier: &SurvivalTier) -> Option<u64> {
        const MAX_HEARTBEAT_INTERVAL_MS: u64 = 3_600_000; // 1 hour max
        const MIN_HEARTBEAT_INTERVAL_MS: u64 = 10_000; // 10 second min

        let new = match tier {
            SurvivalTier::LowCompute => self.interval_ms * 2,
            SurvivalTier::Critical => self.interval_ms * 2,
            SurvivalTier::Dead => self.interval_ms * 10,
            _ => return None,
        };
        let max = match tier {
            SurvivalTier::Dead => MAX_HEARTBEAT_INTERVAL_MS,
            _ => 300_000, // 5 minutes max for non-dead tiers
        };
        Some(new.clamp(MIN_HEARTBEAT_INTERVAL_MS, max))
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
    ]
}

/// Run the heartbeat loop. Fetches balances from the wallet, builds a tick context,
/// runs tasks, and adjusts interval based on survival tier.
pub async fn run(
    mut daemon: HeartbeatDaemon,
    wallet: std::sync::Arc<ironclad_wallet::WalletService>,
    db: ironclad_db::Database,
) {
    use crate::tasks::{HeartbeatTask, execute_task};
    use std::time::Duration;

    daemon.running = true;
    let tasks = default_tasks();
    let mut interval = tokio::time::interval(Duration::from_millis(daemon.interval_ms));
    let mut last_atoken_balance: Option<f64> = None;

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
            // Record task run in cron_runs for observability
            ironclad_db::cron::record_run(
                &db,
                &format!("heartbeat_{:?}", task).to_lowercase(),
                if result.success { "success" } else { "error" },
                None,
                if result.success {
                    None
                } else {
                    Some(&result.message)
                },
            )
            .ok();
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
            interval = tokio::time::interval(Duration::from_millis(new_interval));
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

    // Phase 4I: default_tasks() returns expected task set
    #[test]
    fn default_tasks_returns_expected_set() {
        use crate::tasks::HeartbeatTask;
        let tasks = default_tasks();
        assert_eq!(tasks.len(), 6);
        assert_eq!(tasks[0], HeartbeatTask::SurvivalCheck);
        assert_eq!(tasks[1], HeartbeatTask::UsdcMonitor);
        assert_eq!(tasks[2], HeartbeatTask::YieldTask);
        assert_eq!(tasks[3], HeartbeatTask::MemoryPrune);
        assert_eq!(tasks[4], HeartbeatTask::CacheEvict);
        assert_eq!(tasks[5], HeartbeatTask::MetricSnapshot);
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
}
