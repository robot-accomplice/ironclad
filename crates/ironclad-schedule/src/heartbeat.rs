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
        match tier {
            SurvivalTier::LowCompute => Some(self.interval_ms * 2),
            SurvivalTier::Critical => Some(self.interval_ms * 4),
            SurvivalTier::Dead => Some(self.interval_ms * 10),
            _ => None,
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
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::LowCompute),
            Some(2000)
        );
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::Critical),
            Some(4000)
        );
        assert_eq!(
            daemon.should_adjust_interval(&SurvivalTier::Dead),
            Some(10000)
        );
    }
}
