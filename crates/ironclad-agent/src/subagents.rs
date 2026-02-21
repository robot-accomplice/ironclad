use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, Semaphore};
use tracing::{debug, warn};

use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentRunState {
    Idle,
    Running,
    Stopped,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub name: String,
    pub model: String,
    pub state: AgentRunState,
    pub session_count: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstanceConfig {
    pub id: String,
    pub name: String,
    pub model: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub allowed_subagents: Vec<String>,
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent: usize,
}

fn default_max_concurrent() -> usize {
    4
}

pub struct SubagentRegistry {
    agents: Mutex<HashMap<String, AgentInstance>>,
    concurrency: Arc<Semaphore>,
    max_concurrent: usize,
    allowed_ids: Vec<String>,
}

impl SubagentRegistry {
    pub fn new(max_concurrent: usize, allowed_ids: Vec<String>) -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            concurrency: Arc::new(Semaphore::new(max_concurrent)),
            max_concurrent,
            allowed_ids,
        }
    }

    pub fn is_allowed(&self, agent_id: &str) -> bool {
        self.allowed_ids.is_empty() || self.allowed_ids.iter().any(|id| id == agent_id)
    }

    pub async fn register(&self, config: AgentInstanceConfig) -> Result<()> {
        if !self.is_allowed(&config.id) {
            return Err(IroncladError::Config(format!(
                "agent '{}' is not in the allowed list",
                config.id
            )));
        }

        let instance = AgentInstance {
            id: config.id.clone(),
            name: config.name,
            model: config.model,
            state: AgentRunState::Idle,
            session_count: 0,
            started_at: None,
            last_error: None,
        };

        debug!(id = %config.id, "registered agent");
        let mut agents = self.agents.lock().await;
        agents.insert(config.id, instance);
        Ok(())
    }

    pub async fn start_agent(&self, agent_id: &str) -> Result<()> {
        let mut agents = self.agents.lock().await;
        let agent = agents.get_mut(agent_id)
            .ok_or_else(|| IroncladError::Config(format!("agent '{agent_id}' not found")))?;

        if agent.state == AgentRunState::Running {
            return Ok(());
        }

        agent.state = AgentRunState::Running;
        agent.started_at = Some(Utc::now());
        agent.last_error = None;

        debug!(id = agent_id, "agent started");
        Ok(())
    }

    pub async fn stop_agent(&self, agent_id: &str) -> Result<()> {
        let mut agents = self.agents.lock().await;
        let agent = agents.get_mut(agent_id)
            .ok_or_else(|| IroncladError::Config(format!("agent '{agent_id}' not found")))?;

        agent.state = AgentRunState::Stopped;
        debug!(id = agent_id, "agent stopped");
        Ok(())
    }

    pub async fn mark_error(&self, agent_id: &str, error: String) {
        let mut agents = self.agents.lock().await;
        if let Some(agent) = agents.get_mut(agent_id) {
            agent.state = AgentRunState::Error;
            agent.last_error = Some(error);
            warn!(id = agent_id, "agent errored");
        }
    }

    pub async fn get_agent(&self, agent_id: &str) -> Option<AgentInstance> {
        let agents = self.agents.lock().await;
        agents.get(agent_id).cloned()
    }

    pub async fn list_agents(&self) -> Vec<AgentInstance> {
        let agents = self.agents.lock().await;
        agents.values().cloned().collect()
    }

    pub async fn running_count(&self) -> usize {
        let agents = self.agents.lock().await;
        agents.values().filter(|a| a.state == AgentRunState::Running).count()
    }

    pub async fn agent_count(&self) -> usize {
        let agents = self.agents.lock().await;
        agents.len()
    }

    pub async fn acquire_slot(&self) -> Result<tokio::sync::OwnedSemaphorePermit> {
        Arc::clone(&self.concurrency)
            .acquire_owned()
            .await
            .map_err(|_| IroncladError::Config("concurrency semaphore closed".into()))
    }

    pub fn max_concurrent(&self) -> usize {
        self.max_concurrent
    }

    pub fn available_slots(&self) -> usize {
        self.concurrency.available_permits()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config(id: &str) -> AgentInstanceConfig {
        AgentInstanceConfig {
            id: id.into(),
            name: format!("Agent {id}"),
            model: "test-model".into(),
            skills: vec![],
            allowed_subagents: vec![],
            max_concurrent: 4,
        }
    }

    #[test]
    fn allowed_empty_means_all() {
        let reg = SubagentRegistry::new(4, vec![]);
        assert!(reg.is_allowed("anything"));
    }

    #[test]
    fn allowed_filters() {
        let reg = SubagentRegistry::new(4, vec!["a".into(), "b".into()]);
        assert!(reg.is_allowed("a"));
        assert!(!reg.is_allowed("c"));
    }

    #[tokio::test]
    async fn register_and_list() {
        let reg = SubagentRegistry::new(4, vec![]);
        reg.register(test_config("agent-1")).await.unwrap();
        assert_eq!(reg.agent_count().await, 1);
        let agents = reg.list_agents().await;
        assert_eq!(agents[0].id, "agent-1");
        assert_eq!(agents[0].state, AgentRunState::Idle);
    }

    #[tokio::test]
    async fn register_disallowed_fails() {
        let reg = SubagentRegistry::new(4, vec!["allowed".into()]);
        let result = reg.register(test_config("not-allowed")).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn start_and_stop() {
        let reg = SubagentRegistry::new(4, vec![]);
        reg.register(test_config("a")).await.unwrap();

        reg.start_agent("a").await.unwrap();
        let agent = reg.get_agent("a").await.unwrap();
        assert_eq!(agent.state, AgentRunState::Running);
        assert!(agent.started_at.is_some());

        reg.stop_agent("a").await.unwrap();
        let agent = reg.get_agent("a").await.unwrap();
        assert_eq!(agent.state, AgentRunState::Stopped);
    }

    #[tokio::test]
    async fn start_nonexistent_fails() {
        let reg = SubagentRegistry::new(4, vec![]);
        let result = reg.start_agent("nope").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn mark_error() {
        let reg = SubagentRegistry::new(4, vec![]);
        reg.register(test_config("e")).await.unwrap();
        reg.start_agent("e").await.unwrap();
        reg.mark_error("e", "something broke".into()).await;
        let agent = reg.get_agent("e").await.unwrap();
        assert_eq!(agent.state, AgentRunState::Error);
        assert_eq!(agent.last_error.as_deref(), Some("something broke"));
    }

    #[tokio::test]
    async fn running_count() {
        let reg = SubagentRegistry::new(4, vec![]);
        reg.register(test_config("a")).await.unwrap();
        reg.register(test_config("b")).await.unwrap();
        reg.start_agent("a").await.unwrap();
        assert_eq!(reg.running_count().await, 1);
    }

    #[tokio::test]
    async fn concurrency_slots() {
        let reg = SubagentRegistry::new(2, vec![]);
        assert_eq!(reg.available_slots(), 2);
        assert_eq!(reg.max_concurrent(), 2);
        let _permit = reg.acquire_slot().await.unwrap();
        assert_eq!(reg.available_slots(), 1);
    }

    #[test]
    fn agent_instance_config_defaults() {
        let cfg = test_config("test");
        assert_eq!(cfg.max_concurrent, 4);
        assert!(cfg.skills.is_empty());
        assert!(cfg.allowed_subagents.is_empty());
    }

    #[test]
    fn agent_run_state_serde() {
        for state in [AgentRunState::Idle, AgentRunState::Running, AgentRunState::Stopped, AgentRunState::Error] {
            let json = serde_json::to_string(&state).unwrap();
            let back: AgentRunState = serde_json::from_str(&json).unwrap();
            assert_eq!(state, back);
        }
    }
}
