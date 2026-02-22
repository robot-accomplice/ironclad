use chrono::{DateTime, Utc};
use ironclad_core::{IroncladError, Result};
use k256::ecdsa::SigningKey;
use rand::rngs::OsRng;
use serde::{Deserialize, Serialize};
use sha3::{Digest, Keccak256};
use std::collections::HashMap;
use tracing::{debug, info, warn};

fn derive_child_address() -> String {
    let signing_key = SigningKey::random(&mut OsRng);
    let verifying_key = signing_key.verifying_key();
    let pubkey_point = verifying_key.to_encoded_point(false);
    let pubkey_bytes = &pubkey_point.as_bytes()[1..];
    let hash = Keccak256::digest(pubkey_bytes);
    let addr_bytes = &hash[hash.len() - 20..];
    format!("0x{}", hex::encode(addr_bytes))
}

/// Configuration for spawning a child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnConfig {
    pub parent_id: String,
    pub child_name: String,
    pub task_description: String,
    pub budget_usdc: f64,
    pub timeout_seconds: u64,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub model_preference: Option<String>,
}

/// A spawned child agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnedAgent {
    pub child_id: String,
    pub parent_id: String,
    pub name: String,
    pub task: String,
    pub budget_usdc: f64,
    pub spent_usdc: f64,
    pub status: SpawnStatus,
    pub wallet_address: Option<String>,
    pub spawned_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub result: Option<String>,
}

/// Status of a spawned child agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SpawnStatus {
    Provisioning,
    Running,
    Completed,
    Failed,
    TimedOut,
    Reclaimed,
}

impl std::fmt::Display for SpawnStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpawnStatus::Provisioning => write!(f, "provisioning"),
            SpawnStatus::Running => write!(f, "running"),
            SpawnStatus::Completed => write!(f, "completed"),
            SpawnStatus::Failed => write!(f, "failed"),
            SpawnStatus::TimedOut => write!(f, "timed_out"),
            SpawnStatus::Reclaimed => write!(f, "reclaimed"),
        }
    }
}

/// Manages the lifecycle of spawned child agents.
pub struct SpawnManager {
    children: HashMap<String, SpawnedAgent>,
    spawn_counter: u64,
    max_children_per_parent: usize,
}

impl SpawnManager {
    pub fn new(max_children_per_parent: usize) -> Self {
        Self {
            children: HashMap::new(),
            spawn_counter: 0,
            max_children_per_parent,
        }
    }

    /// Spawn a child agent with the given configuration.
    pub fn spawn(&mut self, config: SpawnConfig) -> Result<SpawnedAgent> {
        if config.budget_usdc < 0.0 {
            return Err(IroncladError::Config("budget cannot be negative".into()));
        }

        let parent_children = self
            .children
            .values()
            .filter(|c| c.parent_id == config.parent_id && c.status == SpawnStatus::Running)
            .count();

        if parent_children >= self.max_children_per_parent {
            return Err(IroncladError::Config(format!(
                "parent '{}' has reached max children ({})",
                config.parent_id, self.max_children_per_parent
            )));
        }

        self.spawn_counter += 1;
        let child_id = format!("child_{}_{}", config.parent_id, self.spawn_counter);

        let child = SpawnedAgent {
            child_id: child_id.clone(),
            parent_id: config.parent_id,
            name: config.child_name,
            task: config.task_description,
            budget_usdc: config.budget_usdc,
            spent_usdc: 0.0,
            status: SpawnStatus::Provisioning,
            wallet_address: Some(derive_child_address()),
            spawned_at: Utc::now(),
            completed_at: None,
            result: None,
        };

        info!(
            child_id = %child.child_id,
            parent = %child.parent_id,
            budget = child.budget_usdc,
            "spawned child agent"
        );

        self.children.insert(child_id, child.clone());
        Ok(child)
    }

    /// Activate a provisioned child agent.
    pub fn activate(&mut self, child_id: &str) -> Result<()> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        if child.status != SpawnStatus::Provisioning {
            return Err(IroncladError::Config(format!(
                "child '{}' is not in provisioning state",
                child_id
            )));
        }

        child.status = SpawnStatus::Running;
        debug!(child_id, "child agent activated");
        Ok(())
    }

    /// Record spending by a child agent.
    pub fn record_spending(&mut self, child_id: &str, amount: f64) -> Result<()> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        if child.spent_usdc + amount > child.budget_usdc {
            return Err(IroncladError::Config(format!(
                "child '{}' would exceed budget ({} + {} > {})",
                child_id, child.spent_usdc, amount, child.budget_usdc
            )));
        }

        child.spent_usdc += amount;
        debug!(
            child_id,
            spent = amount,
            total = child.spent_usdc,
            "child spending recorded"
        );
        Ok(())
    }

    /// Complete a child agent's task. Returns remaining unspent budget for reclamation.
    pub fn complete(&mut self, child_id: &str, result: String) -> Result<f64> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        child.status = SpawnStatus::Completed;
        child.completed_at = Some(Utc::now());
        child.result = Some(result);

        let remaining = child.budget_usdc - child.spent_usdc;
        info!(child_id, remaining, "child completed, funds to reclaim");
        Ok(remaining)
    }

    /// Mark a child as failed. Returns remaining unspent budget for reclamation.
    pub fn fail(&mut self, child_id: &str, error: &str) -> Result<f64> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        child.status = SpawnStatus::Failed;
        child.completed_at = Some(Utc::now());
        child.result = Some(format!("FAILED: {}", error));

        let remaining = child.budget_usdc - child.spent_usdc;
        warn!(child_id, error, remaining, "child failed");
        Ok(remaining)
    }

    /// Mark a child as timed out. Returns remaining unspent budget for reclamation.
    pub fn timeout(&mut self, child_id: &str) -> Result<f64> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        child.status = SpawnStatus::TimedOut;
        child.completed_at = Some(Utc::now());

        let remaining = child.budget_usdc - child.spent_usdc;
        warn!(child_id, remaining, "child timed out");
        Ok(remaining)
    }

    /// Mark funds as reclaimed after completion/failure/timeout.
    pub fn mark_reclaimed(&mut self, child_id: &str) -> Result<()> {
        let child = self
            .children
            .get_mut(child_id)
            .ok_or_else(|| IroncladError::Config(format!("child '{}' not found", child_id)))?;

        child.status = SpawnStatus::Reclaimed;
        info!(child_id, "funds reclaimed");
        Ok(())
    }

    /// Get a child agent by ID.
    pub fn get(&self, child_id: &str) -> Option<&SpawnedAgent> {
        self.children.get(child_id)
    }

    /// List children of a parent.
    pub fn children_of(&self, parent_id: &str) -> Vec<&SpawnedAgent> {
        self.children
            .values()
            .filter(|c| c.parent_id == parent_id)
            .collect()
    }

    /// List all active (running) children.
    pub fn active_children(&self) -> Vec<&SpawnedAgent> {
        self.children
            .values()
            .filter(|c| c.status == SpawnStatus::Running)
            .collect()
    }

    /// Check for children that have exceeded their timeout (hardcoded 1h ceiling).
    pub fn check_timeouts(&mut self, now: DateTime<Utc>) -> Vec<String> {
        self.children
            .values()
            .filter(|c| c.status == SpawnStatus::Running)
            .filter(|c| {
                let elapsed = (now - c.spawned_at).num_seconds();
                elapsed > 3600
            })
            .map(|c| c.child_id.clone())
            .collect()
    }

    pub fn total_count(&self) -> usize {
        self.children.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> SpawnConfig {
        SpawnConfig {
            parent_id: "parent-1".into(),
            child_name: "Research Agent".into(),
            task_description: "Research quantum computing".into(),
            budget_usdc: 10.0,
            timeout_seconds: 3600,
            allowed_tools: vec!["http_get".into(), "memory_search".into()],
            model_preference: Some("openai/gpt-4o".into()),
        }
    }

    #[test]
    fn spawn_child() {
        let mut mgr = SpawnManager::new(5);
        let child = mgr.spawn(test_config()).unwrap();
        assert!(child.child_id.contains("parent-1"));
        assert_eq!(child.status, SpawnStatus::Provisioning);
        assert!(child.wallet_address.is_some());
    }

    #[test]
    fn spawn_negative_budget() {
        let mut mgr = SpawnManager::new(5);
        let mut config = test_config();
        config.budget_usdc = -1.0;
        assert!(mgr.spawn(config).is_err());
    }

    #[test]
    fn spawn_max_children() {
        let mut mgr = SpawnManager::new(2);
        let c1 = mgr.spawn(test_config()).unwrap();
        mgr.activate(&c1.child_id).unwrap();
        let c2 = mgr.spawn(test_config()).unwrap();
        mgr.activate(&c2.child_id).unwrap();

        assert!(mgr.spawn(test_config()).is_err());
    }

    #[test]
    fn full_lifecycle() {
        let mut mgr = SpawnManager::new(5);
        let child = mgr.spawn(test_config()).unwrap();
        let id = child.child_id.clone();

        mgr.activate(&id).unwrap();
        assert_eq!(mgr.get(&id).unwrap().status, SpawnStatus::Running);

        mgr.record_spending(&id, 3.0).unwrap();
        mgr.record_spending(&id, 2.0).unwrap();
        assert!((mgr.get(&id).unwrap().spent_usdc - 5.0).abs() < f64::EPSILON);

        let remaining = mgr.complete(&id, "done".into()).unwrap();
        assert!((remaining - 5.0).abs() < f64::EPSILON);

        mgr.mark_reclaimed(&id).unwrap();
        assert_eq!(mgr.get(&id).unwrap().status, SpawnStatus::Reclaimed);
    }

    #[test]
    fn budget_exceeded() {
        let mut mgr = SpawnManager::new(5);
        let child = mgr.spawn(test_config()).unwrap();
        mgr.activate(&child.child_id).unwrap();
        assert!(mgr.record_spending(&child.child_id, 11.0).is_err());
    }

    #[test]
    fn fail_child() {
        let mut mgr = SpawnManager::new(5);
        let child = mgr.spawn(test_config()).unwrap();
        mgr.activate(&child.child_id).unwrap();
        mgr.record_spending(&child.child_id, 2.0).unwrap();
        let remaining = mgr.fail(&child.child_id, "crashed").unwrap();
        assert!((remaining - 8.0).abs() < f64::EPSILON);
    }

    #[test]
    fn timeout_child() {
        let mut mgr = SpawnManager::new(5);
        let child = mgr.spawn(test_config()).unwrap();
        mgr.activate(&child.child_id).unwrap();
        let remaining = mgr.timeout(&child.child_id).unwrap();
        assert!((remaining - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn children_of_parent() {
        let mut mgr = SpawnManager::new(5);
        mgr.spawn(test_config()).unwrap();

        let mut config2 = test_config();
        config2.parent_id = "parent-2".into();
        mgr.spawn(config2).unwrap();

        assert_eq!(mgr.children_of("parent-1").len(), 1);
        assert_eq!(mgr.children_of("parent-2").len(), 1);
    }

    #[test]
    fn status_display() {
        assert_eq!(format!("{}", SpawnStatus::Provisioning), "provisioning");
        assert_eq!(format!("{}", SpawnStatus::Running), "running");
        assert_eq!(format!("{}", SpawnStatus::Completed), "completed");
        assert_eq!(format!("{}", SpawnStatus::TimedOut), "timed_out");
        assert_eq!(format!("{}", SpawnStatus::Reclaimed), "reclaimed");
    }

    #[test]
    fn status_serde() {
        for status in [
            SpawnStatus::Provisioning,
            SpawnStatus::Running,
            SpawnStatus::Completed,
            SpawnStatus::Failed,
            SpawnStatus::TimedOut,
            SpawnStatus::Reclaimed,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: SpawnStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }
}
