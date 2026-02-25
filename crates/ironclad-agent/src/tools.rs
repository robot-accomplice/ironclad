use std::collections::HashMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use async_trait::async_trait;
use serde_json::Value;

use ironclad_core::{InputAuthority, RiskLevel};

const MAX_FILE_BYTES: usize = 1024 * 1024;
const MAX_SEARCH_RESULTS: usize = 100;
const MAX_WALK_FILES: usize = 5000;

fn workspace_root_from_ctx(ctx: &ToolContext) -> std::result::Result<PathBuf, ToolError> {
    std::fs::canonicalize(&ctx.workspace_root).map_err(|e| ToolError {
        message: format!(
            "failed to resolve workspace root '{}': {e}",
            ctx.workspace_root.display()
        ),
    })
}

fn validate_rel_path(rel: &Path) -> std::result::Result<(), ToolError> {
    if rel.is_absolute() {
        return Err(ToolError {
            message: "absolute paths are not allowed".into(),
        });
    }
    if rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(ToolError {
            message: "path traversal ('..') is not allowed".into(),
        });
    }
    Ok(())
}

fn resolve_workspace_path(
    root: &Path,
    rel: &str,
    allow_nonexistent: bool,
) -> std::result::Result<PathBuf, ToolError> {
    let rel_path = Path::new(rel);
    validate_rel_path(rel_path)?;
    let joined = root.join(rel_path);
    if joined.exists() {
        let canonical = std::fs::canonicalize(&joined).map_err(|e| ToolError {
            message: format!("failed to resolve '{}': {e}", joined.display()),
        })?;
        if !canonical.starts_with(root) {
            return Err(ToolError {
                message: "resolved path escapes workspace root".into(),
            });
        }
        return Ok(canonical);
    }

    if !allow_nonexistent {
        return Err(ToolError {
            message: format!("path does not exist: {}", joined.display()),
        });
    }

    if let Some(parent) = joined.parent() {
        let mut existing_ancestor = parent;
        while !existing_ancestor.exists() {
            existing_ancestor = existing_ancestor.parent().ok_or_else(|| ToolError {
                message: "unable to resolve existing parent for target path".into(),
            })?;
        }
        let canonical_parent = std::fs::canonicalize(existing_ancestor).map_err(|e| ToolError {
            message: format!(
                "failed to resolve parent '{}': {e}",
                existing_ancestor.display()
            ),
        })?;
        if !canonical_parent.starts_with(root) {
            return Err(ToolError {
                message: "target path escapes workspace root".into(),
            });
        }
    }

    Ok(joined)
}

fn walk_workspace_files(
    base: &Path,
    out: &mut Vec<PathBuf>,
    count: &mut usize,
) -> std::result::Result<(), ToolError> {
    if *count >= MAX_WALK_FILES {
        return Ok(());
    }
    let rd = std::fs::read_dir(base).map_err(|e| ToolError {
        message: format!("failed to read directory '{}': {e}", base.display()),
    })?;
    for entry in rd {
        if *count >= MAX_WALK_FILES {
            break;
        }
        let entry = entry.map_err(|e| ToolError {
            message: format!("failed to read directory entry: {e}"),
        })?;
        let path = entry.path();
        let ftype = entry.file_type().map_err(|e| ToolError {
            message: format!("failed to inspect '{}': {e}", path.display()),
        })?;
        if ftype.is_symlink() {
            continue;
        }
        if ftype.is_dir() {
            walk_workspace_files(&path, out, count)?;
        } else if ftype.is_file() {
            out.push(path);
            *count += 1;
        }
    }
    Ok(())
}

fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    // Simple glob-style matcher for '*' and '?'.
    let p: Vec<char> = pattern.chars().collect();
    let s: Vec<char> = candidate.chars().collect();
    let (mut pi, mut si) = (0usize, 0usize);
    let (mut star, mut match_i) = (None::<usize>, 0usize);
    while si < s.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == s[si]) {
            pi += 1;
            si += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            pi += 1;
            match_i = si;
        } else if let Some(star_idx) = star {
            pi = star_idx + 1;
            match_i += 1;
            si = match_i;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn risk_level(&self) -> RiskLevel;
    fn parameters_schema(&self) -> Value;
    async fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError>;
}

#[derive(Debug, Clone)]
pub struct ToolContext {
    pub session_id: String,
    pub agent_id: String,
    pub authority: InputAuthority,
    pub workspace_root: PathBuf,
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
                if result.exit_code != 0 {
                    return Err(ToolError {
                        message: format!(
                            "script exited with code {}: {}",
                            result.exit_code, result.stderr
                        ),
                    });
                }
                Ok(ToolResult {
                    output: result.stdout,
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

pub struct ReadFileTool;

#[async_trait]
impl Tool for ReadFileTool {
    fn name(&self) -> &str {
        "read_file"
    }

    fn description(&self) -> &str {
        "Read a UTF-8 text file from the workspace"
    }

    fn risk_level(&self) -> RiskLevel {
        // Reading workspace files should not be callable by untrusted External input.
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let rel = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'path' parameter".into(),
            })?;
        let root = workspace_root_from_ctx(ctx)?;
        let path = resolve_workspace_path(&root, rel, false)?;
        let meta = std::fs::metadata(&path).map_err(|e| ToolError {
            message: format!("failed to stat '{}': {e}", path.display()),
        })?;
        if meta.len() as usize > MAX_FILE_BYTES {
            return Err(ToolError {
                message: format!(
                    "file too large (>{MAX_FILE_BYTES} bytes): {}",
                    path.display()
                ),
            });
        }
        let content = std::fs::read_to_string(&path).map_err(|e| ToolError {
            message: format!("failed to read '{}': {e}", path.display()),
        })?;
        Ok(ToolResult {
            output: content,
            metadata: Some(
                serde_json::json!({ "path": path.display().to_string(), "bytes": meta.len() }),
            ),
        })
    }
}

pub struct WriteFileTool;

#[async_trait]
impl Tool for WriteFileTool {
    fn name(&self) -> &str {
        "write_file"
    }

    fn description(&self) -> &str {
        "Write text content to a workspace file"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "content": { "type": "string" },
                "append": { "type": "boolean", "default": false }
            },
            "required": ["path", "content"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let rel = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'path' parameter".into(),
            })?;
        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'content' parameter".into(),
            })?;
        let append = params
            .get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = workspace_root_from_ctx(ctx)?;
        let path = resolve_workspace_path(&root, rel, true)?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| ToolError {
                message: format!("failed to create parent dirs '{}': {e}", parent.display()),
            })?;
        }
        if append {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .map_err(|e| ToolError {
                    message: format!("failed to open '{}': {e}", path.display()),
                })?;
            f.write_all(content.as_bytes()).map_err(|e| ToolError {
                message: format!("failed to append '{}': {e}", path.display()),
            })?;
        } else {
            std::fs::write(&path, content).map_err(|e| ToolError {
                message: format!("failed to write '{}': {e}", path.display()),
            })?;
        }
        Ok(ToolResult {
            output: "ok".into(),
            metadata: Some(
                serde_json::json!({ "path": path.display().to_string(), "append": append }),
            ),
        })
    }
}

pub struct EditFileTool;

#[async_trait]
impl Tool for EditFileTool {
    fn name(&self) -> &str {
        "edit_file"
    }

    fn description(&self) -> &str {
        "Replace text in an existing workspace file"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string" },
                "old_text": { "type": "string" },
                "new_text": { "type": "string" },
                "replace_all": { "type": "boolean", "default": false }
            },
            "required": ["path", "old_text", "new_text"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let rel = params
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'path' parameter".into(),
            })?;
        let old_text = params
            .get("old_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'old_text' parameter".into(),
            })?;
        let new_text = params
            .get("new_text")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'new_text' parameter".into(),
            })?;
        let replace_all = params
            .get("replace_all")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = workspace_root_from_ctx(ctx)?;
        let path = resolve_workspace_path(&root, rel, false)?;
        let content = std::fs::read_to_string(&path).map_err(|e| ToolError {
            message: format!("failed to read '{}': {e}", path.display()),
        })?;
        if !content.contains(old_text) {
            return Err(ToolError {
                message: "old_text not found in file".into(),
            });
        }
        let updated = if replace_all {
            content.replace(old_text, new_text)
        } else {
            content.replacen(old_text, new_text, 1)
        };
        std::fs::write(&path, updated).map_err(|e| ToolError {
            message: format!("failed to write '{}': {e}", path.display()),
        })?;
        Ok(ToolResult {
            output: "ok".into(),
            metadata: Some(
                serde_json::json!({ "path": path.display().to_string(), "replace_all": replace_all }),
            ),
        })
    }
}

pub struct ListDirectoryTool;

#[async_trait]
impl Tool for ListDirectoryTool {
    fn name(&self) -> &str {
        "list_directory"
    }

    fn description(&self) -> &str {
        "List files and folders in a workspace directory"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": { "type": "string", "default": "." }
            }
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let rel = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let root = workspace_root_from_ctx(ctx)?;
        let path = resolve_workspace_path(&root, rel, false)?;
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(&path).map_err(|e| ToolError {
            message: format!("failed to read directory '{}': {e}", path.display()),
        })? {
            let entry = entry.map_err(|e| ToolError {
                message: format!("failed to read entry: {e}"),
            })?;
            let p = entry.path();
            let kind = if p.is_dir() { "dir" } else { "file" };
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            entries.push(serde_json::json!({ "name": name, "kind": kind }));
        }
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or_default()
                .cmp(b["name"].as_str().unwrap_or_default())
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&entries).unwrap_or_else(|_| "[]".to_string()),
            metadata: Some(
                serde_json::json!({ "path": path.display().to_string(), "count": entries.len() }),
            ),
        })
    }
}

pub struct GlobFilesTool;

#[async_trait]
impl Tool for GlobFilesTool {
    fn name(&self) -> &str {
        "glob_files"
    }

    fn description(&self) -> &str {
        "Find files matching a wildcard pattern under the workspace"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "pattern": { "type": "string" },
                "path": { "type": "string", "default": "." },
                "limit": { "type": "integer", "default": 50, "minimum": 1, "maximum": 500 }
            },
            "required": ["pattern"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let pattern = params
            .get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'pattern' parameter".into(),
            })?;
        let rel = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(50)
            .min(500);
        let root = workspace_root_from_ctx(ctx)?;
        let base = resolve_workspace_path(&root, rel, false)?;
        let mut files = Vec::new();
        let mut count = 0usize;
        walk_workspace_files(&base, &mut files, &mut count)?;
        let mut matches = Vec::new();
        for p in files {
            let rel = p.strip_prefix(&root).unwrap_or(&p);
            let rel_norm = rel.to_string_lossy().replace('\\', "/");
            if wildcard_match(pattern, &rel_norm) {
                matches.push(rel_norm);
                if matches.len() >= limit {
                    break;
                }
            }
        }
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&matches).unwrap_or_else(|_| "[]".to_string()),
            metadata: Some(serde_json::json!({ "count": matches.len(), "pattern": pattern })),
        })
    }
}

pub struct SearchFilesTool;

#[async_trait]
impl Tool for SearchFilesTool {
    fn name(&self) -> &str {
        "search_files"
    }

    fn description(&self) -> &str {
        "Search for text content across workspace files"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": { "type": "string" },
                "path": { "type": "string", "default": "." },
                "limit": { "type": "integer", "default": 20, "minimum": 1, "maximum": 100 },
                "case_sensitive": { "type": "boolean", "default": false }
            },
            "required": ["query"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let query = params
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'query' parameter".into(),
            })?;
        let rel = params.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let limit = params
            .get("limit")
            .and_then(|v| v.as_u64())
            .map(|n| n as usize)
            .unwrap_or(20)
            .min(MAX_SEARCH_RESULTS);
        let case_sensitive = params
            .get("case_sensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let root = workspace_root_from_ctx(ctx)?;
        let base = resolve_workspace_path(&root, rel, false)?;
        let mut files = Vec::new();
        let mut count = 0usize;
        walk_workspace_files(&base, &mut files, &mut count)?;
        let mut hits = Vec::new();
        let mut unreadable_files = 0usize;
        let query_cmp = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        for p in files {
            if hits.len() >= limit {
                break;
            }
            let content = match std::fs::read_to_string(&p) {
                Ok(c) => c,
                Err(_) => {
                    unreadable_files += 1;
                    continue;
                }
            };
            for (idx, line) in content.lines().enumerate() {
                let cmp = if case_sensitive {
                    line.to_string()
                } else {
                    line.to_lowercase()
                };
                if cmp.contains(&query_cmp) {
                    let relp = p
                        .strip_prefix(&root)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .replace('\\', "/");
                    hits.push(serde_json::json!({
                        "path": relp,
                        "line": idx + 1,
                        "preview": line
                    }));
                    if hits.len() >= limit {
                        break;
                    }
                }
            }
        }
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&hits).unwrap_or_else(|_| "[]".to_string()),
            metadata: Some(serde_json::json!({
                "count": hits.len(),
                "query": query,
                "unreadable_files": unreadable_files
            })),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironclad_core::config::SkillsConfig;
    use uuid::Uuid;

    fn test_ctx() -> ToolContext {
        ToolContext {
            session_id: "test-session".into(),
            agent_id: "test-agent".into(),
            authority: InputAuthority::Creator,
            workspace_root: std::env::current_dir().unwrap(),
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

    #[test]
    fn wildcard_match_supports_star_and_question() {
        assert!(wildcard_match("src/*.rs", "src/main.rs"));
        assert!(wildcard_match("src/???.rs", "src/mod.rs"));
        assert!(!wildcard_match("src/*.rs", "src/main.ts"));
    }

    #[tokio::test]
    async fn filesystem_tools_roundtrip() {
        let ctx = test_ctx();
        let unique = format!(".tmp_tools_test_{}", Uuid::new_v4());
        let rel_file = format!("{unique}/note.txt");
        let _ = std::fs::create_dir_all(&unique);

        let write = WriteFileTool;
        write
            .execute(
                serde_json::json!({"path": rel_file, "content": "hello tools"}),
                &ctx,
            )
            .await
            .unwrap();

        let read = ReadFileTool;
        let out = read
            .execute(
                serde_json::json!({"path": format!("{unique}/note.txt")}),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(out.output, "hello tools");

        let list = ListDirectoryTool;
        let listed = list
            .execute(serde_json::json!({"path": unique.clone()}), &ctx)
            .await
            .unwrap();
        assert!(listed.output.contains("note.txt"));

        let _ = std::fs::remove_dir_all(unique);
    }

    #[test]
    fn filesystem_tool_risk_levels_block_external_authority() {
        assert_eq!(ReadFileTool.risk_level(), RiskLevel::Caution);
        assert_eq!(ListDirectoryTool.risk_level(), RiskLevel::Caution);
        assert_eq!(GlobFilesTool.risk_level(), RiskLevel::Caution);
        assert_eq!(SearchFilesTool.risk_level(), RiskLevel::Caution);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn run_script_tool_nonzero_exit_is_error() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script_path = dir.path().join("fail.sh");
        std::fs::write(&script_path, "#!/bin/bash\necho boom >&2\nexit 7").unwrap();
        std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o755)).unwrap();

        let cfg = SkillsConfig {
            skills_dir: dir.path().to_path_buf(),
            allowed_interpreters: vec!["bash".to_string()],
            ..Default::default()
        };
        let tool = ScriptRunnerTool::new(cfg);
        let ctx = test_ctx();

        let err = tool
            .execute(serde_json::json!({"path": "fail.sh"}), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("script exited with code 7"));
    }
}
