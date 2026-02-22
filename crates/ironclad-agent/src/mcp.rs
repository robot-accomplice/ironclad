use ironclad_core::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// MCP tool descriptor exposed to clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolDescriptor {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

/// MCP resource descriptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub mime_type: String,
}

/// Registry for MCP-exposed tools and resources.
#[derive(Debug, Default)]
pub struct McpServerRegistry {
    tools: HashMap<String, McpToolDescriptor>,
    resources: HashMap<String, McpResource>,
}

impl McpServerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_tool(&mut self, descriptor: McpToolDescriptor) {
        debug!(name = %descriptor.name, "registered MCP tool");
        self.tools.insert(descriptor.name.clone(), descriptor);
    }

    pub fn register_resource(&mut self, resource: McpResource) {
        debug!(uri = %resource.uri, "registered MCP resource");
        self.resources.insert(resource.uri.clone(), resource);
    }

    pub fn list_tools(&self) -> Vec<&McpToolDescriptor> {
        self.tools.values().collect()
    }

    pub fn get_tool(&self, name: &str) -> Option<&McpToolDescriptor> {
        self.tools.get(name)
    }

    pub fn list_resources(&self) -> Vec<&McpResource> {
        self.resources.values().collect()
    }

    pub fn get_resource(&self, uri: &str) -> Option<&McpResource> {
        self.resources.get(uri)
    }

    pub fn tool_count(&self) -> usize {
        self.tools.len()
    }

    pub fn resource_count(&self) -> usize {
        self.resources.len()
    }
}

/// Represents a connection to an external MCP server.
#[derive(Debug, Clone)]
pub struct McpClientConnection {
    pub name: String,
    pub url: String,
    pub available_tools: Vec<McpToolDescriptor>,
    pub available_resources: Vec<McpResource>,
    pub connected: bool,
}

impl McpClientConnection {
    pub fn new(name: String, url: String) -> Self {
        Self {
            name,
            url,
            available_tools: Vec::new(),
            available_resources: Vec::new(),
            connected: false,
        }
    }

    /// Discover tools from the remote MCP server.
    /// In production, this would make HTTP/SSE requests.
    pub fn discover(&mut self) -> Result<()> {
        info!(name = %self.name, url = %self.url, "MCP client discovering tools");
        self.connected = true;
        Ok(())
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    pub fn disconnect(&mut self) {
        self.connected = false;
        self.available_tools.clear();
        self.available_resources.clear();
        debug!(name = %self.name, "MCP client disconnected");
    }
}

/// Manages multiple MCP client connections.
#[derive(Debug, Default)]
pub struct McpClientManager {
    connections: HashMap<String, McpClientConnection>,
}

impl McpClientManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_connection(&mut self, conn: McpClientConnection) {
        self.connections.insert(conn.name.clone(), conn);
    }

    pub fn get_connection(&self, name: &str) -> Option<&McpClientConnection> {
        self.connections.get(name)
    }

    pub fn get_connection_mut(&mut self, name: &str) -> Option<&mut McpClientConnection> {
        self.connections.get_mut(name)
    }

    pub fn list_connections(&self) -> Vec<&McpClientConnection> {
        self.connections.values().collect()
    }

    pub fn connected_count(&self) -> usize {
        self.connections.values().filter(|c| c.connected).count()
    }

    pub fn total_count(&self) -> usize {
        self.connections.len()
    }

    /// Get all tools available across all connected MCP servers.
    pub fn all_available_tools(&self) -> Vec<(&str, &McpToolDescriptor)> {
        self.connections
            .values()
            .filter(|c| c.connected)
            .flat_map(|c| c.available_tools.iter().map(move |t| (c.name.as_str(), t)))
            .collect()
    }

    pub fn disconnect_all(&mut self) {
        for conn in self.connections.values_mut() {
            conn.disconnect();
        }
    }
}

/// Build MCP tool descriptors from the internal tool registry.
pub fn export_tools_as_mcp(
    tools: &[(String, String, serde_json::Value)],
) -> Vec<McpToolDescriptor> {
    tools
        .iter()
        .map(|(name, desc, schema)| McpToolDescriptor {
            name: name.clone(),
            description: desc.clone(),
            input_schema: schema.clone(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_tool() -> McpToolDescriptor {
        McpToolDescriptor {
            name: "memory_search".to_string(),
            description: "Search agent memory".to_string(),
            input_schema: serde_json::json!({"type": "object", "properties": {"query": {"type": "string"}}}),
        }
    }

    fn sample_resource() -> McpResource {
        McpResource {
            uri: "ironclad://sessions/current".to_string(),
            name: "Current Session".to_string(),
            description: "The active session context".to_string(),
            mime_type: "application/json".to_string(),
        }
    }

    #[test]
    fn server_registry_tools() {
        let mut reg = McpServerRegistry::new();
        reg.register_tool(sample_tool());
        assert_eq!(reg.tool_count(), 1);
        assert!(reg.get_tool("memory_search").is_some());
        assert!(reg.get_tool("nonexistent").is_none());
    }

    #[test]
    fn server_registry_resources() {
        let mut reg = McpServerRegistry::new();
        reg.register_resource(sample_resource());
        assert_eq!(reg.resource_count(), 1);
        assert!(reg.get_resource("ironclad://sessions/current").is_some());
    }

    #[test]
    fn server_registry_list() {
        let mut reg = McpServerRegistry::new();
        reg.register_tool(sample_tool());
        reg.register_resource(sample_resource());
        assert_eq!(reg.list_tools().len(), 1);
        assert_eq!(reg.list_resources().len(), 1);
    }

    #[test]
    fn client_connection_lifecycle() {
        let mut conn =
            McpClientConnection::new("test-server".into(), "http://localhost:8080".into());
        assert!(!conn.is_connected());

        conn.discover().unwrap();
        assert!(conn.is_connected());

        conn.disconnect();
        assert!(!conn.is_connected());
    }

    #[test]
    fn client_manager_basic() {
        let mut mgr = McpClientManager::new();
        let conn = McpClientConnection::new("server-a".into(), "http://a.example.com".into());
        mgr.add_connection(conn);

        assert_eq!(mgr.total_count(), 1);
        assert_eq!(mgr.connected_count(), 0);
        assert!(mgr.get_connection("server-a").is_some());
    }

    #[test]
    fn client_manager_discover() {
        let mut mgr = McpClientManager::new();
        mgr.add_connection(McpClientConnection::new(
            "s1".into(),
            "http://s1.local".into(),
        ));
        mgr.add_connection(McpClientConnection::new(
            "s2".into(),
            "http://s2.local".into(),
        ));

        mgr.get_connection_mut("s1").unwrap().discover().unwrap();
        assert_eq!(mgr.connected_count(), 1);

        mgr.get_connection_mut("s2").unwrap().discover().unwrap();
        assert_eq!(mgr.connected_count(), 2);
    }

    #[test]
    fn client_manager_disconnect_all() {
        let mut mgr = McpClientManager::new();
        let mut c1 = McpClientConnection::new("a".into(), "http://a".into());
        c1.discover().unwrap();
        mgr.add_connection(c1);

        let mut c2 = McpClientConnection::new("b".into(), "http://b".into());
        c2.discover().unwrap();
        mgr.add_connection(c2);

        assert_eq!(mgr.connected_count(), 2);
        mgr.disconnect_all();
        assert_eq!(mgr.connected_count(), 0);
    }

    #[test]
    fn export_tools_as_mcp_conversion() {
        let tools = vec![
            (
                "tool_a".to_string(),
                "Description A".to_string(),
                serde_json::json!({}),
            ),
            (
                "tool_b".to_string(),
                "Description B".to_string(),
                serde_json::json!({"type": "object"}),
            ),
        ];
        let mcp_tools = export_tools_as_mcp(&tools);
        assert_eq!(mcp_tools.len(), 2);
        assert_eq!(mcp_tools[0].name, "tool_a");
        assert_eq!(mcp_tools[1].description, "Description B");
    }

    #[test]
    fn all_available_tools_across_connections() {
        let mut mgr = McpClientManager::new();
        let mut c1 = McpClientConnection::new("s1".into(), "http://s1".into());
        c1.discover().unwrap();
        c1.available_tools.push(sample_tool());
        mgr.add_connection(c1);

        let c2 = McpClientConnection::new("s2".into(), "http://s2".into());
        mgr.add_connection(c2);

        let all_tools = mgr.all_available_tools();
        assert_eq!(all_tools.len(), 1);
        assert_eq!(all_tools[0].0, "s1");
    }

    #[test]
    fn mcp_transport_default() {
        let transport = ironclad_core::config::McpTransport::default();
        matches!(transport, ironclad_core::config::McpTransport::Sse);
    }

    #[test]
    fn tool_descriptor_serde() {
        let tool = sample_tool();
        let json = serde_json::to_string(&tool).unwrap();
        let back: McpToolDescriptor = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, tool.name);
    }

    #[test]
    fn resource_serde() {
        let res = sample_resource();
        let json = serde_json::to_string(&res).unwrap();
        let back: McpResource = serde_json::from_str(&json).unwrap();
        assert_eq!(back.uri, res.uri);
    }
}
