use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tracing::info;

use ironclad_core::{IroncladError, Result};

/// A discovered agent on the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredAgent {
    pub agent_id: String,
    pub name: String,
    pub url: String,
    pub capabilities: Vec<String>,
    pub verified: bool,
    pub discovered_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    pub discovery_method: DiscoveryMethod,
}

/// How the agent was discovered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiscoveryMethod {
    DnsSd,
    MDns,
    Manual,
    A2AHandshake,
}

impl std::fmt::Display for DiscoveryMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoveryMethod::DnsSd => write!(f, "DNS-SD"),
            DiscoveryMethod::MDns => write!(f, "mDNS"),
            DiscoveryMethod::Manual => write!(f, "manual"),
            DiscoveryMethod::A2AHandshake => write!(f, "A2A"),
        }
    }
}

/// Manages discovered agents and their verification state.
pub struct DiscoveryRegistry {
    agents: HashMap<String, DiscoveredAgent>,
}

impl DiscoveryRegistry {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Register a newly discovered agent (unverified).
    pub fn register(&mut self, agent: DiscoveredAgent) {
        info!(
            id = %agent.agent_id,
            url = %agent.url,
            method = %agent.discovery_method,
            "discovered agent"
        );
        self.agents.insert(agent.agent_id.clone(), agent);
    }

    /// Mark a discovered agent as verified (after mutual auth).
    pub fn verify(&mut self, agent_id: &str) -> Result<()> {
        let agent = self
            .agents
            .get_mut(agent_id)
            .ok_or_else(|| IroncladError::Config(format!("agent '{}' not found", agent_id)))?;
        agent.verified = true;
        agent.last_seen = Utc::now();
        info!(id = agent_id, "agent verified");
        Ok(())
    }

    /// Update the last-seen timestamp.
    pub fn touch(&mut self, agent_id: &str) {
        if let Some(agent) = self.agents.get_mut(agent_id) {
            agent.last_seen = Utc::now();
        }
    }

    /// Remove a discovered agent.
    pub fn remove(&mut self, agent_id: &str) -> Option<DiscoveredAgent> {
        self.agents.remove(agent_id)
    }

    /// Get a discovered agent by ID.
    pub fn get(&self, agent_id: &str) -> Option<&DiscoveredAgent> {
        self.agents.get(agent_id)
    }

    /// List all verified agents.
    pub fn verified_agents(&self) -> Vec<&DiscoveredAgent> {
        self.agents.values().filter(|a| a.verified).collect()
    }

    /// List all agents.
    pub fn all_agents(&self) -> Vec<&DiscoveredAgent> {
        self.agents.values().collect()
    }

    /// Find agents by capability.
    pub fn find_by_capability(&self, capability: &str) -> Vec<&DiscoveredAgent> {
        self.agents
            .values()
            .filter(|a| a.verified && a.capabilities.iter().any(|c| c == capability))
            .collect()
    }

    /// Remove agents not seen since the given threshold.
    pub fn prune_stale(&mut self, max_age: chrono::Duration) -> usize {
        let cutoff = Utc::now() - max_age;
        let stale_ids: Vec<String> = self
            .agents
            .values()
            .filter(|a| a.last_seen < cutoff)
            .map(|a| a.agent_id.clone())
            .collect();
        let count = stale_ids.len();
        for id in stale_ids {
            self.agents.remove(&id);
        }
        if count > 0 {
            info!(pruned = count, "pruned stale discovered agents");
        }
        count
    }

    pub fn count(&self) -> usize {
        self.agents.len()
    }
}

impl Default for DiscoveryRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// DNS SRV record representation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SrvRecord {
    pub service: String,
    pub protocol: String,
    pub domain: String,
    pub port: u16,
    pub priority: u16,
    pub weight: u16,
    pub target: String,
}

/// DNS TXT record for capability advertisement.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxtRecord {
    pub service: String,
    pub entries: HashMap<String, String>,
}

/// Build SRV and TXT records for advertising this agent.
pub fn build_advertisement(
    agent_id: &str,
    domain: &str,
    port: u16,
    capabilities: &[String],
) -> (SrvRecord, TxtRecord) {
    let srv = SrvRecord {
        service: "_ironclad".to_string(),
        protocol: "_tcp".to_string(),
        domain: domain.to_string(),
        port,
        priority: 10,
        weight: 100,
        target: domain.to_string(),
    };

    let mut entries = HashMap::new();
    entries.insert("agent_id".to_string(), agent_id.to_string());
    entries.insert("caps".to_string(), capabilities.join(","));
    entries.insert("version".to_string(), "0.1".to_string());

    let txt = TxtRecord {
        service: "_ironclad._tcp".to_string(),
        entries,
    };

    (srv, txt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_agent(id: &str) -> DiscoveredAgent {
        DiscoveredAgent {
            agent_id: id.to_string(),
            name: format!("Agent {id}"),
            url: format!("http://{id}.local:3000"),
            capabilities: vec!["research".to_string(), "coding".to_string()],
            verified: false,
            discovered_at: Utc::now(),
            last_seen: Utc::now(),
            discovery_method: DiscoveryMethod::MDns,
        }
    }

    #[test]
    fn register_and_get() {
        let mut reg = DiscoveryRegistry::new();
        reg.register(test_agent("agent-1"));
        assert_eq!(reg.count(), 1);
        assert!(reg.get("agent-1").is_some());
    }

    #[test]
    fn verify_agent() {
        let mut reg = DiscoveryRegistry::new();
        reg.register(test_agent("agent-1"));
        assert!(reg.verified_agents().is_empty());

        reg.verify("agent-1").unwrap();
        assert_eq!(reg.verified_agents().len(), 1);
    }

    #[test]
    fn verify_nonexistent() {
        let mut reg = DiscoveryRegistry::new();
        assert!(reg.verify("nope").is_err());
    }

    #[test]
    fn remove_agent() {
        let mut reg = DiscoveryRegistry::new();
        reg.register(test_agent("agent-1"));
        let removed = reg.remove("agent-1");
        assert!(removed.is_some());
        assert_eq!(reg.count(), 0);
    }

    #[test]
    fn find_by_capability() {
        let mut reg = DiscoveryRegistry::new();
        let mut a1 = test_agent("a1");
        a1.verified = true;
        reg.register(a1);

        let mut a2 = test_agent("a2");
        a2.capabilities = vec!["finance".to_string()];
        a2.verified = true;
        reg.register(a2);

        assert_eq!(reg.find_by_capability("research").len(), 1);
        assert_eq!(reg.find_by_capability("finance").len(), 1);
        assert_eq!(reg.find_by_capability("unknown").len(), 0);
    }

    #[test]
    fn unverified_excluded_from_capability_search() {
        let mut reg = DiscoveryRegistry::new();
        reg.register(test_agent("unverified"));
        assert_eq!(reg.find_by_capability("research").len(), 0);
    }

    #[test]
    fn prune_stale() {
        let mut reg = DiscoveryRegistry::new();
        let mut old = test_agent("old");
        old.last_seen = Utc::now() - chrono::Duration::hours(48);
        reg.register(old);
        reg.register(test_agent("fresh"));

        let pruned = reg.prune_stale(chrono::Duration::hours(24));
        assert_eq!(pruned, 1);
        assert_eq!(reg.count(), 1);
    }

    #[test]
    fn build_advertisement_records() {
        let caps = vec!["research".to_string(), "coding".to_string()];
        let (srv, txt) = build_advertisement("agent-1", "myhost.local", 3000, &caps);
        assert_eq!(srv.port, 3000);
        assert_eq!(txt.entries["agent_id"], "agent-1");
        assert!(txt.entries["caps"].contains("research"));
    }

    #[test]
    fn discovery_method_display() {
        assert_eq!(format!("{}", DiscoveryMethod::DnsSd), "DNS-SD");
        assert_eq!(format!("{}", DiscoveryMethod::MDns), "mDNS");
        assert_eq!(format!("{}", DiscoveryMethod::Manual), "manual");
        assert_eq!(format!("{}", DiscoveryMethod::A2AHandshake), "A2A");
    }

    #[test]
    fn discovery_method_serde() {
        for method in [
            DiscoveryMethod::DnsSd,
            DiscoveryMethod::MDns,
            DiscoveryMethod::Manual,
            DiscoveryMethod::A2AHandshake,
        ] {
            let json = serde_json::to_string(&method).unwrap();
            let back: DiscoveryMethod = serde_json::from_str(&json).unwrap();
            assert_eq!(method, back);
        }
    }
}
