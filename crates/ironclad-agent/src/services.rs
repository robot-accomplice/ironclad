use chrono::{DateTime, Utc};
use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info, warn};

/// A service the agent can offer to other agents or users.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDefinition {
    pub id: String,
    pub name: String,
    pub description: String,
    pub price_usdc: f64,
    #[serde(default)]
    pub capabilities_required: Vec<String>,
    #[serde(default)]
    pub max_concurrent: usize,
    #[serde(default)]
    pub estimated_duration_seconds: u64,
}

/// A request for a service from a client.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceRequest {
    pub id: String,
    pub service_id: String,
    pub requester: String,
    pub parameters: serde_json::Value,
    pub status: ServiceStatus,
    pub payment_tx: Option<String>,
    pub created_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
}

/// Status of a service request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ServiceStatus {
    Quoted,
    PaymentPending,
    PaymentVerified,
    InProgress,
    Completed,
    Failed,
    Refunded,
}

/// Manages the service catalog and request lifecycle.
pub struct ServiceManager {
    catalog: HashMap<String, ServiceDefinition>,
    requests: HashMap<String, ServiceRequest>,
    request_counter: u64,
}

impl ServiceManager {
    pub fn new() -> Self {
        Self {
            catalog: HashMap::new(),
            requests: HashMap::new(),
            request_counter: 0,
        }
    }

    /// Register a service in the catalog.
    pub fn register_service(&mut self, service: ServiceDefinition) -> Result<()> {
        if service.id.is_empty() {
            return Err(IroncladError::Config("service id cannot be empty".into()));
        }
        if service.price_usdc < 0.0 {
            return Err(IroncladError::Config("price cannot be negative".into()));
        }
        info!(id = %service.id, name = %service.name, price = service.price_usdc, "registered service");
        self.catalog.insert(service.id.clone(), service);
        Ok(())
    }

    /// Get a service definition by ID.
    pub fn get_service(&self, service_id: &str) -> Option<&ServiceDefinition> {
        self.catalog.get(service_id)
    }

    /// List all available services.
    pub fn list_services(&self) -> Vec<&ServiceDefinition> {
        self.catalog.values().collect()
    }

    /// Create a quote for a service request.
    pub fn create_quote(
        &mut self,
        service_id: &str,
        requester: &str,
        params: serde_json::Value,
    ) -> Result<ServiceRequest> {
        let service = self
            .catalog
            .get(service_id)
            .ok_or_else(|| IroncladError::Config(format!("service '{}' not found", service_id)))?;

        self.request_counter += 1;
        let request_id = format!("req_{}", self.request_counter);

        let request = ServiceRequest {
            id: request_id.clone(),
            service_id: service_id.to_string(),
            requester: requester.to_string(),
            parameters: params,
            status: ServiceStatus::Quoted,
            payment_tx: None,
            created_at: Utc::now(),
            completed_at: None,
        };

        info!(
            request_id = %request.id,
            service = %service.name,
            price = service.price_usdc,
            "created service quote"
        );

        self.requests.insert(request_id, request.clone());
        Ok(request)
    }

    /// Record a payment for a request.
    pub fn record_payment(&mut self, request_id: &str, tx_hash: &str) -> Result<()> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Config(format!("request '{}' not found", request_id)))?;

        if request.status != ServiceStatus::Quoted {
            return Err(IroncladError::Config(format!(
                "request '{}' is not in quoted state",
                request_id
            )));
        }

        request.payment_tx = Some(tx_hash.to_string());
        request.status = ServiceStatus::PaymentVerified;
        info!(request_id, tx = tx_hash, "payment recorded");
        Ok(())
    }

    /// Mark a request as in progress.
    pub fn start_fulfillment(&mut self, request_id: &str) -> Result<()> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Config(format!("request '{}' not found", request_id)))?;

        if request.status != ServiceStatus::PaymentVerified {
            return Err(IroncladError::Config(format!(
                "request '{}' payment not verified",
                request_id
            )));
        }

        request.status = ServiceStatus::InProgress;
        debug!(request_id, "fulfillment started");
        Ok(())
    }

    /// Mark a request as completed.
    pub fn complete_fulfillment(&mut self, request_id: &str) -> Result<()> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Config(format!("request '{}' not found", request_id)))?;

        if request.status != ServiceStatus::InProgress {
            return Err(IroncladError::Config(format!(
                "request '{}' is not in progress",
                request_id
            )));
        }

        request.status = ServiceStatus::Completed;
        request.completed_at = Some(Utc::now());
        info!(request_id, "fulfillment completed");
        Ok(())
    }

    /// Mark a request as failed.
    pub fn fail_fulfillment(&mut self, request_id: &str) -> Result<()> {
        let request = self
            .requests
            .get_mut(request_id)
            .ok_or_else(|| IroncladError::Config(format!("request '{}' not found", request_id)))?;

        request.status = ServiceStatus::Failed;
        warn!(request_id, "fulfillment failed");
        Ok(())
    }

    /// Get a request by ID.
    pub fn get_request(&self, request_id: &str) -> Option<&ServiceRequest> {
        self.requests.get(request_id)
    }

    /// List requests by status.
    pub fn requests_by_status(&self, status: ServiceStatus) -> Vec<&ServiceRequest> {
        self.requests
            .values()
            .filter(|r| r.status == status)
            .collect()
    }

    /// Calculate total revenue from completed requests.
    pub fn total_revenue(&self) -> f64 {
        self.requests
            .values()
            .filter(|r| r.status == ServiceStatus::Completed)
            .filter_map(|r| self.catalog.get(&r.service_id))
            .map(|s| s.price_usdc)
            .sum()
    }

    pub fn catalog_size(&self) -> usize {
        self.catalog.len()
    }

    pub fn request_count(&self) -> usize {
        self.requests.len()
    }
}

impl Default for ServiceManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_service() -> ServiceDefinition {
        ServiceDefinition {
            id: "code-review".into(),
            name: "Code Review".into(),
            description: "Automated code review".into(),
            price_usdc: 5.0,
            capabilities_required: vec!["coding".into()],
            max_concurrent: 3,
            estimated_duration_seconds: 300,
        }
    }

    #[test]
    fn register_and_list() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();
        assert_eq!(mgr.catalog_size(), 1);
        assert!(mgr.get_service("code-review").is_some());
    }

    #[test]
    fn reject_empty_id() {
        let mut mgr = ServiceManager::new();
        let mut svc = test_service();
        svc.id = String::new();
        assert!(mgr.register_service(svc).is_err());
    }

    #[test]
    fn reject_negative_price() {
        let mut mgr = ServiceManager::new();
        let mut svc = test_service();
        svc.price_usdc = -1.0;
        assert!(mgr.register_service(svc).is_err());
    }

    #[test]
    fn full_lifecycle() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();

        let quote = mgr
            .create_quote("code-review", "client-1", serde_json::json!({}))
            .unwrap();
        assert_eq!(quote.status, ServiceStatus::Quoted);

        mgr.record_payment(&quote.id, "0xabc123").unwrap();
        assert_eq!(
            mgr.get_request(&quote.id).unwrap().status,
            ServiceStatus::PaymentVerified
        );

        mgr.start_fulfillment(&quote.id).unwrap();
        assert_eq!(
            mgr.get_request(&quote.id).unwrap().status,
            ServiceStatus::InProgress
        );

        mgr.complete_fulfillment(&quote.id).unwrap();
        let req = mgr.get_request(&quote.id).unwrap();
        assert_eq!(req.status, ServiceStatus::Completed);
        assert!(req.completed_at.is_some());
    }

    #[test]
    fn invalid_state_transitions() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();
        let quote = mgr
            .create_quote("code-review", "client", serde_json::json!({}))
            .unwrap();

        assert!(mgr.start_fulfillment(&quote.id).is_err());
        assert!(mgr.complete_fulfillment(&quote.id).is_err());
    }

    #[test]
    fn total_revenue() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();

        for i in 0..3 {
            let q = mgr
                .create_quote("code-review", &format!("client-{i}"), serde_json::json!({}))
                .unwrap();
            mgr.record_payment(&q.id, &format!("tx-{i}")).unwrap();
            mgr.start_fulfillment(&q.id).unwrap();
            if i < 2 {
                mgr.complete_fulfillment(&q.id).unwrap();
            }
        }

        assert!((mgr.total_revenue() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn requests_by_status_filter() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();

        mgr.create_quote("code-review", "a", serde_json::json!({}))
            .unwrap();
        mgr.create_quote("code-review", "b", serde_json::json!({}))
            .unwrap();

        assert_eq!(mgr.requests_by_status(ServiceStatus::Quoted).len(), 2);
        assert_eq!(mgr.requests_by_status(ServiceStatus::Completed).len(), 0);
    }

    #[test]
    fn fail_fulfillment() {
        let mut mgr = ServiceManager::new();
        mgr.register_service(test_service()).unwrap();
        let q = mgr
            .create_quote("code-review", "c", serde_json::json!({}))
            .unwrap();
        mgr.record_payment(&q.id, "tx").unwrap();
        mgr.start_fulfillment(&q.id).unwrap();
        mgr.fail_fulfillment(&q.id).unwrap();
        assert_eq!(
            mgr.get_request(&q.id).unwrap().status,
            ServiceStatus::Failed
        );
    }

    #[test]
    fn service_status_serde() {
        for status in [
            ServiceStatus::Quoted,
            ServiceStatus::PaymentPending,
            ServiceStatus::PaymentVerified,
            ServiceStatus::InProgress,
            ServiceStatus::Completed,
            ServiceStatus::Failed,
            ServiceStatus::Refunded,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: ServiceStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }
}
