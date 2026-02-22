use std::collections::HashMap;
use std::fmt;

use async_trait::async_trait;
use serde_json::Value;

use ironclad_core::{InputAuthority, RiskLevel};

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn risk_level(&self) -> RiskLevel;
    fn parameters_schema(&self) -> Value;
    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError>;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub agent_id: String,
    pub authority: InputAuthority,
}

#[derive(Debug, Clone)]
pub struct ToolResult {
    pub output: String,
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone)]
pub struct ToolError {
    pub message: String,
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ToolError: {}", self.message)
    }
}

impl std::error::Error for ToolError {}

pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EchoTool;

#[async_trait]
impl Tool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }

    fn description(&self) -> &str {
        "Echoes input back as output"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": { "type": "string" }
            },
            "required": ["message"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'message' parameter".into(),
            })?;

        Ok(ToolResult {
            output: message.to_string(),
            metadata: None,
        })
    }
}

/// Tool wrapper around `ScriptRunner` for executing skill scripts via the ToolRegistry.
pub struct ScriptRunnerTool {
    runner: crate::script_runner::ScriptRunner,
}

impl ScriptRunnerTool {
    pub fn new(config: ironclad_core::config::SkillsConfig) -> Self {
        Self {
            runner: crate::script_runner::ScriptRunner::new(config),
        }
    }
}

#[async_trait]
impl Tool for ScriptRunnerTool {
    fn name(&self) -> &str {
        "run_script"
    }

    fn description(&self) -> &str {
        "Execute a whitelisted skill script with sandboxed environment"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "description": "Path to the script file" },
                "args": { "type": "array", "items": { "type": "string" }, "description": "Arguments to pass" }
            },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let path = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'path' parameter".into(),
            })?;

        let args: Vec<String> = params
            .get("args")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        let script_path = std::path::Path::new(path);

        match self.runner.execute(script_path, &arg_refs).await {
            Ok(result) => {
                let output = if result.exit_code == 0 {
                    result.stdout
                } else {
                    format!("exit code {}: {}", result.exit_code, result.stderr)
                };
                Ok(ToolResult {
                    output,
                    metadata: Some(serde_json::json!({
                        "exit_code": result.exit_code,
                        "duration_ms": result.duration_ms,
                    })),
                })
            }
            Err(e) => Err(ToolError {
                message: e.to_string(),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> ToolContext {
        ToolContext {
            session_id: "test-session".into(),
            agent_id: "test-agent".into(),
            authority: InputAuthority::Creator,
        }
    }

    #[test]
    fn register_and_retrieve() {
        let mut registry = ToolRegistry::new();
        registry.register(Box::new(EchoTool));

        let tool = registry.get("echo");
        assert!(tool.is_some());
        assert_eq!(tool.unwrap().name(), "echo");
        assert_eq!(tool.unwrap().risk_level(), RiskLevel::Safe);

        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn list_tools() {
        let mut registry = ToolRegistry::new();
        assert!(registry.list().is_empty());

        registry.register(Box::new(EchoTool));
        let tools = registry.list();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name(), "echo");
    }

    #[tokio::test]
    async fn echo_tool_execution() {
        let tool = EchoTool;
        let ctx = test_ctx();
        let params = serde_json::json!({ "message": "hello world" });

        let result = tool.execute(params, &ctx).await.unwrap();
        assert_eq!(result.output, "hello world");
        assert!(result.metadata.is_none());

        let bad_params = serde_json::json!({});
        let err = tool.execute(bad_params, &ctx).await.unwrap_err();
        assert!(err.message.contains("missing"));
    }
}
