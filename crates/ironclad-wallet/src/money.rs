//! Fixed-point money type (cents) for treasury and financial logic.

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Money(i64); // cents

impl Money {
    pub fn from_dollars(dollars: f64) -> Self {
        let cents = (dollars * 100.0).round();
        assert!(
            cents >= i64::MIN as f64 && cents <= i64::MAX as f64,
            "dollar amount out of representable range"
        );
        Money(cents as i64)
    }

    pub fn dollars(&self) -> f64 {
        self.0 as f64 / 100.0
    }

    pub fn cents(&self) -> i64 {
        self.0
    }

    pub fn zero() -> Self {
        Money(0)
    }
}

impl std::fmt::Display for Money {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "${:.2}", self.dollars())
    }
}

impl std::ops::Add for Money {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Money(self.0.saturating_add(rhs.0))
    }
}

impl std::ops::Sub for Money {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Money(self.0.saturating_sub(rhs.0))
    }
}

impl Money {
    pub fn checked_add(self, rhs: Self) -> Option<Self> {
        self.0.checked_add(rhs.0).map(Money)
    }

    pub fn checked_sub(self, rhs: Self) -> Option<Self> {
        self.0.checked_sub(rhs.0).map(Money)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_dollars_roundtrip() {
        assert_eq!(Money::from_dollars(0.0).cents(), 0);
        assert_eq!(Money::from_dollars(1.0).cents(), 100);
        assert_eq!(Money::from_dollars(10.50).cents(), 1050);
        assert_eq!(Money::from_dollars(99.99).cents(), 9999);
        assert_eq!(Money::from_dollars(99.99).dollars(), 99.99);
        assert!((Money::from_dollars(33.33).dollars() - 33.33).abs() < 0.001);
    }

    #[test]
    fn display_format() {
        assert_eq!(Money::from_dollars(0.0).to_string(), "$0.00");
        assert_eq!(Money::from_dollars(1.5).to_string(), "$1.50");
        assert_eq!(Money::from_dollars(100.0).to_string(), "$100.00");
    }

    #[test]
    fn arithmetic() {
        let a = Money::from_dollars(10.00);
        let b = Money::from_dollars(5.50);
        assert_eq!((a + b).dollars(), 15.50);
        assert_eq!((a - b).dollars(), 4.50);
        assert_eq!(Money::zero() + a, a);
        assert_eq!(a - a, Money::zero());
    }

    #[test]
    fn saturating_arithmetic() {
        let max_cents = Money(i64::MAX);
        let one = Money(1);
        assert_eq!(max_cents + one, Money(i64::MAX), "add saturates at MAX");
        let min_cents = Money(i64::MIN);
        assert_eq!(min_cents - one, Money(i64::MIN), "sub saturates at MIN");
    }

    #[test]
    fn checked_arithmetic() {
        let a = Money::from_dollars(10.00);
        let b = Money::from_dollars(5.50);
        assert_eq!(a.checked_add(b).unwrap().dollars(), 15.50);
        assert_eq!(a.checked_sub(b).unwrap().dollars(), 4.50);
        assert!(Money(i64::MAX).checked_add(Money(1)).is_none());
        assert!(Money(i64::MIN).checked_sub(Money(1)).is_none());
    }

    #[test]
    fn money_add_saturates_on_overflow() {
        let big = Money(i64::MAX);
        let one = Money(1);
        let result = big + one;
        assert_eq!(result, Money(i64::MAX)); // saturates, doesn't wrap
    }

    #[test]
    fn money_sub_saturates_on_underflow() {
        let small = Money(i64::MIN);
        let one = Money(1);
        let result = small - one;
        assert_eq!(result, Money(i64::MIN)); // saturates, doesn't wrap
    }

    #[test]
    fn money_checked_add_returns_none_on_overflow() {
        let big = Money(i64::MAX);
        let one = Money(1);
        assert!(big.checked_add(one).is_none());
    }

    #[test]
    fn money_checked_sub_returns_none_on_underflow() {
        let small = Money(i64::MIN);
        let one = Money(1);
        assert!(small.checked_sub(one).is_none());
    }

    #[test]
    #[should_panic(expected = "out of representable range")]
    fn from_dollars_rejects_extreme_positive() {
        Money::from_dollars(f64::MAX);
    }

    #[test]
    #[should_panic(expected = "out of representable range")]
    fn from_dollars_rejects_extreme_negative() {
        Money::from_dollars(f64::MIN);
    }
}
