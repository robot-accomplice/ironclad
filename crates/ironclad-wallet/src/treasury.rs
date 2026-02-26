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
        let amt = Money::from_dollars(amount);
        let cap = Money::from_dollars(self.per_payment_cap);
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
        let new_amt = Money::from_dollars(new_amount);
        let limit = Money::from_dollars(self.hourly_transfer_limit);
        if new_amt <= Money::zero() {
            return Err(IroncladError::Policy {
                rule: "non_positive_amount".into(),
                reason: format!("payment amount must be positive, got {new_amount}"),
            });
        }
        let projected = Money::from_dollars(recent_hourly_total) + new_amt;
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
        let new_amt = Money::from_dollars(new_amount);
        let limit = Money::from_dollars(self.daily_transfer_limit);
        if new_amt <= Money::zero() {
            return Err(IroncladError::Policy {
                rule: "non_positive_amount".into(),
                reason: format!("payment amount must be positive, got {new_amount}"),
            });
        }
        let projected = Money::from_dollars(recent_daily_total) + new_amt;
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
        let remaining = Money::from_dollars(current_balance) - Money::from_dollars(amount);
        let reserve = Money::from_dollars(self.minimum_reserve);
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
        let projected = Money::from_dollars(daily_inference_total) + Money::from_dollars(new_cost);
        let budget = Money::from_dollars(self.daily_inference_budget);
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
        });
        assert!(policy.check_per_payment(0.01).is_err());
        assert!(policy.check_per_payment(1.0).is_err());
        assert!(policy.check_hourly_limit(0.0, 0.01).is_err());
        assert!(policy.check_daily_limit(0.0, 0.01).is_err());
        assert!(policy.check_inference_budget(0.0, 0.01).is_err());
    }
}
