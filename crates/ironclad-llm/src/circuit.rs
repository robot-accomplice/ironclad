use std::collections::HashMap;
use std::time::{Duration, Instant};

use ironclad_core::config::CircuitBreakerConfig;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

#[derive(Debug)]
struct CircuitBreaker {
    state: CircuitState,
    failure_count: u32,
    last_failure_at: Option<Instant>,
    cooldown: Duration,
    max_cooldown: Duration,
    threshold: u32,
    window: Duration,
}

impl CircuitBreaker {
    fn new(config: &CircuitBreakerConfig) -> Self {
        Self {
            state: CircuitState::Closed,
            failure_count: 0,
            last_failure_at: None,
            cooldown: Duration::from_secs(config.cooldown_seconds),
            max_cooldown: Duration::from_secs(config.max_cooldown_seconds),
            threshold: config.threshold,
            window: Duration::from_secs(config.window_seconds),
        }
    }

    fn effective_state(&self) -> CircuitState {
        match self.state {
            CircuitState::Open => {
                if let Some(last) = self.last_failure_at
                    && last.elapsed() >= self.cooldown
                {
                    return CircuitState::HalfOpen;
                }
                CircuitState::Open
            }
            other => other,
        }
    }
}

#[derive(Debug)]
pub struct CircuitBreakerRegistry {
    breakers: HashMap<String, CircuitBreaker>,
    config: CircuitBreakerConfig,
}

#[cfg(test)]
impl CircuitBreakerRegistry {
    fn force_half_open(&mut self, provider: &str) {
        let cb = self.get_or_create(provider);
        cb.last_failure_at = Some(Instant::now() - cb.cooldown - Duration::from_millis(1));
    }
}

impl CircuitBreakerRegistry {
    pub fn new(config: &CircuitBreakerConfig) -> Self {
        Self {
            breakers: HashMap::new(),
            config: config.clone(),
        }
    }

    fn get_or_create(&mut self, provider: &str) -> &mut CircuitBreaker {
        let config = self.config.clone();
        self.breakers
            .entry(provider.to_string())
            .or_insert_with(|| CircuitBreaker::new(&config))
    }

    pub fn is_blocked(&self, provider: &str) -> bool {
        match self.breakers.get(provider) {
            Some(cb) => cb.effective_state() == CircuitState::Open,
            None => false,
        }
    }

    pub fn record_success(&mut self, provider: &str) {
        let base_cooldown = self.config.cooldown_seconds;
        let cb = self.get_or_create(provider);
        match cb.effective_state() {
            CircuitState::HalfOpen => {
                cb.state = CircuitState::Closed;
                cb.failure_count = 0;
                cb.cooldown = Duration::from_secs(base_cooldown);
            }
            CircuitState::Closed => {
                cb.failure_count = 0;
            }
            CircuitState::Open => {}
        }
    }

    pub fn record_failure(&mut self, provider: &str) {
        let cb = self.get_or_create(provider);
        match cb.effective_state() {
            CircuitState::HalfOpen => {
                cb.state = CircuitState::Open;
                cb.last_failure_at = Some(Instant::now());
                let doubled = cb.cooldown * 2;
                cb.cooldown = doubled.min(cb.max_cooldown);
            }
            CircuitState::Closed => {
                let now = Instant::now();
                if let Some(last) = cb.last_failure_at
                    && now.duration_since(last) > cb.window
                {
                    cb.failure_count = 0;
                }
                cb.failure_count += 1;
                cb.last_failure_at = Some(now);
                if cb.failure_count >= cb.threshold {
                    cb.state = CircuitState::Open;
                }
            }
            CircuitState::Open => {}
        }
    }

    pub fn record_credit_error(&mut self, provider: &str) {
        let credit_cooldown = self.config.credit_cooldown_seconds;
        let cb = self.get_or_create(provider);
        cb.state = CircuitState::Open;
        cb.last_failure_at = Some(Instant::now());
        cb.cooldown = Duration::from_secs(credit_cooldown);
    }

    pub fn reset(&mut self, provider: &str) {
        let base_cooldown = self.config.cooldown_seconds;
        let cb = self.get_or_create(provider);
        cb.state = CircuitState::Closed;
        cb.failure_count = 0;
        cb.last_failure_at = None;
        cb.cooldown = Duration::from_secs(base_cooldown);
    }

    pub fn get_state(&self, provider: &str) -> CircuitState {
        match self.breakers.get(provider) {
            Some(cb) => cb.effective_state(),
            None => CircuitState::Closed,
        }
    }

    pub fn list_providers(&self) -> Vec<(String, CircuitState)> {
        self.breakers
            .iter()
            .map(|(name, cb)| (name.clone(), cb.effective_state()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> CircuitBreakerConfig {
        CircuitBreakerConfig {
            threshold: 3,
            window_seconds: 60,
            cooldown_seconds: 2,
            credit_cooldown_seconds: 10,
            max_cooldown_seconds: 30,
        }
    }

    #[test]
    fn normal_operation() {
        let mut reg = CircuitBreakerRegistry::new(&test_config());
        assert!(!reg.is_blocked("openai"));
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);

        reg.record_success("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);
        assert!(!reg.is_blocked("openai"));
    }

    #[test]
    fn trip_on_threshold() {
        let mut reg = CircuitBreakerRegistry::new(&test_config());

        reg.record_failure("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);
        reg.record_failure("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);
        reg.record_failure("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::Open);
        assert!(reg.is_blocked("openai"));
    }

    #[test]
    fn recovery_after_cooldown() {
        let config = CircuitBreakerConfig {
            threshold: 1,
            cooldown_seconds: 0,
            ..test_config()
        };
        let mut reg = CircuitBreakerRegistry::new(&config);

        reg.record_failure("openai");
        // 0s cooldown means effective_state transitions to HalfOpen immediately
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(reg.get_state("openai"), CircuitState::HalfOpen);

        reg.record_success("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);
        assert!(!reg.is_blocked("openai"));
    }

    #[test]
    fn credit_error_immediate_trip() {
        let mut reg = CircuitBreakerRegistry::new(&test_config());
        assert_eq!(reg.get_state("anthropic"), CircuitState::Closed);

        reg.record_credit_error("anthropic");
        assert_eq!(reg.get_state("anthropic"), CircuitState::Open);
        assert!(reg.is_blocked("anthropic"));
    }

    #[test]
    fn reset_clears_state() {
        let mut reg = CircuitBreakerRegistry::new(&test_config());
        reg.record_credit_error("openai");
        assert!(reg.is_blocked("openai"));

        reg.reset("openai");
        assert!(!reg.is_blocked("openai"));
        assert_eq!(reg.get_state("openai"), CircuitState::Closed);
    }

    #[test]
    fn half_open_failure_doubles_cooldown() {
        let config = CircuitBreakerConfig {
            threshold: 1,
            cooldown_seconds: 1,
            max_cooldown_seconds: 8,
            ..test_config()
        };
        let mut reg = CircuitBreakerRegistry::new(&config);

        reg.record_failure("openai");
        reg.force_half_open("openai");
        assert_eq!(reg.get_state("openai"), CircuitState::HalfOpen);

        reg.record_failure("openai");
        // Should be Open again with doubled cooldown (1s -> 2s)
        assert_eq!(reg.get_state("openai"), CircuitState::Open);
    }
}
