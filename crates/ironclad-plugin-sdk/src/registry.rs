use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Mutex;
use tracing::{debug, warn};

use ironclad_core::{IroncladError, Result};

use crate::{Plugin, PluginStatus, ToolDef, ToolResult};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginInfo {
    pub name: String,
    pub version: String,
    pub status: PluginStatus,
    pub tools: Vec<ToolDef>,
}

struct PluginEntry {
    plugin: Box<dyn Plugin>,
    status: PluginStatus,
}

pub struct PluginRegistry {
    plugins: Mutex<HashMap<String, PluginEntry>>,
    allow_list: Vec<String>,
    deny_list: Vec<String>,
}

impl PluginRegistry {
    pub fn new(allow_list: Vec<String>, deny_list: Vec<String>) -> Self {
        Self {
            plugins: Mutex::new(HashMap::new()),
            allow_list,
            deny_list,
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

        debug!(name = %name, version = %plugin.version(), "registering plugin");

        let entry = PluginEntry {
            plugin,
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
            match entry.plugin.init().await {
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
        let plugins = self.plugins.lock().await;

        for (_, entry) in plugins.iter() {
            if entry.status != PluginStatus::Active {
                continue;
            }
            let tools = entry.plugin.tools();
            if tools.iter().any(|t| t.name == tool_name) {
                return entry.plugin.execute_tool(tool_name, input).await;
            }
        }

        Err(IroncladError::Tool {
            tool: tool_name.to_string(),
            message: "no plugin provides this tool".into(),
        })
    }

    pub async fn find_tool(&self, tool_name: &str) -> Option<(String, ToolDef)> {
        let plugins = self.plugins.lock().await;
        for (plugin_name, entry) in plugins.iter() {
            if entry.status != PluginStatus::Active {
                continue;
            }
            for tool in entry.plugin.tools() {
                if tool.name == tool_name {
                    return Some((plugin_name.clone(), tool));
                }
            }
        }
        None
    }

    pub async fn list_plugins(&self) -> Vec<PluginInfo> {
        let plugins = self.plugins.lock().await;
        plugins
            .values()
            .map(|entry| PluginInfo {
                name: entry.plugin.name().to_string(),
                version: entry.plugin.version().to_string(),
                status: entry.status,
                tools: entry.plugin.tools(),
            })
            .collect()
    }

    pub async fn list_all_tools(&self) -> Vec<(String, ToolDef)> {
        let plugins = self.plugins.lock().await;
        let mut tools = Vec::new();
        for (name, entry) in plugins.iter() {
            if entry.status != PluginStatus::Active {
                continue;
            }
            for tool in entry.plugin.tools() {
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
        let reg = PluginRegistry::new(vec![], vec!["blocked".into()]);
        assert!(reg.is_allowed("anything"));
        assert!(!reg.is_allowed("blocked"));

        let reg2 = PluginRegistry::new(vec!["only_this".into()], vec![]);
        assert!(reg2.is_allowed("only_this"));
        assert!(!reg2.is_allowed("other"));
    }

    #[tokio::test]
    async fn register_and_list() {
        let reg = PluginRegistry::new(vec![], vec![]);
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
        let reg = PluginRegistry::new(vec![], vec!["bad".into()]);
        let result = reg.register(Box::new(MockPlugin::new("bad"))).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn init_all_activates() {
        let reg = PluginRegistry::new(vec![], vec![]);
        reg.register(Box::new(MockPlugin::new("p1"))).await.unwrap();
        let errors = reg.init_all().await;
        assert!(errors.is_empty());
        let plugins = reg.list_plugins().await;
        assert_eq!(plugins[0].status, PluginStatus::Active);
    }

    #[tokio::test]
    async fn init_failure_marks_error() {
        let reg = PluginRegistry::new(vec![], vec![]);
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
        let reg = PluginRegistry::new(vec![], vec![]);
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
        let reg = PluginRegistry::new(vec![], vec![]);
        let result = reg
            .execute_tool("nonexistent", &serde_json::json!({}))
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn find_tool() {
        let reg = PluginRegistry::new(vec![], vec![]);
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
        let reg = PluginRegistry::new(vec![], vec![]);
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
    async fn list_all_tools() {
        let reg = PluginRegistry::new(vec![], vec![]);
        reg.register(Box::new(MockPlugin::new("a"))).await.unwrap();
        reg.register(Box::new(MockPlugin::new("b"))).await.unwrap();
        reg.init_all().await;
        let tools = reg.list_all_tools().await;
        assert_eq!(tools.len(), 2);
    }
}
