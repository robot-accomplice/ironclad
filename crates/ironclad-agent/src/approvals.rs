use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};
use uuid::Uuid;

use ironclad_core::config::ApprovalsConfig;
use ironclad_core::{InputAuthority, IroncladError, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolClassification {
    Safe,
    Gated,
    Blocked,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ApprovalStatus {
    Pending,
    Approved,
    Denied,
    TimedOut,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: String,
    pub tool_name: String,
    pub tool_input: String,
    pub session_id: Option<String>,
    #[serde(default = "default_requested_authority")]
    pub requested_authority: InputAuthority,
    pub status: ApprovalStatus,
    pub decided_by: Option<String>,
    pub decided_at: Option<DateTime<Utc>>,
    pub timeout_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

fn default_requested_authority() -> InputAuthority {
    InputAuthority::External
}

pub struct ApprovalManager {
    config: ApprovalsConfig,
    pending: Arc<Mutex<HashMap<String, ApprovalRequest>>>,
}

impl ApprovalManager {
    pub fn new(config: ApprovalsConfig) -> Self {
        Self {
            config,
            pending: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn classify_tool(&self, tool_name: &str) -> ToolClassification {
        if self.config.blocked_tools.iter().any(|t| t == tool_name) {
            ToolClassification::Blocked
        } else if self.config.gated_tools.iter().any(|t| t == tool_name) {
            ToolClassification::Gated
        } else {
            ToolClassification::Safe
        }
    }

    pub fn check_tool(&self, tool_name: &str) -> Result<ToolClassification> {
        if !self.config.enabled {
            return Ok(ToolClassification::Safe);
        }

        let classification = self.classify_tool(tool_name);

        if classification == ToolClassification::Blocked {
            return Err(IroncladError::Tool {
                tool: tool_name.to_string(),
                message: "tool is blocked by policy".into(),
            });
        }

        Ok(classification)
    }

    pub fn request_approval(
        &self,
        tool_name: &str,
        tool_input: &str,
        session_id: Option<&str>,
        requested_authority: InputAuthority,
    ) -> Result<ApprovalRequest> {
        let id = Uuid::new_v4().to_string();
        let timeout_at = Utc::now() + Duration::seconds(self.config.timeout_seconds as i64);

        let request = ApprovalRequest {
            id: id.clone(),
            tool_name: tool_name.to_string(),
            tool_input: tool_input.to_string(),
            session_id: session_id.map(|s| s.to_string()),
            requested_authority,
            status: ApprovalStatus::Pending,
            decided_by: None,
            decided_at: None,
            timeout_at,
            created_at: Utc::now(),
        };

        debug!(id = %id, tool = tool_name, "approval requested");

        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.insert(id, request.clone());

        Ok(request)
    }

    pub fn approve(&self, request_id: &str, decided_by: &str) -> Result<ApprovalRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let request = pending
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Tool {
                tool: "approvals".into(),
                message: format!("request {request_id} not found"),
            })?;

        if request.status != ApprovalStatus::Pending {
            return Err(IroncladError::Tool {
                tool: "approvals".into(),
                message: format!("request {request_id} is already {:?}", request.status),
            });
        }

        request.status = ApprovalStatus::Approved;
        request.decided_by = Some(decided_by.to_string());
        request.decided_at = Some(Utc::now());

        debug!(id = request_id, by = decided_by, "approval granted");
        Ok(request.clone())
    }

    pub fn deny(&self, request_id: &str, decided_by: &str) -> Result<ApprovalRequest> {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let request = pending
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Tool {
                tool: "approvals".into(),
                message: format!("request {request_id} not found"),
            })?;

        if request.status != ApprovalStatus::Pending {
            return Err(IroncladError::Tool {
                tool: "approvals".into(),
                message: format!("request {request_id} is already {:?}", request.status),
            });
        }

        request.status = ApprovalStatus::Denied;
        request.decided_by = Some(decided_by.to_string());
        request.decided_at = Some(Utc::now());

        warn!(id = request_id, by = decided_by, "approval denied");
        Ok(request.clone())
    }

    pub fn get_request(&self, request_id: &str) -> Option<ApprovalRequest> {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.get(request_id).cloned()
    }

    pub fn list_pending(&self) -> Vec<ApprovalRequest> {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending
            .values()
            .filter(|r| r.status == ApprovalStatus::Pending)
            .cloned()
            .collect()
    }

    pub fn list_all(&self) -> Vec<ApprovalRequest> {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        pending.values().cloned().collect()
    }

    pub fn expire_timed_out(&self) -> Vec<String> {
        let now = Utc::now();
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let mut expired = Vec::new();

        for (id, request) in pending.iter_mut() {
            if request.status == ApprovalStatus::Pending && now >= request.timeout_at {
                request.status = ApprovalStatus::TimedOut;
                expired.push(id.clone());
                debug!(id = %id, tool = %request.tool_name, "approval timed out");
            }
        }

        expired
    }

    pub fn clear_decided(&self) -> usize {
        let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let before = pending.len();
        pending.retain(|_, r| r.status == ApprovalStatus::Pending);
        before - pending.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_config() -> ApprovalsConfig {
        ApprovalsConfig {
            enabled: true,
            gated_tools: vec!["shell".into(), "write_file".into()],
            blocked_tools: vec!["rm_rf".into()],
            timeout_seconds: 60,
        }
    }

    fn disabled_config() -> ApprovalsConfig {
        ApprovalsConfig {
            enabled: false,
            ..test_config()
        }
    }

    #[test]
    fn classify_safe_tool() {
        let mgr = ApprovalManager::new(test_config());
        assert_eq!(mgr.classify_tool("read_file"), ToolClassification::Safe);
    }

    #[test]
    fn classify_gated_tool() {
        let mgr = ApprovalManager::new(test_config());
        assert_eq!(mgr.classify_tool("shell"), ToolClassification::Gated);
        assert_eq!(mgr.classify_tool("write_file"), ToolClassification::Gated);
    }

    #[test]
    fn classify_blocked_tool() {
        let mgr = ApprovalManager::new(test_config());
        assert_eq!(mgr.classify_tool("rm_rf"), ToolClassification::Blocked);
    }

    #[test]
    fn check_tool_blocked_returns_error() {
        let mgr = ApprovalManager::new(test_config());
        let result = mgr.check_tool("rm_rf");
        assert!(result.is_err());
    }

    #[test]
    fn check_tool_disabled_always_safe() {
        let mgr = ApprovalManager::new(disabled_config());
        assert_eq!(mgr.check_tool("shell").unwrap(), ToolClassification::Safe);
        assert_eq!(mgr.check_tool("rm_rf").unwrap(), ToolClassification::Safe);
    }

    #[test]
    fn request_approval_creates_pending() {
        let mgr = ApprovalManager::new(test_config());
        let req = mgr
            .request_approval("shell", "ls -la", Some("sess-1"), InputAuthority::External)
            .unwrap();
        assert_eq!(req.status, ApprovalStatus::Pending);
        assert_eq!(req.tool_name, "shell");
        assert_eq!(req.requested_authority, InputAuthority::External);
        assert!(req.decided_by.is_none());
    }

    #[test]
    fn request_approval_preserves_requested_authority() {
        let mgr = ApprovalManager::new(test_config());
        let req = mgr
            .request_approval("shell", "ls", None, InputAuthority::Peer)
            .unwrap();
        assert_eq!(req.requested_authority, InputAuthority::Peer);
    }

    #[test]
    fn approve_request() {
        let mgr = ApprovalManager::new(test_config());
        let req = mgr
            .request_approval("shell", "ls", None, InputAuthority::External)
            .unwrap();
        let approved = mgr.approve(&req.id, "admin").unwrap();
        assert_eq!(approved.status, ApprovalStatus::Approved);
        assert_eq!(approved.decided_by.as_deref(), Some("admin"));
    }

    #[test]
    fn deny_request() {
        let mgr = ApprovalManager::new(test_config());
        let req = mgr
            .request_approval("write_file", "{}", None, InputAuthority::External)
            .unwrap();
        let denied = mgr.deny(&req.id, "admin").unwrap();
        assert_eq!(denied.status, ApprovalStatus::Denied);
    }

    #[test]
    fn approve_nonexistent_fails() {
        let mgr = ApprovalManager::new(test_config());
        let result = mgr.approve("nonexistent", "admin");
        assert!(result.is_err());
    }

    #[test]
    fn double_approve_fails() {
        let mgr = ApprovalManager::new(test_config());
        let req = mgr
            .request_approval("shell", "cmd", None, InputAuthority::External)
            .unwrap();
        mgr.approve(&req.id, "admin").unwrap();
        let result = mgr.approve(&req.id, "admin2");
        assert!(result.is_err());
    }

    #[test]
    fn list_pending_filters() {
        let mgr = ApprovalManager::new(test_config());
        mgr.request_approval("shell", "1", None, InputAuthority::External)
            .unwrap();
        let req2 = mgr
            .request_approval("write_file", "2", None, InputAuthority::External)
            .unwrap();
        mgr.approve(&req2.id, "admin").unwrap();

        let pending = mgr.list_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].tool_name, "shell");
    }

    #[test]
    fn expire_timed_out() {
        let mgr = ApprovalManager::new(ApprovalsConfig {
            timeout_seconds: 0,
            ..test_config()
        });
        mgr.request_approval("shell", "cmd", None, InputAuthority::External)
            .unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        let expired = mgr.expire_timed_out();
        assert_eq!(expired.len(), 1);
        assert_eq!(mgr.list_pending().len(), 0);
    }

    #[test]
    fn clear_decided() {
        let mgr = ApprovalManager::new(test_config());
        mgr.request_approval("shell", "1", None, InputAuthority::External)
            .unwrap();
        let req2 = mgr
            .request_approval("write_file", "2", None, InputAuthority::External)
            .unwrap();
        mgr.approve(&req2.id, "admin").unwrap();

        let cleared = mgr.clear_decided();
        assert_eq!(cleared, 1);
        assert_eq!(mgr.list_all().len(), 1);
    }
}
