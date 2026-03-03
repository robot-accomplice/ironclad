use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use ironclad_core::{IroncladError, Result};

use crate::{Plugin, PluginStatus, ToolDef, ToolResult};

/// Controls how undeclared plugin permissions are handled at runtime.
pub struct PermissionPolicy {
    pub strict: bool,
    pub allowed: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub status: PluginStatus,
    pub tools: Vec<ToolDef>,
}

struct PluginEntry {
    plugin: Arc<tokio::sync::Mutex<Box<dyn Plugin>>>,
    status: PluginStatus,
}

/// Central registry that owns all loaded plugin instances.
///
/// ## Lock acquisition pattern
///
/// This registry uses a two-level locking scheme:
///
/// 1. **Outer lock** (`self.plugins`): A `tokio::sync::Mutex<HashMap<...>>` that
///    guards the plugin map itself (registration, removal, iteration).
/// 2. **Inner lock** (each `PluginEntry::plugin`): A per-plugin
///    `Arc<tokio::sync::Mutex<Box<dyn Plugin>>>` that guards access to individual
///    plugin instances.
///
/// Several methods (e.g., `execute_tool`, `find_tool`, `list_plugins`,
/// `list_all_tools`) acquire the outer lock and then, while still holding it,
/// acquire one or more inner plugin locks. This nested acquisition is safe from
/// deadlocks because the inner locks are never held when attempting to acquire
/// the outer lock. However, it means that a slow plugin `init()` or
/// `execute_tool()` call can block all other registry operations for the
/// duration.
///
/// `tools()` on the `Plugin` trait is expected to be non-blocking (it returns a
/// `Vec<ToolDef>` synchronously) so the inner lock contention during tool
/// lookups should be negligible. If plugin execution latency becomes a concern,
/// consider cloning the `Arc` outside the outer lock and releasing the outer
/// lock before awaiting the inner one (as `execute_tool` already does).
pub struct PluginRegistry {
    plugins: Mutex<HashMap<String, PluginEntry>>,
    allow_list: Vec<String>,
    deny_list: Vec<String>,
    permission_policy: PermissionPolicy,
}

impl PluginRegistry {
    pub fn new(
        allow_list: Vec<String>,
        deny_list: Vec<String>,
        permission_policy: PermissionPolicy,
    ) -> Self {
        let normalized_allowed: Vec<String> = permission_policy
            .allowed
            .into_iter()
            .map(|p| p.to_ascii_lowercase())
            .collect();
        Self {
            plugins: Mutex::new(HashMap::new()),
            allow_list,
            deny_list,
            permission_policy: PermissionPolicy {
                strict: permission_policy.strict,
                allowed: normalized_allowed,
            },
        }
    }

    pub fn is_allowed(&self, name: &str) -> bool {
        if self.deny_list.iter().any(|d| d == name) {
            return false;
        }
        if self.allow_list.is_empty() {
            return true;
        }
        self.allow_list.iter().any(|a| a == name)
    }

    pub async fn register(&self, plugin: Box<dyn Plugin>) -> Result<()> {
        let name = plugin.name().to_string();

        if !self.is_allowed(&name) {
            return Err(IroncladError::Config(format!(
                "plugin '{name}' is not allowed by policy"
            )));
        }

        if self.permission_policy.strict {
            for tool in plugin.tools() {
                for perm in tool.permissions {
                    let normalized = perm.to_ascii_lowercase();
                    if !self
                        .permission_policy
                        .allowed
                        .iter()
                        .any(|p| p == &normalized)
                    {
                        return Err(IroncladError::Config(format!(
                            "plugin '{name}' tool '{}' declares permission '{perm}' not in allowed_permissions",
                            tool.name
                        )));
                    }
                }
            }
        }

        debug!(name = %name, version = %plugin.version(), "registering plugin");

        let entry = PluginEntry {
            plugin: Arc::new(tokio::sync::Mutex::new(plugin)),
            status: PluginStatus::Loaded,
        };

        let mut plugins = self.plugins.lock().await;
        plugins.insert(name, entry);
        Ok(())
    }

    pub async fn init_all(&self) -> Vec<String> {
        let mut errors = Vec::new();
        let mut plugins = self.plugins.lock().await;

        for (name, entry) in plugins.iter_mut() {
            let mut plugin = entry.plugin.lock().await;
            match plugin.init().await {
                Ok(()) => {
                    entry.status = PluginStatus::Active;
                    debug!(name = %name, "plugin initialized");
                }
                Err(e) => {
                    entry.status = PluginStatus::Error;
                    warn!(name = %name, error = %e, "plugin init failed");
                    errors.push(format!("{name}: {e}"));
                }
            }
        }

        errors
    }

    pub async fn execute_tool(&self, tool_name: &str, input: &Value) -> Result<ToolResult> {
        let plugin_arc = {
            let plugins = self.plugins.lock().await;
            let mut found = None;
            for entry in plugins.values() {
                if entry.status != PluginStatus::Active {
                    continue;
                }
                let p = entry.plugin.lock().await;
                if p.tools().iter().any(|t| t.name == tool_name) {
                    drop(p);
                    found = Some(Arc::clone(&entry.plugin));
                    break;
                }
            }
            found
        };

        let plugin_arc = match plugin_arc {
            Some(p) => p,
            None => {
                return Err(IroncladError::Tool {
                    tool: tool_name.to_string(),
                    message: "no plugin provides this tool".into(),
                });
            }
        };

        // Check permission policy before executing.
        let plugin = plugin_arc.lock().await;
        let tool_permissions: Vec<String> = plugin
            .tools()
            .iter()
            .find(|t| t.name == tool_name)
            .map(|t| t.permissions.clone())
            .unwrap_or_default();

        for perm in &tool_permissions {
            let normalized = perm.to_ascii_lowercase();
            if !self
                .permission_policy
                .allowed
                .iter()
                .any(|p| p == &normalized)
            {
                if self.permission_policy.strict {
                    return Err(IroncladError::Tool {
                        tool: tool_name.to_string(),
                        message: format!(
                            "permission '{perm}' is not allowed by policy (strict mode)"
                        ),
                    });
                }
                warn!(
                    tool = %tool_name,
                    permission = %perm,
                    "tool requires permission not in allowed list (permissive mode)"
                );
            }
        }

        plugin.execute_tool(tool_name, input).await
    }

    pub async fn shutdown_all(&self) {
        let mut plugins = self.plugins.lock().await;
        for (name, entry) in plugins.iter_mut() {
            let mut plugin = entry.plugin.lock().await;
            if let Err(e) = plugin.shutdown().await {
                warn!(name = %name, error = %e, "plugin shutdown failed");
            }
            entry.status = PluginStatus::Disabled;
        }
    }

    pub async fn find_tool(&self, tool_name: &str) -> Option<(String, ToolDef)> {
        let plugins = self.plugins.lock().await;
        for (plugin_name, entry) in plugins.iter() {
            if entry.status != PluginStatus::Active {
                continue;
            }
            let plugin = entry.plugin.lock().await;
            for tool in plugin.tools() {
                if tool.name == tool_name {
                    return Some((plugin_name.clone(), tool));
                }
            }
        }
        None
    }

    pub async fn list_plugins(&self) -> Vec<PluginInfo> {
        let plugins = self.plugins.lock().await;
        let mut result = Vec::new();
        for entry in plugins.values() {
            let plugin = entry.plugin.lock().await;
            result.push(PluginInfo {
                name: plugin.name().to_string(),
                version: plugin.version().to_string(),
                status: entry.status,
                tools: plugin.tools(),
            });
        }
        result
    }

    pub async fn list_all_tools(&self) -> Vec<(String, ToolDef)> {
        let plugins = self.plugins.lock().await;
        let mut tools = Vec::new();
        for (name, entry) in plugins.iter() {
            if entry.status != PluginStatus::Active {
                continue;
            }
            let plugin = entry.plugin.lock().await;
            for tool in plugin.tools() {
                tools.push((name.clone(), tool));
            }
        }
        tools
    }

    pub async fn disable_plugin(&self, name: &str) -> Result<()> {
        let mut plugins = self.plugins.lock().await;
        let entry = plugins
            .get_mut(name)
            .ok_or_else(|| IroncladError::Config(format!("plugin '{name}' not found")))?;
        entry.status = PluginStatus::Disabled;
        debug!(name, "plugin disabled");
        Ok(())
    }

    pub async fn enable_plugin(&self, name: &str) -> Result<()> {
        let mut plugins = self.plugins.lock().await;
        let entry = plugins
            .get_mut(name)
            .ok_or_else(|| IroncladError::Config(format!("plugin '{name}' not found")))?;
        entry.status = PluginStatus::Active;
        debug!(name, "plugin enabled");
        Ok(())
    }

    /// Removes a plugin from the registry entirely, shutting it down first.
    ///
    /// Unlike `disable_plugin` (which keeps the entry around so it can be
    /// re-enabled), `unregister` drops the plugin and frees all associated
    /// resources. This should be used when a plugin is permanently removed
    /// (e.g., uninstalled or revoked by policy).
    pub async fn unregister(&self, name: &str) -> Result<()> {
        let mut plugins = self.plugins.lock().await;
        let entry = plugins
            .remove(name)
            .ok_or_else(|| IroncladError::Config(format!("plugin '{name}' not found")))?;
        // Best-effort shutdown -- log but do not propagate errors.
        let mut plugin = entry.plugin.lock().await;
        if let Err(e) = plugin.shutdown().await {
            warn!(name, error = %e, "plugin shutdown failed during unregister");
        }
        debug!(name, "plugin unregistered");
        Ok(())
    }

    pub async fn plugin_count(&self) -> usize {
        let plugins = self.plugins.lock().await;
        plugins.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    struct MockPlugin {
        name: String,
        init_fail: bool,
    }

    impl MockPlugin {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
                init_fail: false,
            }
        }
        fn failing(name: &str) -> Self {
            Self {
                name: name.into(),
                init_fail: true,
            }
        }
    }

    #[async_trait]
    impl Plugin for MockPlugin {
        fn name(&self) -> &str {
            &self.name
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: format!("{}_tool", self.name),
                description: "mock tool".into(),
                parameters: serde_json::json!({}),
                risk_level: ironclad_core::RiskLevel::Safe,
                permissions: vec![],
            }]
        }
        async fn init(&mut self) -> Result<()> {
            if self.init_fail {
                Err(IroncladError::Config("init failed".into()))
            } else {
                Ok(())
            }
        }
        async fn execute_tool(&self, tool_name: &str, _input: &Value) -> Result<ToolResult> {
            Ok(ToolResult {
                success: true,
                output: format!("executed {tool_name}"),
                metadata: None,
            })
        }
        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn allow_deny_lists() {
        let reg = PluginRegistry::new(
            vec![],
            vec!["blocked".into()],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        assert!(reg.is_allowed("anything"));
        assert!(!reg.is_allowed("blocked"));

        let reg2 = PluginRegistry::new(
            vec!["only_this".into()],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        assert!(reg2.is_allowed("only_this"));
        assert!(!reg2.is_allowed("other"));
    }

    #[tokio::test]
    async fn register_and_list() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("test")))
            .await
            .unwrap();
        assert_eq!(reg.plugin_count().await, 1);
        let plugins = reg.list_plugins().await;
        assert_eq!(plugins[0].name, "test");
        assert_eq!(plugins[0].status, PluginStatus::Loaded);
    }

    #[tokio::test]
    async fn register_denied_fails() {
        let reg = PluginRegistry::new(
            vec![],
            vec!["bad".into()],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        let result = reg.register(Box::new(MockPlugin::new("bad"))).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn init_all_activates() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("p1"))).await.unwrap();
        let errors = reg.init_all().await;
        assert!(errors.is_empty());
        let plugins = reg.list_plugins().await;
        assert_eq!(plugins[0].status, PluginStatus::Active);
    }

    #[tokio::test]
    async fn init_failure_marks_error() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::failing("bad")))
            .await
            .unwrap();
        let errors = reg.init_all().await;
        assert_eq!(errors.len(), 1);
        let plugins = reg.list_plugins().await;
        assert_eq!(plugins[0].status, PluginStatus::Error);
    }

    #[tokio::test]
    async fn execute_tool_found() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("p1"))).await.unwrap();
        reg.init_all().await;
        let result = reg
            .execute_tool("p1_tool", &serde_json::json!({}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("p1_tool"));
    }

    #[tokio::test]
    async fn execute_tool_not_found() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        let result = reg
            .execute_tool("nonexistent", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_tool() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("alpha")))
            .await
            .unwrap();
        reg.init_all().await;
        let found = reg.find_tool("alpha_tool").await;
        assert!(found.is_some());
        let (plugin_name, tool) = found.unwrap();
        assert_eq!(plugin_name, "alpha");
        assert_eq!(tool.name, "alpha_tool");
    }

    #[tokio::test]
    async fn disable_enable_plugin() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("p"))).await.unwrap();
        reg.init_all().await;

        reg.disable_plugin("p").await.unwrap();
        let plugins = reg.list_plugins().await;
        assert_eq!(plugins[0].status, PluginStatus::Disabled);

        let result = reg.execute_tool("p_tool", &serde_json::json!({})).await;
        assert!(result.is_err());

        reg.enable_plugin("p").await.unwrap();
        let result = reg.execute_tool("p_tool", &serde_json::json!({})).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn unregister_removes_plugin() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("removable")))
            .await
            .unwrap();
        assert_eq!(reg.plugin_count().await, 1);

        reg.unregister("removable").await.unwrap();
        assert_eq!(reg.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn unregister_nonexistent_fails() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        let result = reg.unregister("ghost").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn unregister_makes_tool_unavailable() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("p1"))).await.unwrap();
        reg.init_all().await;

        // Tool should be available before unregister.
        assert!(reg.find_tool("p1_tool").await.is_some());

        reg.unregister("p1").await.unwrap();

        // Tool should be gone after unregister.
        assert!(reg.find_tool("p1_tool").await.is_none());
        let result = reg.execute_tool("p1_tool", &serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn list_all_tools() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(MockPlugin::new("a"))).await.unwrap();
        reg.register(Box::new(MockPlugin::new("b"))).await.unwrap();
        reg.init_all().await;
        let tools = reg.list_all_tools().await;
        assert_eq!(tools.len(), 2);
    }

    /// A mock plugin whose tool declares specific permissions.
    struct PermissionMockPlugin {
        name: String,
        permissions: Vec<String>,
    }

    impl PermissionMockPlugin {
        fn new(name: &str, permissions: Vec<String>) -> Self {
            Self {
                name: name.into(),
                permissions,
            }
        }
    }

    #[async_trait]
    impl Plugin for PermissionMockPlugin {
        fn name(&self) -> &str {
            &self.name
        }
        fn version(&self) -> &str {
            "1.0.0"
        }
        fn tools(&self) -> Vec<ToolDef> {
            vec![ToolDef {
                name: format!("{}_tool", self.name),
                description: "mock tool with permissions".into(),
                parameters: serde_json::json!({}),
                risk_level: ironclad_core::RiskLevel::Safe,
                permissions: self.permissions.clone(),
            }]
        }
        async fn init(&mut self) -> Result<()> {
            Ok(())
        }
        async fn execute_tool(&self, tool_name: &str, _input: &Value) -> Result<ToolResult> {
            Ok(ToolResult {
                success: true,
                output: format!("executed {tool_name}"),
                metadata: None,
            })
        }
        async fn shutdown(&mut self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn strict_mode_blocks_unauthorized_plugin() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: true,
                allowed: vec![],
            },
        );
        // Strict mode now rejects unauthorized plugins at registration time (fail-fast)
        let result = reg
            .register(Box::new(PermissionMockPlugin::new(
                "net",
                vec!["network".into()],
            )))
            .await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("permission"));
    }

    #[tokio::test]
    async fn permissive_mode_allows_unauthorized_plugin() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: false,
                allowed: vec![],
            },
        );
        reg.register(Box::new(PermissionMockPlugin::new(
            "net",
            vec!["network".into()],
        )))
        .await
        .unwrap();
        reg.init_all().await;

        let result = reg.execute_tool("net_tool", &serde_json::json!({})).await;
        assert!(result.is_ok());
        assert!(result.unwrap().success);
    }

    #[tokio::test]
    async fn allowed_permissions_pass_strict_check() {
        let reg = PluginRegistry::new(
            vec![],
            vec![],
            PermissionPolicy {
                strict: true,
                allowed: vec!["network".into()],
            },
        );
        reg.register(Box::new(PermissionMockPlugin::new(
            "net",
            vec!["network".into()],
        )))
        .await
        .unwrap();
        reg.init_all().await;

        let result = reg.execute_tool("net_tool", &serde_json::json!({})).await;
        assert!(result.is_ok());
        assert!(result.unwrap().success);
    }
}
