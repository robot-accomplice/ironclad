use ironclad_core::RiskLevel;
use serde_json::Value;
use std::marker::PhantomData;

/// Marker types for typestate pattern.
pub mod states {
    /// Tool call has not been evaluated by the policy engine.
    #[derive(Debug)]
    pub struct Unevaluated;

    /// Tool call has been approved by the policy engine.
    #[derive(Debug)]
    pub struct Approved;

    /// Tool call has been denied by the policy engine.
    #[derive(Debug)]
    pub struct Denied;

    /// Tool call has been executed.
    #[derive(Debug)]
    pub struct Executed;
}

/// A tool call request with compile-time state tracking.
/// Only `ToolCallRequest<Approved>` can be executed.
#[derive(Debug)]
pub struct ToolCallRequest<State> {
    pub tool_name: String,
    pub parameters: Value,
    pub risk_level: RiskLevel,
    _state: PhantomData<State>,
}

impl ToolCallRequest<states::Unevaluated> {
    /// Create a new unevaluated tool call request.
    pub fn new(tool_name: String, parameters: Value, risk_level: RiskLevel) -> Self {
        Self {
            tool_name,
            parameters,
            risk_level,
            _state: PhantomData,
        }
    }

    /// Approve the tool call (transitions Unevaluated -> Approved).
    pub fn approve(self) -> ToolCallRequest<states::Approved> {
        ToolCallRequest {
            tool_name: self.tool_name,
            parameters: self.parameters,
            risk_level: self.risk_level,
            _state: PhantomData,
        }
    }

    /// Deny the tool call (transitions Unevaluated -> Denied).
    pub fn deny(self) -> ToolCallRequest<states::Denied> {
        ToolCallRequest {
            tool_name: self.tool_name,
            parameters: self.parameters,
            risk_level: self.risk_level,
            _state: PhantomData,
        }
    }
}

impl ToolCallRequest<states::Approved> {
    /// Mark as executed (transitions Approved -> Executed).
    pub fn mark_executed(self) -> ToolCallRequest<states::Executed> {
        ToolCallRequest {
            tool_name: self.tool_name,
            parameters: self.parameters,
            risk_level: self.risk_level,
            _state: PhantomData,
        }
    }
}

impl<S> ToolCallRequest<S> {
    pub fn tool_name(&self) -> &str {
        &self.tool_name
    }

    pub fn parameters(&self) -> &Value {
        &self.parameters
    }

    pub fn risk_level(&self) -> &RiskLevel {
        &self.risk_level
    }
}

/// Agent lifecycle states as type-level markers.
pub mod lifecycle {
    #[derive(Debug)]
    pub struct Setup;

    #[derive(Debug)]
    pub struct Waking;

    #[derive(Debug)]
    pub struct Running;

    #[derive(Debug)]
    pub struct Sleeping;

    #[derive(Debug)]
    pub struct Dead;
}

/// Agent handle with compile-time lifecycle state tracking.
#[derive(Debug)]
pub struct AgentHandle<State> {
    pub agent_id: String,
    _state: PhantomData<State>,
}

impl AgentHandle<lifecycle::Setup> {
    pub fn new(agent_id: String) -> Self {
        Self {
            agent_id,
            _state: PhantomData,
        }
    }

    pub fn wake(self) -> AgentHandle<lifecycle::Waking> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }
}

impl AgentHandle<lifecycle::Waking> {
    pub fn start(self) -> AgentHandle<lifecycle::Running> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }
}

impl AgentHandle<lifecycle::Running> {
    pub fn sleep(self) -> AgentHandle<lifecycle::Sleeping> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }

    pub fn terminate(self) -> AgentHandle<lifecycle::Dead> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }
}

impl AgentHandle<lifecycle::Sleeping> {
    pub fn wake(self) -> AgentHandle<lifecycle::Waking> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }

    pub fn terminate(self) -> AgentHandle<lifecycle::Dead> {
        AgentHandle {
            agent_id: self.agent_id,
            _state: PhantomData,
        }
    }
}

impl<S> AgentHandle<S> {
    pub fn agent_id(&self) -> &str {
        &self.agent_id
    }
}

/// Treasury with const-generic spending limits.
#[derive(Debug)]
pub struct BoundedTreasury<const MAX_PER_TX: u64, const MAX_DAILY: u64> {
    pub balance: u64,
    pub spent_today: u64,
}

impl<const MAX_PER_TX: u64, const MAX_DAILY: u64> BoundedTreasury<MAX_PER_TX, MAX_DAILY> {
    pub fn new(balance: u64) -> Self {
        Self {
            balance,
            spent_today: 0,
        }
    }

    pub fn can_spend(&self, amount: u64) -> bool {
        amount <= MAX_PER_TX && self.spent_today + amount <= MAX_DAILY && amount <= self.balance
    }

    pub fn spend(&mut self, amount: u64) -> Result<(), &'static str> {
        if !self.can_spend(amount) {
            return Err("spending limit exceeded or insufficient balance");
        }
        self.balance -= amount;
        self.spent_today += amount;
        Ok(())
    }

    pub fn reset_daily(&mut self) {
        self.spent_today = 0;
    }

    pub fn max_per_tx() -> u64 {
        MAX_PER_TX
    }

    pub fn max_daily() -> u64 {
        MAX_DAILY
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_lifecycle() {
        let request = ToolCallRequest::new(
            "memory_search".into(),
            serde_json::json!({"query": "test"}),
            RiskLevel::Safe,
        );
        assert_eq!(request.tool_name(), "memory_search");

        let approved = request.approve();
        assert_eq!(approved.tool_name(), "memory_search");

        let executed = approved.mark_executed();
        assert_eq!(executed.tool_name(), "memory_search");
    }

    #[test]
    fn tool_call_deny() {
        let request = ToolCallRequest::new(
            "dangerous_tool".into(),
            serde_json::json!({}),
            RiskLevel::Dangerous,
        );
        let denied = request.deny();
        assert_eq!(denied.tool_name(), "dangerous_tool");
    }

    #[test]
    fn agent_lifecycle() {
        let agent = AgentHandle::<lifecycle::Setup>::new("agent-1".into());
        assert_eq!(agent.agent_id(), "agent-1");

        let waking = agent.wake();
        let running = waking.start();
        let sleeping = running.sleep();
        let waking_again = sleeping.wake();
        let running_again = waking_again.start();
        let _dead = running_again.terminate();
    }

    #[test]
    fn agent_setup_to_dead_via_sleep() {
        let agent = AgentHandle::<lifecycle::Setup>::new("a".into());
        let _dead = agent.wake().start().sleep().terminate();
    }

    #[test]
    fn bounded_treasury_limits() {
        let mut treasury = BoundedTreasury::<100, 500>::new(1000);
        assert!(treasury.can_spend(100));
        assert!(!treasury.can_spend(101));

        treasury.spend(100).unwrap();
        treasury.spend(100).unwrap();
        treasury.spend(100).unwrap();
        treasury.spend(100).unwrap();
        treasury.spend(100).unwrap();
        assert!(!treasury.can_spend(1));

        treasury.reset_daily();
        assert!(treasury.can_spend(100));
    }

    #[test]
    fn bounded_treasury_insufficient_balance() {
        let mut treasury = BoundedTreasury::<1000, 10000>::new(50);
        assert!(!treasury.can_spend(51));
        assert!(treasury.can_spend(50));
        treasury.spend(50).unwrap();
        assert!(!treasury.can_spend(1));
    }

    #[test]
    fn bounded_treasury_const_accessors() {
        assert_eq!(BoundedTreasury::<42, 999>::max_per_tx(), 42);
        assert_eq!(BoundedTreasury::<42, 999>::max_daily(), 999);
    }
}
