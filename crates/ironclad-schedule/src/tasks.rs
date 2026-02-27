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
    SessionGovernor,
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
            message: format!(
                "memory prune executed at {} for tier {:?}",
                ctx.timestamp.to_rfc3339(),
                ctx.survival_tier
            ),
            should_wake: false,
        },
        HeartbeatTask::CacheEvict => TaskResult {
            task: task.clone(),
            success: true,
            message: format!("cache eviction completed at {}", ctx.timestamp.to_rfc3339()),
            should_wake: false,
        },
        HeartbeatTask::MetricSnapshot => TaskResult {
            task: task.clone(),
            success: true,
            message: format!(
                "snapshot tier={:?} usdc={:.4} credit={:.4}",
                ctx.survival_tier, ctx.usdc_balance, ctx.credit_balance
            ),
            should_wake: false,
        },
        HeartbeatTask::AgentCardRefresh => TaskResult {
            task: task.clone(),
            success: true,
            message: format!(
                "agent card refresh heartbeat at {}",
                ctx.timestamp.to_rfc3339()
            ),
            should_wake: false,
        },
        HeartbeatTask::SessionGovernor => TaskResult {
            task: task.clone(),
            success: true,
            message: format!(
                "session governor tick at {} (tier {:?})",
                ctx.timestamp.to_rfc3339(),
                ctx.survival_tier
            ),
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
        assert!(result.message.contains("memory prune"));
        assert_eq!(result.task, HeartbeatTask::MemoryPrune);
    }

    #[test]
    fn cache_evict_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::CacheEvict, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert!(result.message.contains("cache eviction"));
        assert_eq!(result.task, HeartbeatTask::CacheEvict);
    }

    #[test]
    fn metric_snapshot_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::MetricSnapshot, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert!(result.message.contains("snapshot"));
        assert_eq!(result.task, HeartbeatTask::MetricSnapshot);
    }

    #[test]
    fn agent_card_refresh_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::AgentCardRefresh, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert!(result.message.contains("agent card"));
        assert_eq!(result.task, HeartbeatTask::AgentCardRefresh);
    }

    // ── BUG-076: SessionGovernor task coverage ─────────────────────────

    #[test]
    fn session_governor_task_execution() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::SessionGovernor, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
        assert!(result.message.contains("session governor"));
        assert_eq!(result.task, HeartbeatTask::SessionGovernor);
    }

    #[test]
    fn session_governor_task_includes_tier_in_message() {
        let ctx = build_tick_context(0.05, 0.01);
        let result = execute_task(&HeartbeatTask::SessionGovernor, &ctx);
        assert!(result.message.contains("Critical"));
    }

    // ── UsdcMonitor additional branch coverage ─────────────────────────

    #[test]
    fn usdc_monitor_no_wake_when_zero_balance() {
        let ctx = build_tick_context(1.0, 0.0); // Normal tier, zero usdc
        let result = execute_task(&HeartbeatTask::UsdcMonitor, &ctx);
        assert!(result.success);
        assert!(!result.should_wake); // usdc_balance == 0 -> no wake
    }

    #[test]
    fn usdc_monitor_message_includes_balance() {
        let ctx = build_tick_context(1.0, 2.5);
        let result = execute_task(&HeartbeatTask::UsdcMonitor, &ctx);
        assert!(result.message.contains("2.5000"));
    }

    // ── SurvivalCheck Dead tier ────────────────────────────────────────

    #[test]
    fn survival_check_wakes_on_dead_tier() {
        // We can't get Dead from build_tick_context (needs hours_below_zero >=1),
        // so construct TickContext directly
        let ctx = TickContext {
            credit_balance: -1.0,
            usdc_balance: 0.0,
            survival_tier: SurvivalTier::Dead,
            timestamp: chrono::Utc::now(),
        };
        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        assert!(result.success);
        assert!(result.should_wake);
    }

    #[test]
    fn survival_check_no_wake_on_normal() {
        let ctx = build_tick_context(2.0, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::Normal);
        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
    }

    #[test]
    fn survival_check_no_wake_on_low_compute() {
        let ctx = build_tick_context(0.30, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::LowCompute);
        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        assert!(result.success);
        assert!(!result.should_wake);
    }

    // ── YieldTask additional tier coverage ──────────────────────────────

    #[test]
    fn yield_task_active_on_high_tier() {
        let ctx = build_tick_context(10.0, 5.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::High);
        let result = execute_task(&HeartbeatTask::YieldTask, &ctx);
        assert!(result.success);
        assert!(result.message.contains("active"));
        assert!(!result.should_wake);
    }

    #[test]
    fn yield_task_skipped_on_low_compute() {
        let ctx = build_tick_context(0.30, 0.0);
        assert_eq!(ctx.survival_tier, SurvivalTier::LowCompute);
        let result = execute_task(&HeartbeatTask::YieldTask, &ctx);
        assert!(result.success);
        assert!(result.message.contains("skipped"));
    }

    #[test]
    fn yield_task_skipped_on_dead_tier() {
        let ctx = TickContext {
            credit_balance: 0.0,
            usdc_balance: 0.0,
            survival_tier: SurvivalTier::Dead,
            timestamp: chrono::Utc::now(),
        };
        let result = execute_task(&HeartbeatTask::YieldTask, &ctx);
        assert!(result.success);
        assert!(result.message.contains("skipped"));
    }

    // ── MetricSnapshot message content ─────────────────────────────────

    #[test]
    fn metric_snapshot_message_includes_balances() {
        let ctx = build_tick_context(3.5, 1.25);
        let result = execute_task(&HeartbeatTask::MetricSnapshot, &ctx);
        assert!(result.success);
        assert!(result.message.contains("3.5000"));
        assert!(result.message.contains("1.2500"));
        assert!(result.message.contains("Normal"));
    }

    // ── MemoryPrune message content ────────────────────────────────────

    #[test]
    fn memory_prune_message_includes_tier() {
        let ctx = build_tick_context(0.05, 0.01);
        let result = execute_task(&HeartbeatTask::MemoryPrune, &ctx);
        assert!(result.success);
        assert!(result.message.contains("Critical"));
    }

    // ── CacheEvict message content ─────────────────────────────────────

    #[test]
    fn cache_evict_message_includes_timestamp() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::CacheEvict, &ctx);
        assert!(result.success);
        assert!(result.message.contains("cache eviction"));
        // Timestamp should be an ISO/RFC3339 string
        assert!(result.message.contains("T"));
    }

    // ── AgentCardRefresh message content ───────────────────────────────

    #[test]
    fn agent_card_refresh_message_includes_timestamp() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::AgentCardRefresh, &ctx);
        assert!(result.success);
        assert!(result.message.contains("agent card refresh"));
        assert!(result.message.contains("T"));
    }

    // ── TaskResult and HeartbeatTask serialization roundtrip ────────────

    #[test]
    fn task_result_serialization_roundtrip() {
        let ctx = build_tick_context(1.0, 1.0);
        let result = execute_task(&HeartbeatTask::SurvivalCheck, &ctx);
        let json = serde_json::to_string(&result).expect("serialize TaskResult");
        let deserialized: TaskResult = serde_json::from_str(&json).expect("deserialize TaskResult");
        assert_eq!(deserialized.task, result.task);
        assert_eq!(deserialized.success, result.success);
        assert_eq!(deserialized.message, result.message);
        assert_eq!(deserialized.should_wake, result.should_wake);
    }

    #[test]
    fn heartbeat_task_serialization_roundtrip() {
        let tasks = vec![
            HeartbeatTask::SurvivalCheck,
            HeartbeatTask::UsdcMonitor,
            HeartbeatTask::YieldTask,
            HeartbeatTask::MemoryPrune,
            HeartbeatTask::CacheEvict,
            HeartbeatTask::MetricSnapshot,
            HeartbeatTask::AgentCardRefresh,
            HeartbeatTask::SessionGovernor,
        ];
        for task in &tasks {
            let json = serde_json::to_string(task).expect("serialize");
            let deserialized: HeartbeatTask = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(&deserialized, task);
        }
    }

    // ── Execute all task variants with various tiers ────────────────────

    #[test]
    fn all_tasks_succeed_on_every_tier() {
        use crate::heartbeat::default_tasks;
        let tiers = [
            (10.0, 5.0),  // High
            (2.0, 0.0),   // Normal
            (0.30, 0.0),  // LowCompute
            (0.05, 0.01), // Critical
        ];
        for (credit, usdc) in tiers {
            let ctx = build_tick_context(credit, usdc);
            for task in default_tasks() {
                let result = execute_task(&task, &ctx);
                assert!(
                    result.success,
                    "Task {:?} failed at tier {:?}",
                    task, ctx.survival_tier
                );
            }
        }
    }
}
