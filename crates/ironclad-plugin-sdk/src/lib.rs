//! # ironclad-plugin-sdk
//!
//! Plugin system for the Ironclad agent runtime. Plugins extend the agent with
//! custom tools that are discovered, loaded, and executed through a unified
//! async trait interface.
//!
//! ## Key Types
//!
//! - [`Plugin`] -- Async trait defining the plugin lifecycle
//! - [`ToolDef`] -- Tool definition with name, description, and JSON Schema parameters
//! - [`ToolResult`] -- Execution result (success flag, output text, optional metadata)
//! - [`PluginStatus`] -- Plugin state: Loaded, Active, Disabled, Error
//!
//! ## Modules
//!
//! - `loader` -- Load plugins from a directory with auto-discovery and hot-reload
//! - `manifest` -- TOML manifest parsing and validation
//! - `registry` -- Plugin registration, lookup, enable/disable
//! - `script` -- Script-based plugin execution (subprocess with sandboxing)

pub mod loader;
pub mod manifest;
pub mod registry;
pub mod script;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use ironclad_core::Result;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    pub success: bool,
    pub output: String,
    pub metadata: Option<Value>,
}

#[async_trait]
pub trait Plugin: Send + Sync {
    fn name(&self) -> &str;
    fn version(&self) -> &str;
    fn tools(&self) -> Vec<ToolDef>;
    async fn init(&mut self) -> Result<()>;
    async fn execute_tool(&self, tool_name: &str, input: &Value) -> Result<ToolResult>;
    async fn shutdown(&mut self) -> Result<()>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginStatus {
    Loaded,
    Active,
    Disabled,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_def_serde() {
        let tool = ToolDef {
            name: "test_tool".into(),
            description: "A test tool".into(),
            parameters: serde_json::json!({"type": "object"}),
        };
        let json = serde_json::to_string(&tool).unwrap();
        let back: ToolDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "test_tool");
    }

    #[test]
    fn tool_result_serde() {
        let result = ToolResult {
            success: true,
            output: "done".into(),
            metadata: Some(serde_json::json!({"elapsed_ms": 42})),
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: ToolResult = serde_json::from_str(&json).unwrap();
        assert!(back.success);
        assert_eq!(back.output, "done");
    }

    #[test]
    fn plugin_status_roundtrip() {
        for status in [
            PluginStatus::Loaded,
            PluginStatus::Active,
            PluginStatus::Disabled,
            PluginStatus::Error,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: PluginStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(status, back);
        }
    }
}
