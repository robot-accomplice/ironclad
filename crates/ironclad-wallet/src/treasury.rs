use ironclad_core::config::TreasuryConfig;
use ironclad_core::{IroncladError, Result};
use tracing::warn;

use crate::money::Money;

#[derive(Debug, Clone)]
pub struct TreasuryPolicy {
    per_payment_cap: f64,
    hourly_transfer_limit: f64,
    daily_transfer_limit: f64,
    minimum_reserve: f64,
    daily_inference_budget: f64,
}

impl TreasuryPolicy {
    pub fn new(config: &TreasuryConfig) -> Self {
        Self {
            per_payment_cap: config.per_payment_cap,
            hourly_transfer_limit: config.hourly_transfer_limit,
            daily_transfer_limit: config.daily_transfer_limit,
            minimum_reserve: config.minimum_reserve,
            daily_inference_budget: config.daily_inference_budget,
        }
    }

    pub fn per_payment_cap(&self) -> f64 {
        self.per_payment_cap
    }

    pub fn hourly_transfer_limit(&self) -> f64 {
        self.hourly_transfer_limit
    }

    pub fn daily_transfer_limit(&self) -> f64 {
        self.daily_transfer_limit
    }

    pub fn minimum_reserve(&self) -> f64 {
        self.minimum_reserve
    }

    pub fn daily_inference_budget(&self) -> f64 {
        self.daily_inference_budget
    }

    /// Ensures a single payment amount is within the per-payment cap.
    ///
    /// # Examples
    ///
    /// ```
    /// use ironclad_wallet::treasury::TreasuryPolicy;
    /// use ironclad_core::config::TreasuryConfig;
    ///
    /// let config = TreasuryConfig::default();
    /// let policy = TreasuryPolicy::new(&config);
    /// assert!(policy.check_per_payment(5.0).is_ok());
    /// ```
    pub fn check_per_payment(&self, amount: f64) -> Result<()> {
        let amt = Money::from_dollars(amount)?;
        let cap = Money::from_dollars(self.per_payment_cap)?;
        if amt <= Money::zero() {
            return Err(IroncladError::Policy {
                rule: "non_positive_amount".into(),
                reason: format!("payment amount must be positive, got {amount}"),
            });
        }
        if amt > cap {
            warn!(
                amount,
                cap = self.per_payment_cap,
                "per-payment cap exceeded"
            );
            return Err(IroncladError::Policy {
                rule: "per_payment_cap".into(),
                reason: format!(
                    "payment {amount} exceeds per-payment cap {}",
                    self.per_payment_cap
                ),
            });
        }
        Ok(())
    }

    pub fn check_hourly_limit(&self, recent_hourly_total: f64, new_amount: f64) -> Result<()> {
        let new_amt = Money::from_dollars(new_amount)?;
        let limit = Money::from_dollars(self.hourly_transfer_limit)?;
        if new_amt <= Money::zero() {
            return Err(IroncladError::Policy {
                rule: "non_positive_amount".into(),
                reason: format!("payment amount must be positive, got {new_amount}"),
            });
        }
        let projected = Money::from_dollars(recent_hourly_total)? + new_amt;
        if projected > limit {
            warn!(
                projected = projected.dollars(),
                limit = self.hourly_transfer_limit,
                "hourly limit exceeded"
            );
            return Err(IroncladError::Policy {
                rule: "hourly_transfer_limit".into(),
                reason: format!(
                    "projected hourly total {} exceeds limit {}",
                    projected.dollars(),
                    self.hourly_transfer_limit
                ),
            });
        }
        Ok(())
    }

    pub fn check_daily_limit(&self, recent_daily_total: f64, new_amount: f64) -> Result<()> {
        let new_amt = Money::from_dollars(new_amount)?;
        let limit = Money::from_dollars(self.daily_transfer_limit)?;
        if new_amt <= Money::zero() {
            return Err(IroncladError::Policy {
                rule: "non_positive_amount".into(),
                reason: format!("payment amount must be positive, got {new_amount}"),
            });
        }
        let projected = Money::from_dollars(recent_daily_total)? + new_amt;
        if projected > limit {
            warn!(
                projected = projected.dollars(),
                limit = self.daily_transfer_limit,
                "daily limit exceeded"
            );
            return Err(IroncladError::Policy {
                rule: "daily_transfer_limit".into(),
                reason: format!(
                    "projected daily total {} exceeds limit {}",
                    projected.dollars(),
                    self.daily_transfer_limit
                ),
            });
        }
        Ok(())
    }

    pub fn check_minimum_reserve(&self, current_balance: f64, amount: f64) -> Result<()> {
        let remaining = Money::from_dollars(current_balance)? - Money::from_dollars(amount)?;
        let reserve = Money::from_dollars(self.minimum_reserve)?;
        if remaining < reserve {
            warn!(
                remaining = remaining.dollars(),
                reserve = self.minimum_reserve,
                "minimum reserve violated"
            );
            return Err(IroncladError::Policy {
                rule: "minimum_reserve".into(),
                reason: format!(
                    "remaining balance {} would fall below minimum reserve {}",
                    remaining.dollars(),
                    self.minimum_reserve
                ),
            });
        }
        Ok(())
    }

    pub fn check_inference_budget(&self, daily_inference_total: f64, new_cost: f64) -> Result<()> {
        let projected =
            Money::from_dollars(daily_inference_total)? + Money::from_dollars(new_cost)?;
        let budget = Money::from_dollars(self.daily_inference_budget)?;
        if projected > budget {
            warn!(
                projected = projected.dollars(),
                budget = self.daily_inference_budget,
                "inference budget exceeded"
            );
            return Err(IroncladError::Policy {
                rule: "daily_inference_budget".into(),
                reason: format!(
                    "projected inference spend {} exceeds daily budget {}",
                    projected.dollars(),
                    self.daily_inference_budget
                ),
            });
        }
        Ok(())
    }

    pub fn check_all(
        &self,
        amount: f64,
        current_balance: f64,
        hourly_total: f64,
        daily_total: f64,
    ) -> Result<()> {
        self.check_per_payment(amount)?;
        self.check_hourly_limit(hourly_total, amount)?;
        self.check_daily_limit(daily_total, amount)?;
        self.check_minimum_reserve(current_balance, amount)?;
        Ok(())
    }
}

impl Default for TreasuryPolicy {
    fn default() -> Self {
        Self::new(&TreasuryConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_policy() -> TreasuryPolicy {
        TreasuryPolicy::new(&TreasuryConfig {
            per_payment_cap: 100.0,
            hourly_transfer_limit: 500.0,
            daily_transfer_limit: 2000.0,
            minimum_reserve: 5.0,
            daily_inference_budget: 50.0,
            revenue_swap: Default::default(),
        })
    }

    #[test]
    fn per_payment_within_cap() {
        let policy = default_policy();
        assert!(policy.check_per_payment(99.99).is_ok());
        assert!(policy.check_per_payment(100.0).is_ok());
    }

    #[test]
    fn per_payment_exceeds_cap() {
        let policy = default_policy();
        assert!(policy.check_per_payment(100.01).is_err());
        assert!(policy.check_per_payment(200.0).is_err());
    }

    #[test]
    fn hourly_limit_within() {
        let policy = default_policy();
        assert!(policy.check_hourly_limit(400.0, 100.0).is_ok());
        assert!(policy.check_hourly_limit(0.0, 500.0).is_ok());
    }

    #[test]
    fn hourly_limit_exceeded() {
        let policy = default_policy();
        assert!(policy.check_hourly_limit(400.0, 100.01).is_err());
        assert!(policy.check_hourly_limit(500.0, 0.01).is_err());
    }

    #[test]
    fn daily_limit_within() {
        let policy = default_policy();
        assert!(policy.check_daily_limit(1900.0, 100.0).is_ok());
    }

    #[test]
    fn daily_limit_exceeded() {
        let policy = default_policy();
        assert!(policy.check_daily_limit(1900.0, 100.01).is_err());
    }

    #[test]
    fn minimum_reserve_maintained() {
        let policy = default_policy();
        assert!(policy.check_minimum_reserve(100.0, 95.0).is_ok());
        assert!(policy.check_minimum_reserve(10.0, 5.0).is_ok());
    }

    #[test]
    fn minimum_reserve_violated() {
        let policy = default_policy();
        assert!(policy.check_minimum_reserve(10.0, 5.01).is_err());
        assert!(policy.check_minimum_reserve(4.0, 0.0).is_err());
    }

    #[test]
    fn inference_budget_within() {
        let policy = default_policy();
        assert!(policy.check_inference_budget(40.0, 10.0).is_ok());
    }

    #[test]
    fn inference_budget_exceeded() {
        let policy = default_policy();
        assert!(policy.check_inference_budget(40.0, 10.01).is_err());
    }

    #[test]
    fn check_all_passes() {
        let policy = default_policy();
        assert!(policy.check_all(50.0, 100.0, 100.0, 500.0).is_ok());
    }

    #[test]
    fn check_all_fails_per_payment() {
        let policy = default_policy();
        let result = policy.check_all(150.0, 1000.0, 0.0, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn check_all_fails_hourly() {
        let policy = default_policy();
        let result = policy.check_all(50.0, 1000.0, 460.0, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn check_all_fails_daily() {
        let policy = default_policy();
        let result = policy.check_all(50.0, 1000.0, 0.0, 1960.0);
        assert!(result.is_err());
    }

    #[test]
    fn check_all_fails_reserve() {
        let policy = default_policy();
        let result = policy.check_all(50.0, 54.0, 0.0, 0.0);
        assert!(result.is_err());
    }

    #[test]
    fn from_treasury_config() {
        let config = TreasuryConfig::default();
        let policy = TreasuryPolicy::new(&config);
        assert!((policy.per_payment_cap() - 100.0).abs() < f64::EPSILON);
        assert!((policy.minimum_reserve() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn negative_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_per_payment(-1.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
        assert!(msg.contains("-1"));
    }

    #[test]
    fn zero_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_per_payment(0.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
    }

    #[test]
    fn positive_amount_passes() {
        let policy = default_policy();
        assert!(policy.check_per_payment(0.01).is_ok());
        assert!(policy.check_per_payment(1.0).is_ok());
        assert!(policy.check_per_payment(99.0).is_ok());
    }

    #[test]
    fn negative_amount_minimum_reserve_rejected() {
        let policy = default_policy();
        assert!(policy.check_minimum_reserve(10.0, 15.0).is_err());
    }

    #[test]
    fn treasury_policy_from_default_config_no_panic() {
        let config = TreasuryConfig::default();
        let policy = TreasuryPolicy::new(&config);
        assert!(policy.check_per_payment(1.0).is_ok());
    }

    // Phase 4K: Treasury with all caps at zero rejects everything
    #[test]
    fn treasury_all_caps_zero_rejects_everything() {
        let policy = TreasuryPolicy::new(&TreasuryConfig {
            per_payment_cap: 0.0,
            hourly_transfer_limit: 0.0,
            daily_transfer_limit: 0.0,
            minimum_reserve: 0.0,
            daily_inference_budget: 0.0,
            revenue_swap: Default::default(),
        });
        assert!(policy.check_per_payment(0.01).is_err());
        assert!(policy.check_per_payment(1.0).is_err());
        assert!(policy.check_hourly_limit(0.0, 0.01).is_err());
        assert!(policy.check_daily_limit(0.0, 0.01).is_err());
        assert!(policy.check_inference_budget(0.0, 0.01).is_err());
    }

    // --- getter methods coverage ---

    #[test]
    fn getter_hourly_transfer_limit() {
        let policy = default_policy();
        assert!((policy.hourly_transfer_limit() - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn getter_daily_transfer_limit() {
        let policy = default_policy();
        assert!((policy.daily_transfer_limit() - 2000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn getter_minimum_reserve() {
        let policy = default_policy();
        assert!((policy.minimum_reserve() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn getter_daily_inference_budget() {
        let policy = default_policy();
        assert!((policy.daily_inference_budget() - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn getter_per_payment_cap() {
        let policy = default_policy();
        assert!((policy.per_payment_cap() - 100.0).abs() < f64::EPSILON);
    }

    // --- check_hourly_limit error message content ---

    #[test]
    fn check_hourly_limit_error_contains_rule_name() {
        let policy = default_policy();
        let err = policy.check_hourly_limit(400.0, 100.01).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("hourly_transfer_limit"));
    }

    #[test]
    fn check_hourly_limit_negative_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_hourly_limit(0.0, -1.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
    }

    #[test]
    fn check_hourly_limit_zero_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_hourly_limit(0.0, 0.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
    }

    #[test]
    fn check_hourly_limit_exact_boundary_passes() {
        let policy = default_policy();
        // 0 + 500.0 = 500.0, which equals the limit exactly
        assert!(policy.check_hourly_limit(0.0, 500.0).is_ok());
    }

    // --- check_daily_limit error message content ---

    #[test]
    fn check_daily_limit_error_contains_rule_name() {
        let policy = default_policy();
        let err = policy.check_daily_limit(1900.0, 100.01).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("daily_transfer_limit"));
    }

    #[test]
    fn check_daily_limit_negative_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_daily_limit(0.0, -5.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
    }

    #[test]
    fn check_daily_limit_zero_amount_rejected() {
        let policy = default_policy();
        let err = policy.check_daily_limit(0.0, 0.0).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("non_positive_amount") || msg.contains("positive"));
    }

    #[test]
    fn check_daily_limit_exact_boundary_passes() {
        let policy = default_policy();
        assert!(policy.check_daily_limit(0.0, 2000.0).is_ok());
    }

    // --- check_minimum_reserve error message content ---

    #[test]
    fn check_minimum_reserve_error_contains_rule_name() {
        let policy = default_policy();
        let err = policy.check_minimum_reserve(10.0, 5.01).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("minimum_reserve"));
    }

    #[test]
    fn check_minimum_reserve_exact_boundary_passes() {
        let policy = default_policy();
        // balance 10, amount 5 → remaining 5 == reserve 5
        assert!(policy.check_minimum_reserve(10.0, 5.0).is_ok());
    }

    #[test]
    fn check_minimum_reserve_zero_amount() {
        let policy = default_policy();
        // balance 10, amount 0 → remaining 10 > reserve 5
        assert!(policy.check_minimum_reserve(10.0, 0.0).is_ok());
    }

    // --- check_inference_budget error message content ---

    #[test]
    fn check_inference_budget_error_contains_rule_name() {
        let policy = default_policy();
        let err = policy.check_inference_budget(40.0, 10.01).unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("daily_inference_budget"));
    }

    #[test]
    fn check_inference_budget_exact_boundary_passes() {
        let policy = default_policy();
        // 0 + 50.0 = 50.0, which equals the budget exactly
        assert!(policy.check_inference_budget(0.0, 50.0).is_ok());
    }

    #[test]
    fn check_inference_budget_zero_cost() {
        let policy = default_policy();
        assert!(policy.check_inference_budget(40.0, 0.0).is_ok());
    }

    // --- check_all success paths ---

    #[test]
    fn check_all_exact_per_payment_cap() {
        let policy = default_policy();
        assert!(policy.check_all(100.0, 1000.0, 0.0, 0.0).is_ok());
    }

    #[test]
    fn check_all_minimal_balance() {
        let policy = default_policy();
        // amount=5.0, balance=10.0 → remaining 5.0 == reserve
        assert!(policy.check_all(5.0, 10.0, 0.0, 0.0).is_ok());
    }

    // --- Default impl ---

    #[test]
    fn treasury_policy_default_impl() {
        let policy = TreasuryPolicy::default();
        // Default config should have reasonable values
        assert!(policy.per_payment_cap() > 0.0);
        assert!(policy.hourly_transfer_limit() > 0.0);
        assert!(policy.daily_transfer_limit() > 0.0);
        assert!(policy.minimum_reserve() >= 0.0);
        assert!(policy.daily_inference_budget() > 0.0);
    }

    // --- Treasury clone ---

    #[test]
    fn treasury_policy_clone_preserves_fields() {
        let policy = default_policy();
        let cloned = policy.clone();
        assert!((policy.per_payment_cap() - cloned.per_payment_cap()).abs() < f64::EPSILON);
        assert!(
            (policy.hourly_transfer_limit() - cloned.hourly_transfer_limit()).abs() < f64::EPSILON
        );
        assert!(
            (policy.daily_transfer_limit() - cloned.daily_transfer_limit()).abs() < f64::EPSILON
        );
        assert!((policy.minimum_reserve() - cloned.minimum_reserve()).abs() < f64::EPSILON);
        assert!(
            (policy.daily_inference_budget() - cloned.daily_inference_budget()).abs()
                < f64::EPSILON
        );
    }

    // --- Debug ---

    #[test]
    fn treasury_policy_debug_format() {
        let policy = default_policy();
        let debug = format!("{:?}", policy);
        assert!(debug.contains("TreasuryPolicy"));
        assert!(debug.contains("per_payment_cap"));
        assert!(debug.contains("hourly_transfer_limit"));
        assert!(debug.contains("daily_transfer_limit"));
        assert!(debug.contains("minimum_reserve"));
        assert!(debug.contains("daily_inference_budget"));
    }

    // --- check_all fails at different stages ---

    #[test]
    fn check_all_fails_at_per_payment_stage_error_message() {
        let policy = default_policy();
        let err = policy.check_all(150.0, 1000.0, 0.0, 0.0).unwrap_err();
        assert!(err.to_string().contains("per_payment_cap"));
    }

    #[test]
    fn check_all_fails_at_hourly_stage_error_message() {
        let policy = default_policy();
        let err = policy.check_all(50.0, 1000.0, 460.0, 0.0).unwrap_err();
        assert!(err.to_string().contains("hourly_transfer_limit"));
    }

    #[test]
    fn check_all_fails_at_daily_stage_error_message() {
        let policy = default_policy();
        let err = policy.check_all(50.0, 1000.0, 0.0, 1960.0).unwrap_err();
        assert!(err.to_string().contains("daily_transfer_limit"));
    }

    #[test]
    fn check_all_fails_at_reserve_stage_error_message() {
        let policy = default_policy();
        let err = policy.check_all(50.0, 54.0, 0.0, 0.0).unwrap_err();
        assert!(err.to_string().contains("minimum_reserve"));
    }

    // --- Large values ---

    #[test]
    fn check_per_payment_large_cap() {
        let policy = TreasuryPolicy::new(&TreasuryConfig {
            per_payment_cap: 1_000_000.0,
            hourly_transfer_limit: 10_000_000.0,
            daily_transfer_limit: 100_000_000.0,
            minimum_reserve: 0.0,
            daily_inference_budget: 1_000_000.0,
            revenue_swap: Default::default(),
        });
        assert!(policy.check_per_payment(999_999.99).is_ok());
        assert!(policy.check_per_payment(1_000_000.0).is_ok());
        assert!(policy.check_per_payment(1_000_000.01).is_err());
    }
}
