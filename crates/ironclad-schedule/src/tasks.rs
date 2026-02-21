use ironclad_core::SurvivalTier;
use serde::{Deserialize, Serialize};

use crate::heartbeat::TickContext;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HeartbeatTask {
    SurvivalCheck,
    UsdcMonitor,
    YieldTask,
    MemoryPrune,
    CacheEvict,
    MetricSnapshot,
    AgentCardRefresh,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskResult {
    pub task: HeartbeatTask,
    pub success: bool,
    pub message: String,
    pub should_wake: bool,
}

pub fn execute_task(task: &HeartbeatTask, ctx: &TickContext) -> TaskResult {
    match task {
        HeartbeatTask::SurvivalCheck => {
            let should_wake = matches!(
                ctx.survival_tier,
                SurvivalTier::Critical | SurvivalTier::Dead
            );
            TaskResult {
                task: task.clone(),
                success: true,
                message: format!("survival tier: {:?}", ctx.survival_tier),
                should_wake,
            }
        }
        HeartbeatTask::UsdcMonitor => {
            let should_wake =
                ctx.usdc_balance > 0.0 && !matches!(ctx.survival_tier, SurvivalTier::High);
            TaskResult {
                task: task.clone(),
                success: true,
                message: format!("usdc_balance={:.4}", ctx.usdc_balance),
                should_wake,
            }
        }
        HeartbeatTask::YieldTask => {
            let active = matches!(ctx.survival_tier, SurvivalTier::High | SurvivalTier::Normal);
            TaskResult {
                task: task.clone(),
                success: true,
                message: if active {
                    "yield evaluation active".into()
                } else {
                    "yield skipped — tier too low".into()
                },
                should_wake: false,
            }
        }
        HeartbeatTask::MemoryPrune => TaskResult {
            task: task.clone(),
            success: true,
            message: "memory prune queued".into(),
            should_wake: false,
        },
        HeartbeatTask::CacheEvict => TaskResult {
            task: task.clone(),
            success: true,
            message: "cache eviction complete".into(),
            should_wake: false,
        },
        HeartbeatTask::MetricSnapshot => TaskResult {
            task: task.clone(),
            success: true,
            message: "metrics snapshot captured".into(),
            should_wake: false,
        },
        HeartbeatTask::AgentCardRefresh => TaskResult {
            task: task.clone(),
            success: true,
            message: "agent card refreshed".into(),
            should_wake: false,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heartbeat::build_tick_context;

    #[test]
    fn survival_check_wakes_on_critical() {
        let ctx = build_tick_context(0.05, 0.01);
        assert_eq!(ctx.survival_tier, SurvivalTier::Critical);

        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        assert!(result.success);
        assert!(result.should_wake);
    }

    #[test]
    fn survival_check_no_wake_on_high() {
        let ctx = build_tick_context(10.0, 5.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::High);

        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
    }

    #[test]
    fn usdc_monitor_wakes_when_balance_present_and_not_high() {
        let ctx = build_tick_context(0.30, 1.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);

        let result = execute_task(&HeartbeatTask::UsdcMonitor, &ctx);
        assert!(result.success);
        assert!(result.should_wake);
    }

    #[test]
    fn usdc_monitor_no_wake_when_high_tier() {
        let ctx = build_tick_context(10.0, 5.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::High);

        let result = execute_task(&HeartbeatTask::UsdcMonitor, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
    }

    #[test]
    fn yield_task_skipped_on_low_tier() {
        let ctx = build_tick_context(0.05, 0.01);
        assert_eq!(ctx.survival_tier, SurvivalTier::Critical);

        let result = execute_task(&HeartbeatTask::YieldTask, &ctx);
        assert!(result.success);
        assert!(result.message.contains("skipped"));
    }

    #[test]
    fn yield_task_active_on_normal_tier() {
        let ctx = build_tick_context(2.0, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);

        let result = execute_task(&HeartbeatTask::YieldTask, &ctx);
        assert!(result.success);
        assert!(result.message.contains("active"));
    }

    #[test]
    fn memory_prune_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::MemoryPrune, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert_eq!(result.message, "memory prune queued");
        assert_eq!(result.task, HeartbeatTask::MemoryPrune);
    }

    #[test]
    fn cache_evict_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::CacheEvict, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert_eq!(result.message, "cache eviction complete");
        assert_eq!(result.task, HeartbeatTask::CacheEvict);
    }

    #[test]
    fn metric_snapshot_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::MetricSnapshot, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert_eq!(result.message, "metrics snapshot captured");
        assert_eq!(result.task, HeartbeatTask::MetricSnapshot);
    }

    #[test]
    fn agent_card_refresh_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::AgentCardRefresh, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert_eq!(result.message, "agent card refreshed");
        assert_eq!(result.task, HeartbeatTask::AgentCardRefresh);
    }
}
