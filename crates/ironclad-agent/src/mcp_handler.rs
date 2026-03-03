//! MCP Server Handler — bridges rmcp's `ServerHandler` to Ironclad's `ToolRegistry`.
//!
//! This module implements the server half of the MCP gateway. External MCP clients
//! (Claude Desktop, Cursor, VS Code, etc.) connect via SSE/HTTP, discover Ironclad's
//! tools through `tools/list`, and invoke them through `tools/call`.
//!
//! All tool calls from MCP clients run with `InputAuthority::External` and pass
//! through the policy engine — Forbidden tools are never exposed, and Dangerous
//! tools require explicit configuration.

use std::borrow::Cow;
use std::sync::Arc;

use rmcp::handler::server::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResult, Content, JsonObject, ListToolsResult,
    PaginatedRequestParams, ServerCapabilities, ServerInfo, Tool as McpTool,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData as McpError, RoleServer};
use serde_json::Value;
use tracing::{debug, info, warn};

use ironclad_core::{InputAuthority, RiskLevel};

use crate::tools::{ToolContext, ToolRegistry};

// ─── Public types ──────────────────────────────────────────────────────────

/// Bridges Ironclad's `ToolRegistry` to the MCP protocol so external clients
/// can discover and invoke tools over SSE/HTTP.
#[derive(Clone)]
pub struct IroncladMcpHandler {
    tool_registry: Arc<ToolRegistry>,
    /// Optional allow-list. When `Some`, only these tool names are exposed.
    /// When `None`, all non-Forbidden tools are exposed.
    allow_list: Option<Vec<String>>,
    default_context: McpToolContext,
}

/// Minimal context needed to construct a `ToolContext` for MCP-originated calls.
#[derive(Clone)]
pub struct McpToolContext {
    pub agent_id: String,
    pub workspace_root: std::path::PathBuf,
    pub db: Option<ironclad_db::Database>,
}

// ─── Construction ──────────────────────────────────────────────────────────

impl IroncladMcpHandler {
    pub fn new(tool_registry: Arc<ToolRegistry>, default_context: McpToolContext) -> Self {
        Self {
            tool_registry,
            allow_list: None,
            default_context,
        }
    }

    /// Restrict which tools are visible over MCP. Only names in this list
    /// will appear in `tools/list` (Forbidden tools are *always* hidden
    /// regardless of this list).
    pub fn with_allow_list(mut self, allow_list: Vec<String>) -> Self {
        self.allow_list = Some(allow_list);
        self
    }

    /// Returns true if the tool should be visible to MCP clients.
    fn is_tool_exposed(&self, name: &str, risk: RiskLevel) -> bool {
        if risk == RiskLevel::Forbidden {
            return false;
        }
        if let Some(ref allow_list) = self.allow_list {
            return allow_list.iter().any(|n| n == name);
        }
        true
    }

    /// Convert a serde_json::Value (expected to be an object) into rmcp's JsonObject.
    fn to_json_object(schema: &Value) -> JsonObject {
        match schema.as_object() {
            Some(obj) => obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            None => {
                let mut obj = JsonObject::new();
                obj.insert("type".to_string(), Value::String("object".to_string()));
                obj
            }
        }
    }

    /// Build a `ToolContext` for an MCP-originated tool call.
    fn build_tool_context(&self, session_id: String) -> ToolContext {
        ToolContext {
            session_id,
            agent_id: self.default_context.agent_id.clone(),
            authority: InputAuthority::External,
            workspace_root: self.default_context.workspace_root.clone(),
            channel: Some("mcp".to_string()),
            db: self.default_context.db.clone(),
        }
    }
}

// ─── Core logic (testable without RequestContext) ──────────────────────────

impl IroncladMcpHandler {
    /// List all tools that should be visible to MCP clients.
    pub async fn list_exposed_tools(&self) -> Vec<McpTool> {
        let registry = &self.tool_registry;
        registry
            .list()
            .into_iter()
            .filter(|t| self.is_tool_exposed(t.name(), t.risk_level()))
            .map(|t| {
                let schema = Self::to_json_object(&t.parameters_schema());
                McpTool {
                    name: Cow::Owned(t.name().to_string()),
                    title: None,
                    description: Some(Cow::Owned(t.description().to_string())),
                    input_schema: Arc::new(schema),
                    output_schema: None,
                    annotations: None,
                    execution: None,
                    icons: None,
                    meta: None,
                }
            })
            .collect()
    }

    /// Execute a tool call and return an MCP-formatted result.
    pub async fn execute_tool_call(
        &self,
        tool_name: &str,
        arguments: JsonObject,
    ) -> CallToolResult {
        let registry = &self.tool_registry;

        let tool = match registry.get(tool_name) {
            Some(t) => t,
            None => {
                warn!(tool = tool_name, "MCP client requested unknown tool");
                return CallToolResult::error(vec![Content::text(format!(
                    "Unknown tool: {tool_name}"
                ))]);
            }
        };

        // Never execute Forbidden tools, even if somehow requested directly.
        if tool.risk_level() == RiskLevel::Forbidden {
            warn!(
                tool = tool_name,
                "MCP client attempted to call Forbidden tool"
            );
            return CallToolResult::error(vec![Content::text(
                "This tool is not available via MCP".to_string(),
            )]);
        }

        // Check allow-list
        if !self.is_tool_exposed(tool_name, tool.risk_level()) {
            warn!(
                tool = tool_name,
                "MCP client attempted to call tool not in allow-list"
            );
            return CallToolResult::error(vec![Content::text(
                "This tool is not available via MCP".to_string(),
            )]);
        }

        let params: Value = Value::Object(
            arguments
                .into_iter()
                .collect::<serde_json::Map<String, Value>>(),
        );

        let session_id = uuid::Uuid::new_v4().to_string();
        let ctx = self.build_tool_context(session_id);

        debug!(tool = tool_name, "Executing MCP tool call");

        match tool.execute(params, &ctx).await {
            Ok(result) => {
                let mut content = vec![Content::text(result.output)];
                if let Some(meta) = result.metadata {
                    content.push(Content::text(format!(
                        "\n---\nMetadata: {}",
                        serde_json::to_string_pretty(&meta).unwrap_or_default()
                    )));
                }
                CallToolResult::success(content)
            }
            Err(e) => {
                warn!(tool = tool_name, error = %e, "MCP tool call failed");
                CallToolResult::error(vec![Content::text(e.message)])
            }
        }
    }
}

// ─── ServerHandler trait impl (thin delegation) ────────────────────────────

impl ServerHandler for IroncladMcpHandler {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Ironclad agent runtime — tools are filtered by policy engine".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }

    #[allow(unused_variables)]
    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        let tools = self.list_exposed_tools().await;
        info!(count = tools.len(), "MCP tools/list");
        Ok(ListToolsResult {
            tools,
            next_cursor: None,
            meta: None,
        })
    }

    #[allow(unused_variables)]
    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, McpError> {
        info!(tool = %request.name, "MCP tools/call");
        Ok(self
            .execute_tool_call(&request.name, request.arguments.unwrap_or_default())
            .await)
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Tool, ToolError, ToolResult};
    use async_trait::async_trait;
    use serde_json::json;

    // --- Dummy tools for testing ---

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echoes input back"
        }
        fn risk_level(&self) -> RiskLevel {
            RiskLevel::Safe
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object", "properties": {"message": {"type": "string"}}})
        }
        async fn execute(
            &self,
            params: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            let msg = params
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("(empty)");
            Ok(ToolResult {
                output: format!("Echo: {msg}"),
                metadata: None,
            })
        }
    }

    struct ForbiddenTool;

    #[async_trait]
    impl Tool for ForbiddenTool {
        fn name(&self) -> &str {
            "nuke"
        }
        fn description(&self) -> &str {
            "Forbidden operation"
        }
        fn risk_level(&self) -> RiskLevel {
            RiskLevel::Forbidden
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            panic!("should never execute");
        }
    }

    struct FailingTool;

    #[async_trait]
    impl Tool for FailingTool {
        fn name(&self) -> &str {
            "fail"
        }
        fn description(&self) -> &str {
            "Always fails"
        }
        fn risk_level(&self) -> RiskLevel {
            RiskLevel::Safe
        }
        fn parameters_schema(&self) -> Value {
            json!({"type": "object"})
        }
        async fn execute(
            &self,
            _params: Value,
            _ctx: &ToolContext,
        ) -> Result<ToolResult, ToolError> {
            Err(ToolError {
                message: "Intentional failure".to_string(),
            })
        }
    }

    fn make_handler(tools: Vec<Box<dyn Tool>>) -> IroncladMcpHandler {
        let mut registry = ToolRegistry::new();
        for tool in tools {
            registry.register(tool);
        }
        let ctx = McpToolContext {
            agent_id: "test-agent".to_string(),
            workspace_root: std::path::PathBuf::from("/tmp"),
            db: None,
        };
        IroncladMcpHandler::new(Arc::new(registry), ctx)
    }

    #[tokio::test]
    async fn list_tools_excludes_forbidden() {
        let handler = make_handler(vec![Box::new(EchoTool), Box::new(ForbiddenTool)]);
        let tools = handler.list_exposed_tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name.as_ref(), "echo");
    }

    #[tokio::test]
    async fn list_tools_respects_allow_list() {
        let handler = make_handler(vec![Box::new(EchoTool), Box::new(FailingTool)])
            .with_allow_list(vec!["echo".to_string()]);
        let tools = handler.list_exposed_tools().await;
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name.as_ref(), "echo");
    }

    #[tokio::test]
    async fn execute_tool_success() {
        let handler = make_handler(vec![Box::new(EchoTool)]);
        let mut args = JsonObject::new();
        args.insert("message".to_string(), Value::String("hello".to_string()));
        let result = handler.execute_tool_call("echo", args).await;
        assert!(!result.is_error.unwrap_or(false));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert_eq!(text, "Echo: hello");
    }

    #[tokio::test]
    async fn execute_unknown_tool_returns_error() {
        let handler = make_handler(vec![Box::new(EchoTool)]);
        let result = handler
            .execute_tool_call("nonexistent", JsonObject::new())
            .await;
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn execute_forbidden_tool_returns_error() {
        let handler = make_handler(vec![Box::new(ForbiddenTool)]);
        let result = handler.execute_tool_call("nuke", JsonObject::new()).await;
        assert!(result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn execute_failing_tool_returns_error() {
        let handler = make_handler(vec![Box::new(FailingTool)]);
        let result = handler.execute_tool_call("fail", JsonObject::new()).await;
        assert!(result.is_error.unwrap_or(false));
        let text = result
            .content
            .first()
            .and_then(|c| c.as_text())
            .map(|t| t.text.as_str())
            .unwrap_or("");
        assert!(text.contains("Intentional failure"));
    }

    #[tokio::test]
    async fn tool_context_uses_external_authority() {
        let handler = make_handler(vec![]);
        let ctx = handler.build_tool_context("test-session".to_string());
        assert_eq!(ctx.authority, InputAuthority::External);
        assert_eq!(ctx.channel.as_deref(), Some("mcp"));
    }
}
