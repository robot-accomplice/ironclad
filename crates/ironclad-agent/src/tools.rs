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
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let skip_dir = name.starts_with('.') || name == "node_modules";
        let path = entry.path();
        let ftype = entry.file_type().map_err(|e| ToolError {
            message: format!("failed to inspect '{}': {e}", path.display()),
        })?;
        if ftype.is_symlink() {
            continue;
        }
        if ftype.is_dir() {
            if skip_dir {
                continue;
            }
            walk_workspace_files(&path, out, count)?;
        } else if ftype.is_file() {
            out.push(path);
            *count += 1;
        }
    }
    Ok(())
}

fn wildcard_match_segment(pattern: &str, candidate: &str) -> bool {
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

fn wildcard_match(pattern: &str, candidate: &str) -> bool {
    fn rec(
        p: &[&str],
        c: &[&str],
        pi: usize,
        ci: usize,
        memo: &mut std::collections::HashMap<(usize, usize), bool>,
    ) -> bool {
        if let Some(v) = memo.get(&(pi, ci)) {
            return *v;
        }

        let out = if pi == p.len() {
            ci == c.len()
        } else if p[pi] == "**" {
            // Collapse consecutive ** tokens.
            let mut next_pi = pi + 1;
            while next_pi < p.len() && p[next_pi] == "**" {
                next_pi += 1;
            }
            if next_pi == p.len() {
                true
            } else {
                (ci..=c.len()).any(|next_ci| rec(p, c, next_pi, next_ci, memo))
            }
        } else if ci < c.len() && wildcard_match_segment(p[pi], c[ci]) {
            rec(p, c, pi + 1, ci + 1, memo)
        } else {
            false
        };

        memo.insert((pi, ci), out);
        out
    }

    let pattern_norm = pattern.replace('\\', "/");
    let candidate_norm = candidate.replace('\\', "/");
    let p: Vec<&str> = pattern_norm.split('/').filter(|s| !s.is_empty()).collect();
    let c: Vec<&str> = candidate_norm
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    rec(&p, &c, 0, 0, &mut std::collections::HashMap::new())
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
    /// The channel through which the current message arrived (e.g. "api", "telegram", "discord").
    /// `None` when channel is unknown or the tool was invoked outside a channel context.
    pub channel: Option<String>,
    /// Optional database handle for tools that need to query runtime state
    /// (e.g. subagent status, task lists, delivery queue depth).
    pub db: Option<ironclad_db::Database>,
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
        let mut skipped_large_files = 0usize;
        let query_cmp = if case_sensitive {
            query.to_string()
        } else {
            query.to_lowercase()
        };
        for p in files {
            if hits.len() >= limit {
                break;
            }
            let file_size = match std::fs::metadata(&p) {
                Ok(meta) => meta.len(),
                Err(_) => {
                    unreadable_files += 1;
                    continue;
                }
            };
            if file_size > MAX_FILE_BYTES as u64 {
                skipped_large_files += 1;
                continue;
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
                "unreadable_files": unreadable_files,
                "skipped_large_files": skipped_large_files
            })),
        })
    }
}

// ── Introspection Tools ─────────────────────────────────────────────────────
// Read-only probes that let the agent reason about its own runtime state.
// All return JSON strings so the LLM can parse structured data.

/// Reports runtime context: agent id, session, channel, and workspace.
pub struct GetRuntimeContextTool;

#[async_trait]
impl Tool for GetRuntimeContextTool {
    fn name(&self) -> &str {
        "get_runtime_context"
    }

    fn description(&self) -> &str {
        "Returns the current agent runtime context including session, channel, and workspace path"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let info = serde_json::json!({
            "agent_id": ctx.agent_id,
            "session_id": ctx.session_id,
            "channel": ctx.channel,
            "workspace_root": ctx.workspace_root.display().to_string(),
            "authority": format!("{:?}", ctx.authority),
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&info).unwrap_or_else(|_| "{}".into()),
            metadata: Some(info),
        })
    }
}

/// Reports memory budget allocation and retrieval tier configuration.
pub struct GetMemoryStatsTool;

#[async_trait]
impl Tool for GetMemoryStatsTool {
    fn name(&self) -> &str {
        "get_memory_stats"
    }

    fn description(&self) -> &str {
        "Returns memory retrieval tier allocations and configuration"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        // Return the default tier budgets; runtime overrides would require
        // access to the live config, which we thread in when available.
        let tiers = serde_json::json!({
            "tiers": {
                "working": { "budget_pct": 30, "description": "Active conversation context" },
                "episodic": { "budget_pct": 25, "description": "Session digests and summaries" },
                "semantic": { "budget_pct": 20, "description": "Vector-similarity recalled facts" },
                "procedural": { "budget_pct": 15, "description": "How-to knowledge and procedures" },
                "relationship": { "budget_pct": 10, "description": "Entity relationships and graph" },
            },
            "retrieval_method": "5-tier hybrid (FTS5 + vector cosine)",
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&tiers).unwrap_or_else(|_| "{}".into()),
            metadata: Some(tiers),
        })
    }
}

/// Reports the health of the current delivery channel.
pub struct GetChannelHealthTool;

#[async_trait]
impl Tool for GetChannelHealthTool {
    fn name(&self) -> &str {
        "get_channel_health"
    }

    fn description(&self) -> &str {
        "Returns the health status of the current delivery channel"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let channel = ctx.channel.as_deref().unwrap_or("unknown");
        let health = serde_json::json!({
            "channel": channel,
            "status": "operational",
            "note": "Detailed channel health metrics require a ChannelRouter reference; \
                     basic connectivity confirmed by successful tool invocation.",
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&health).unwrap_or_else(|_| "{}".into()),
            metadata: Some(health),
        })
    }
}

// ── Subagent & Task Introspection ──────────────────────────────────────

/// Returns the status of registered subagents and open tasks.
///
/// Designed to grow over time — future versions may include delegation
/// history, task completion rates, and specialist performance metrics.
pub struct GetSubagentStatusTool;

#[async_trait]
impl Tool for GetSubagentStatusTool {
    fn name(&self) -> &str {
        "get_subagent_status"
    }

    fn description(&self) -> &str {
        "Returns the status of registered subagents (specialists) and open tasks"
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {},
            "required": []
        })
    }

    async fn execute(
        &self,
        _params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let db = match &ctx.db {
            Some(db) => db,
            None => {
                let result = serde_json::json!({
                    "error": "database not available",
                    "subagents": [],
                    "tasks": [],
                });
                return Ok(ToolResult {
                    output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
                    metadata: Some(result),
                });
            }
        };

        // Query subagents
        let subagents = ironclad_db::agents::list_sub_agents(db)
            .unwrap_or_default()
            .into_iter()
            .map(|a| {
                serde_json::json!({
                    "name": a.name,
                    "display_name": a.display_name,
                    "model": a.model,
                    "role": a.role,
                    "enabled": a.enabled,
                    "session_count": a.session_count,
                })
            })
            .collect::<Vec<_>>();

        // Query open tasks
        let tasks = {
            let conn = db.conn();
            conn.prepare(
                "SELECT id, title, status, priority, source, created_at \
                 FROM tasks WHERE status IN ('pending', 'in_progress') \
                 ORDER BY priority DESC, created_at ASC LIMIT 50",
            )
            .ok()
            .map(|mut stmt| {
                stmt.query_map([], |row| {
                    Ok(serde_json::json!({
                        "id": row.get::<_, String>(0)?,
                        "title": row.get::<_, String>(1)?,
                        "status": row.get::<_, String>(2)?,
                        "priority": row.get::<_, i64>(3)?,
                        "source": row.get::<_, Option<String>>(4)?,
                        "created_at": row.get::<_, String>(5)?,
                    }))
                })
                .ok()
                .map(|rows| rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
                .unwrap_or_default()
            })
            .unwrap_or_default()
        };

        let result = serde_json::json!({
            "subagents": subagents,
            "subagent_count": subagents.len(),
            "tasks": tasks,
            "open_task_count": tasks.len(),
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
            metadata: Some(result),
        })
    }
}

// ── Agent Data Tools ───────────────────────────────────────────────────
// Let the agent create, modify, and drop its own database tables.
// All tables are prefixed with the agent id for isolation.

const MAX_AGENT_TABLES: usize = 50;
const MAX_COLUMNS_PER_TABLE: usize = 64;
const ALLOWED_COL_TYPES: &[&str] = &["TEXT", "INTEGER", "REAL", "BLOB"];
const RESERVED_COL_NAMES: &[&str] = &["id", "created_at", "rowid"];

fn require_db(ctx: &ToolContext) -> std::result::Result<&ironclad_db::Database, ToolError> {
    ctx.db.as_ref().ok_or_else(|| ToolError {
        message: "database not available in this context".into(),
    })
}

fn parse_column_defs(
    raw: &[Value],
) -> std::result::Result<Vec<ironclad_db::hippocampus::ColumnDef>, ToolError> {
    let mut cols = Vec::with_capacity(raw.len());
    for (i, v) in raw.iter().enumerate() {
        let name = v
            .get("name")
            .and_then(|n| n.as_str())
            .ok_or_else(|| ToolError {
                message: format!("column {i}: missing 'name'"),
            })?;

        if RESERVED_COL_NAMES.contains(&name.to_lowercase().as_str()) {
            return Err(ToolError {
                message: format!("column '{name}' is reserved and added automatically"),
            });
        }

        let col_type = v
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("TEXT")
            .to_uppercase();

        if !ALLOWED_COL_TYPES.contains(&col_type.as_str()) {
            return Err(ToolError {
                message: format!(
                    "column '{name}': type '{col_type}' not allowed (use TEXT, INTEGER, REAL, or BLOB)"
                ),
            });
        }

        let nullable = v.get("nullable").and_then(|n| n.as_bool()).unwrap_or(true);
        let description = v
            .get("description")
            .and_then(|d| d.as_str())
            .map(String::from);

        cols.push(ironclad_db::hippocampus::ColumnDef {
            name: name.into(),
            col_type,
            nullable,
            description,
        });
    }
    Ok(cols)
}

/// Creates a new agent-owned database table. Tables are automatically
/// prefixed with the agent id and registered in the hippocampus.
pub struct CreateTableTool;

#[async_trait]
impl Tool for CreateTableTool {
    fn name(&self) -> &str {
        "create_table"
    }

    fn description(&self) -> &str {
        "Create a new database table owned by this agent. Tables are prefixed with the agent id \
         for isolation. Columns 'id' (TEXT PK) and 'created_at' are added automatically."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "name": {
                    "type": "string",
                    "description": "Table suffix (will be prefixed with agent id). Alphanumeric and underscores only."
                },
                "description": {
                    "type": "string",
                    "description": "Human-readable description of the table's purpose"
                },
                "columns": {
                    "type": "array",
                    "description": "Column definitions. Each has 'name', optional 'type' (TEXT|INTEGER|REAL|BLOB, default TEXT), optional 'nullable' (default true), optional 'description'.",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": { "type": "string" },
                            "type": { "type": "string" },
                            "nullable": { "type": "boolean" },
                            "description": { "type": "string" }
                        },
                        "required": ["name"]
                    }
                }
            },
            "required": ["name", "description", "columns"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let db = require_db(ctx)?;

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'name' parameter".into(),
            })?;
        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'description' parameter".into(),
            })?;
        let raw_columns = params
            .get("columns")
            .and_then(|v| v.as_array())
            .ok_or_else(|| ToolError {
                message: "missing 'columns' array parameter".into(),
            })?;

        if raw_columns.len() > MAX_COLUMNS_PER_TABLE {
            return Err(ToolError {
                message: format!(
                    "too many columns ({}, max {MAX_COLUMNS_PER_TABLE})",
                    raw_columns.len()
                ),
            });
        }

        // Enforce per-agent table limit
        let existing =
            ironclad_db::hippocampus::list_agent_tables(db, &ctx.agent_id).map_err(|e| {
                ToolError {
                    message: format!("failed to check existing tables: {e}"),
                }
            })?;
        if existing.len() >= MAX_AGENT_TABLES {
            return Err(ToolError {
                message: format!(
                    "agent table limit reached ({MAX_AGENT_TABLES}). Drop unused tables first."
                ),
            });
        }

        let columns = parse_column_defs(raw_columns)?;

        let full_name = ironclad_db::hippocampus::create_agent_table(
            db,
            &ctx.agent_id,
            name,
            description,
            &columns,
        )
        .map_err(|e| ToolError {
            message: format!("failed to create table: {e}"),
        })?;

        let result = serde_json::json!({
            "table_name": full_name,
            "columns_created": columns.len(),
            "note": "Columns 'id' (TEXT PK) and 'created_at' (TEXT) are added automatically."
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
            metadata: Some(result),
        })
    }
}

/// Adds or drops columns on an agent-owned table.
pub struct AlterTableTool;

#[async_trait]
impl Tool for AlterTableTool {
    fn name(&self) -> &str {
        "alter_table"
    }

    fn description(&self) -> &str {
        "Add or drop columns on a table owned by this agent. Use operation 'add_column' or 'drop_column'."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "table_name": {
                    "type": "string",
                    "description": "Full table name (including agent prefix)"
                },
                "operation": {
                    "type": "string",
                    "enum": ["add_column", "drop_column"],
                    "description": "The alteration to perform"
                },
                "column": {
                    "type": "object",
                    "description": "Column definition for add_column: {name, type?, nullable?, description?}. For drop_column: {name}.",
                    "properties": {
                        "name": { "type": "string" },
                        "type": { "type": "string" },
                        "nullable": { "type": "boolean" },
                        "description": { "type": "string" }
                    },
                    "required": ["name"]
                }
            },
            "required": ["table_name", "operation", "column"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let db = require_db(ctx)?;

        let table_name = params
            .get("table_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'table_name' parameter".into(),
            })?;
        let operation = params
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'operation' parameter".into(),
            })?;
        let column = params.get("column").ok_or_else(|| ToolError {
            message: "missing 'column' parameter".into(),
        })?;

        let col_name = column
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "column missing 'name' field".into(),
            })?;

        // Verify ownership via hippocampus
        let entry = ironclad_db::hippocampus::get_table(db, table_name)
            .map_err(|e| ToolError {
                message: format!("failed to look up table: {e}"),
            })?
            .ok_or_else(|| ToolError {
                message: format!("table '{table_name}' not found in hippocampus"),
            })?;

        if !entry.agent_owned || entry.created_by != ctx.agent_id {
            return Err(ToolError {
                message: format!("table '{table_name}' is not owned by this agent"),
            });
        }

        match operation {
            "add_column" => {
                if RESERVED_COL_NAMES.contains(&col_name.to_lowercase().as_str()) {
                    return Err(ToolError {
                        message: format!("column '{col_name}' is reserved"),
                    });
                }

                let col_type = column
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or("TEXT")
                    .to_uppercase();

                if !ALLOWED_COL_TYPES.contains(&col_type.as_str()) {
                    return Err(ToolError {
                        message: format!("type '{col_type}' not allowed"),
                    });
                }

                let nullable = column
                    .get("nullable")
                    .and_then(|n| n.as_bool())
                    .unwrap_or(true);

                // Validate identifier safety
                if !col_name
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_')
                    || col_name.is_empty()
                {
                    return Err(ToolError {
                        message: format!("invalid column name: '{col_name}'"),
                    });
                }

                let null_clause = if nullable { "" } else { " NOT NULL DEFAULT ''" };
                let sql = format!(
                    "ALTER TABLE \"{}\" ADD COLUMN {} {}{}",
                    table_name, col_name, col_type, null_clause
                );
                let conn = db.conn();
                conn.execute(&sql, []).map_err(|e| ToolError {
                    message: format!("ALTER TABLE failed: {e}"),
                })?;

                // Re-introspect and update hippocampus
                let description = column
                    .get("description")
                    .and_then(|d| d.as_str())
                    .map(String::from);
                let mut new_columns = entry.columns.clone();
                new_columns.push(ironclad_db::hippocampus::ColumnDef {
                    name: col_name.into(),
                    col_type: col_type.clone(),
                    nullable,
                    description,
                });
                drop(conn);
                ironclad_db::hippocampus::register_table(
                    db,
                    table_name,
                    &entry.description,
                    &new_columns,
                    &entry.created_by,
                    true,
                    &entry.access_level,
                    entry.row_count,
                )
                .map_err(|e| ToolError {
                    message: format!("failed to update hippocampus: {e}"),
                })?;

                let result = serde_json::json!({
                    "table_name": table_name,
                    "operation": "add_column",
                    "column_name": col_name,
                    "column_type": col_type,
                });
                Ok(ToolResult {
                    output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
                    metadata: Some(result),
                })
            }
            "drop_column" => {
                if RESERVED_COL_NAMES.contains(&col_name.to_lowercase().as_str()) {
                    return Err(ToolError {
                        message: format!("cannot drop reserved column '{col_name}'"),
                    });
                }

                let sql = format!("ALTER TABLE \"{}\" DROP COLUMN {}", table_name, col_name);
                let conn = db.conn();
                conn.execute(&sql, []).map_err(|e| ToolError {
                    message: format!("ALTER TABLE DROP COLUMN failed: {e}"),
                })?;

                // Update hippocampus entry
                let new_columns: Vec<_> = entry
                    .columns
                    .iter()
                    .filter(|c| c.name != col_name)
                    .cloned()
                    .collect();
                drop(conn);
                ironclad_db::hippocampus::register_table(
                    db,
                    table_name,
                    &entry.description,
                    &new_columns,
                    &entry.created_by,
                    true,
                    &entry.access_level,
                    entry.row_count,
                )
                .map_err(|e| ToolError {
                    message: format!("failed to update hippocampus: {e}"),
                })?;

                let result = serde_json::json!({
                    "table_name": table_name,
                    "operation": "drop_column",
                    "column_name": col_name,
                });
                Ok(ToolResult {
                    output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
                    metadata: Some(result),
                })
            }
            other => Err(ToolError {
                message: format!("unknown operation '{other}' (use 'add_column' or 'drop_column')"),
            }),
        }
    }
}

/// Drops an agent-owned table and removes it from the hippocampus.
pub struct DropTableTool;

#[async_trait]
impl Tool for DropTableTool {
    fn name(&self) -> &str {
        "drop_table"
    }

    fn description(&self) -> &str {
        "Drop a table owned by this agent. The table and all its data are permanently deleted."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "table_name": {
                    "type": "string",
                    "description": "Full table name (including agent prefix) to drop"
                }
            },
            "required": ["table_name"]
        })
    }

    async fn execute(
        &self,
        params: Value,
        ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let db = require_db(ctx)?;

        let table_name = params
            .get("table_name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'table_name' parameter".into(),
            })?;

        ironclad_db::hippocampus::drop_agent_table(db, &ctx.agent_id, table_name).map_err(|e| {
            ToolError {
                message: format!("failed to drop table: {e}"),
            }
        })?;

        let result = serde_json::json!({
            "table_name": table_name,
            "status": "dropped",
        });
        Ok(ToolResult {
            output: serde_json::to_string_pretty(&result).unwrap_or_else(|_| "{}".into()),
            metadata: Some(result),
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
            channel: None,
            db: None,
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

    #[test]
    fn wildcard_match_single_star_does_not_cross_directories() {
        assert!(wildcard_match("*.rs", "main.rs"));
        assert!(!wildcard_match("*.rs", "src/main.rs"));
    }

    #[test]
    fn wildcard_match_double_star_crosses_directories() {
        assert!(wildcard_match("**/*.rs", "src/nested/deep/main.rs"));
        assert!(wildcard_match("src/**/*.rs", "src/main.rs"));
        assert!(wildcard_match("src/**/*.rs", "src/nested/main.rs"));
    }

    #[test]
    fn walk_workspace_files_skips_hidden_and_node_modules_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();

        std::fs::create_dir_all(root.join(".git/objects")).unwrap();
        std::fs::write(root.join(".git/objects/hidden.txt"), "x").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "x").unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main() {}").unwrap();

        let mut files = Vec::new();
        let mut count = 0usize;
        walk_workspace_files(root, &mut files, &mut count).unwrap();
        let rels: Vec<String> = files
            .iter()
            .map(|p| {
                p.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();

        assert!(rels.iter().any(|p| p == "src/main.rs"));
        assert!(!rels.iter().any(|p| p.starts_with(".git/")));
        assert!(!rels.iter().any(|p| p.starts_with("node_modules/")));
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

    #[tokio::test]
    async fn search_files_skips_oversized_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("small.txt"), "needle").unwrap();
        std::fs::write(dir.path().join("large.txt"), vec![b'a'; MAX_FILE_BYTES + 1]).unwrap();

        let tool = SearchFilesTool;
        let ctx = ToolContext {
            session_id: "test-session".into(),
            agent_id: "test-agent".into(),
            authority: InputAuthority::Creator,
            workspace_root: dir.path().to_path_buf(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(serde_json::json!({"query": "needle", "path": "."}), &ctx)
            .await
            .unwrap();

        assert!(result.output.contains("small.txt"));
        assert!(!result.output.contains("large.txt"));

        let metadata = result.metadata.unwrap();
        assert_eq!(metadata["skipped_large_files"].as_u64(), Some(1));
    }

    #[test]
    fn validate_rel_path_rejects_absolute() {
        let p = Path::new("/etc/passwd");
        let err = validate_rel_path(p).unwrap_err();
        assert!(err.message.contains("absolute"));
    }

    #[test]
    fn validate_rel_path_rejects_parent_traversal() {
        let p = Path::new("subdir/../../etc/passwd");
        let err = validate_rel_path(p).unwrap_err();
        assert!(err.message.contains("traversal"));
    }

    #[test]
    fn validate_rel_path_accepts_normal() {
        assert!(validate_rel_path(Path::new("src/main.rs")).is_ok());
        assert!(validate_rel_path(Path::new("file.txt")).is_ok());
        assert!(validate_rel_path(Path::new("a/b/c/d")).is_ok());
    }

    #[test]
    fn resolve_workspace_path_nonexistent_disallowed() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let err = resolve_workspace_path(&root, "does_not_exist.txt", false).unwrap_err();
        assert!(err.message.contains("does not exist"));
    }

    #[test]
    fn resolve_workspace_path_nonexistent_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let result = resolve_workspace_path(&root, "new_file.txt", true).unwrap();
        assert!(result.to_string_lossy().contains("new_file.txt"));
    }

    #[test]
    fn resolve_workspace_path_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("hello.txt"), "hi").unwrap();
        let result = resolve_workspace_path(&root, "hello.txt", false).unwrap();
        assert!(result.starts_with(&root));
    }

    #[cfg(unix)]
    #[test]
    fn resolve_workspace_path_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        // Create a symlink that points outside the workspace
        let link_path = root.join("escape");
        std::os::unix::fs::symlink("/tmp", &link_path).unwrap();
        let err = resolve_workspace_path(&root, "escape", false).unwrap_err();
        assert!(err.message.contains("escapes workspace root"));
    }

    #[tokio::test]
    async fn edit_file_tool_single_replacement() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("edit_me.txt"), "foo bar foo baz").unwrap();

        let tool = EditFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "path": "edit_me.txt",
                    "old_text": "foo",
                    "new_text": "qux"
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "ok");

        let content = std::fs::read_to_string(root.join("edit_me.txt")).unwrap();
        // Only first occurrence replaced
        assert_eq!(content, "qux bar foo baz");
    }

    #[tokio::test]
    async fn edit_file_tool_replace_all() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("edit_me.txt"), "foo bar foo baz").unwrap();

        let tool = EditFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "path": "edit_me.txt",
                    "old_text": "foo",
                    "new_text": "qux",
                    "replace_all": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "ok");

        let content = std::fs::read_to_string(root.join("edit_me.txt")).unwrap();
        assert_eq!(content, "qux bar qux baz");
    }

    #[tokio::test]
    async fn edit_file_tool_old_text_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("edit_me.txt"), "hello world").unwrap();

        let tool = EditFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let err = tool
            .execute(
                serde_json::json!({
                    "path": "edit_me.txt",
                    "old_text": "nonexistent",
                    "new_text": "replacement"
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("old_text not found"));
    }

    #[tokio::test]
    async fn edit_file_tool_missing_params() {
        let tool = EditFileTool;
        let ctx = test_ctx();

        // Missing path
        let err = tool
            .execute(
                serde_json::json!({ "old_text": "a", "new_text": "b" }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("missing 'path'"));

        // Missing old_text
        let err = tool
            .execute(
                serde_json::json!({ "path": "file.txt", "new_text": "b" }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("missing 'old_text'"));

        // Missing new_text
        let err = tool
            .execute(
                serde_json::json!({ "path": "file.txt", "old_text": "a" }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("missing 'new_text'"));
    }

    #[tokio::test]
    async fn write_file_tool_append_mode() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("log.txt"), "line 1\n").unwrap();

        let tool = WriteFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "path": "log.txt",
                    "content": "line 2\n",
                    "append": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        assert_eq!(result.output, "ok");
        let meta = result.metadata.unwrap();
        assert_eq!(meta["append"], true);

        let content = std::fs::read_to_string(root.join("log.txt")).unwrap();
        assert_eq!(content, "line 1\nline 2\n");
    }

    #[tokio::test]
    async fn write_file_tool_creates_parent_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();

        let tool = WriteFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        tool.execute(
            serde_json::json!({
                "path": "deep/nested/dir/file.txt",
                "content": "deep content"
            }),
            &ctx,
        )
        .await
        .unwrap();

        let content = std::fs::read_to_string(root.join("deep/nested/dir/file.txt")).unwrap();
        assert_eq!(content, "deep content");
    }

    #[tokio::test]
    async fn search_files_case_sensitive() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(
            root.join("test.txt"),
            "Hello World\nhello world\nHELLO WORLD",
        )
        .unwrap();

        let tool = SearchFilesTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        // Case sensitive should only find exact match
        let result = tool
            .execute(
                serde_json::json!({
                    "query": "Hello World",
                    "path": ".",
                    "case_sensitive": true
                }),
                &ctx,
            )
            .await
            .unwrap();
        let hits: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0]["line"], 1);

        // Case insensitive should find all three
        let result = tool
            .execute(
                serde_json::json!({
                    "query": "Hello World",
                    "path": ".",
                    "case_sensitive": false
                }),
                &ctx,
            )
            .await
            .unwrap();
        let hits: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(hits.len(), 3);
    }

    #[tokio::test]
    async fn search_files_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        let content = (0..50)
            .map(|i| format!("needle line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(root.join("many.txt"), content).unwrap();

        let tool = SearchFilesTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({
                    "query": "needle",
                    "path": ".",
                    "limit": 5
                }),
                &ctx,
            )
            .await
            .unwrap();
        let hits: Vec<Value> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(hits.len(), 5);
    }

    #[tokio::test]
    async fn glob_files_tool_basic() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/main.rs"), "fn main(){}").unwrap();
        std::fs::write(root.join("src/lib.rs"), "// lib").unwrap();
        std::fs::write(root.join("readme.md"), "# readme").unwrap();

        let tool = GlobFilesTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({ "pattern": "src/*.rs", "path": "." }),
                &ctx,
            )
            .await
            .unwrap();
        let matches: Vec<String> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(matches.len(), 2);
        assert!(matches.iter().any(|m| m.contains("main.rs")));
        assert!(matches.iter().any(|m| m.contains("lib.rs")));
    }

    #[tokio::test]
    async fn glob_files_tool_respects_limit() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        for i in 0..10 {
            std::fs::write(root.join(format!("file_{i}.txt")), "content").unwrap();
        }

        let tool = GlobFilesTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let result = tool
            .execute(
                serde_json::json!({ "pattern": "*.txt", "path": ".", "limit": 3 }),
                &ctx,
            )
            .await
            .unwrap();
        let matches: Vec<String> = serde_json::from_str(&result.output).unwrap();
        assert_eq!(matches.len(), 3);
    }

    #[tokio::test]
    async fn read_file_tool_missing_path_param() {
        let tool = ReadFileTool;
        let ctx = test_ctx();
        let err = tool.execute(serde_json::json!({}), &ctx).await.unwrap_err();
        assert!(err.message.contains("missing 'path'"));
    }

    #[tokio::test]
    async fn read_file_tool_oversized_file() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("big.txt"), vec![b'x'; MAX_FILE_BYTES + 1]).unwrap();

        let tool = ReadFileTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        let err = tool
            .execute(serde_json::json!({ "path": "big.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("file too large"));
    }

    #[tokio::test]
    async fn write_file_tool_missing_params() {
        let tool = WriteFileTool;
        let ctx = test_ctx();

        let err = tool
            .execute(serde_json::json!({ "content": "hi" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("missing 'path'"));

        let err = tool
            .execute(serde_json::json!({ "path": "file.txt" }), &ctx)
            .await
            .unwrap_err();
        assert!(err.message.contains("missing 'content'"));
    }

    #[tokio::test]
    async fn list_directory_tool_default_path() {
        let dir = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(dir.path()).unwrap();
        std::fs::write(root.join("a.txt"), "a").unwrap();
        std::fs::create_dir(root.join("subdir")).unwrap();

        let tool = ListDirectoryTool;
        let ctx = ToolContext {
            session_id: "test".into(),
            agent_id: "test".into(),
            authority: InputAuthority::Creator,
            workspace_root: root.clone(),
            channel: None,
            db: None,
        };

        // No path param -- should default to "."
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        assert!(result.output.contains("a.txt"));
        assert!(result.output.contains("subdir"));

        let meta = result.metadata.unwrap();
        assert_eq!(meta["count"], 2);
    }

    #[test]
    fn tool_error_display() {
        let err = ToolError {
            message: "something went wrong".into(),
        };
        let displayed = format!("{err}");
        assert_eq!(displayed, "ToolError: something went wrong");
    }

    #[test]
    fn tool_registry_default() {
        let reg = ToolRegistry::default();
        assert!(reg.list().is_empty());
    }

    #[test]
    fn wildcard_match_exact_filename() {
        assert!(wildcard_match("main.rs", "main.rs"));
        assert!(!wildcard_match("main.rs", "lib.rs"));
    }

    #[test]
    fn wildcard_match_consecutive_double_stars() {
        // Consecutive ** should be collapsed
        assert!(wildcard_match("**/**/*.rs", "src/nested/main.rs"));
        assert!(wildcard_match("src/**/**/*.rs", "src/a/b/c/main.rs"));
    }

    #[test]
    fn wildcard_match_empty_candidate() {
        assert!(!wildcard_match("*.rs", ""));
    }

    #[test]
    fn wildcard_match_double_star_alone() {
        // ** alone should match everything
        assert!(wildcard_match("**", "src/main.rs"));
        assert!(wildcard_match("**", "a/b/c/d/e.txt"));
    }

    #[test]
    fn wildcard_match_question_mark_does_not_cross_directories() {
        assert!(wildcard_match("src/?.rs", "src/a.rs"));
        assert!(!wildcard_match("src/?.rs", "src/ab.rs")); // ? matches exactly one char
    }

    #[test]
    fn wildcard_match_segment_with_star_in_middle() {
        assert!(wildcard_match_segment("foo*bar", "foobazbar"));
        assert!(wildcard_match_segment("foo*bar", "foobar"));
        assert!(!wildcard_match_segment("foo*bar", "foobaz"));
    }

    #[test]
    fn wildcard_match_segment_exact() {
        assert!(wildcard_match_segment("hello", "hello"));
        assert!(!wildcard_match_segment("hello", "helo"));
    }

    #[test]
    fn wildcard_match_segment_question_mark() {
        assert!(wildcard_match_segment("h?llo", "hello"));
        assert!(wildcard_match_segment("h?llo", "hxllo"));
        assert!(!wildcard_match_segment("h?llo", "hlo"));
    }

    #[cfg(unix)]
    #[test]
    fn walk_workspace_files_skips_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join("real.txt"), "real content").unwrap();
        std::os::unix::fs::symlink("/tmp", root.join("symlink_dir")).unwrap();
        std::os::unix::fs::symlink(root.join("real.txt"), root.join("symlink_file")).unwrap();

        let mut files = Vec::new();
        let mut count = 0usize;
        walk_workspace_files(root, &mut files, &mut count).unwrap();
        let names: Vec<String> = files
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert!(names.contains(&"real.txt".to_string()));
        assert!(!names.contains(&"symlink_dir".to_string()));
        assert!(!names.contains(&"symlink_file".to_string()));
    }

    #[test]
    fn edit_file_tool_metadata() {
        assert_eq!(EditFileTool.name(), "edit_file");
        assert_eq!(EditFileTool.risk_level(), RiskLevel::Caution);
        let schema = EditFileTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "path"));
        assert!(required.iter().any(|v| v == "old_text"));
        assert!(required.iter().any(|v| v == "new_text"));
    }

    #[test]
    fn glob_files_tool_metadata() {
        assert_eq!(GlobFilesTool.name(), "glob_files");
        assert_eq!(GlobFilesTool.risk_level(), RiskLevel::Caution);
        let schema = GlobFilesTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "pattern"));
    }

    #[test]
    fn search_files_tool_metadata() {
        assert_eq!(SearchFilesTool.name(), "search_files");
        assert_eq!(SearchFilesTool.risk_level(), RiskLevel::Caution);
        let schema = SearchFilesTool.parameters_schema();
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().any(|v| v == "query"));
    }

    #[test]
    fn script_runner_tool_metadata() {
        let cfg = SkillsConfig::default();
        let tool = ScriptRunnerTool::new(cfg);
        assert_eq!(tool.name(), "run_script");
        assert_eq!(tool.risk_level(), RiskLevel::Caution);
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

    // ── Introspection tool tests ────────────────────────────────────

    #[tokio::test]
    async fn get_runtime_context_returns_all_fields() {
        let tool = GetRuntimeContextTool;
        let mut ctx = test_ctx();
        ctx.channel = Some("telegram".into());

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["agent_id"], "test-agent");
        assert_eq!(parsed["session_id"], "test-session");
        assert_eq!(parsed["channel"], "telegram");
        assert!(parsed["workspace_root"].is_string());
        assert!(result.metadata.is_some());
    }

    #[tokio::test]
    async fn get_runtime_context_no_channel() {
        let tool = GetRuntimeContextTool;
        let ctx = test_ctx();

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["channel"].is_null());
    }

    #[tokio::test]
    async fn get_memory_stats_returns_all_tiers() {
        let tool = GetMemoryStatsTool;
        let ctx = test_ctx();

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        let tiers = &parsed["tiers"];
        assert_eq!(tiers["working"]["budget_pct"], 30);
        assert_eq!(tiers["episodic"]["budget_pct"], 25);
        assert_eq!(tiers["semantic"]["budget_pct"], 20);
        assert_eq!(tiers["procedural"]["budget_pct"], 15);
        assert_eq!(tiers["relationship"]["budget_pct"], 10);
        assert!(
            parsed["retrieval_method"]
                .as_str()
                .unwrap()
                .contains("FTS5")
        );
    }

    #[tokio::test]
    async fn get_channel_health_with_channel() {
        let tool = GetChannelHealthTool;
        let mut ctx = test_ctx();
        ctx.channel = Some("discord".into());

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["channel"], "discord");
        assert_eq!(parsed["status"], "operational");
    }

    #[tokio::test]
    async fn get_channel_health_unknown_channel() {
        let tool = GetChannelHealthTool;
        let ctx = test_ctx();

        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(parsed["channel"], "unknown");
    }

    #[test]
    fn introspection_tools_metadata() {
        let rt = GetRuntimeContextTool;
        assert_eq!(rt.name(), "get_runtime_context");
        assert_eq!(rt.risk_level(), RiskLevel::Safe);

        let ms = GetMemoryStatsTool;
        assert_eq!(ms.name(), "get_memory_stats");
        assert_eq!(ms.risk_level(), RiskLevel::Safe);

        let ch = GetChannelHealthTool;
        assert_eq!(ch.name(), "get_channel_health");
        assert_eq!(ch.risk_level(), RiskLevel::Safe);

        let sa = GetSubagentStatusTool;
        assert_eq!(sa.name(), "get_subagent_status");
        assert_eq!(sa.risk_level(), RiskLevel::Safe);
    }

    #[tokio::test]
    async fn get_subagent_status_without_db_returns_empty() {
        let tool = GetSubagentStatusTool;
        let ctx = test_ctx(); // db: None
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["subagents"], serde_json::json!([]));
        assert_eq!(v["tasks"], serde_json::json!([]));
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains("database not available")
        );
    }

    #[tokio::test]
    async fn get_subagent_status_with_db_returns_agents_and_tasks() {
        let db = ironclad_db::Database::new(":memory:").unwrap();

        // Insert a subagent
        ironclad_db::agents::upsert_sub_agent(
            &db,
            &ironclad_db::agents::SubAgentRow {
                id: "sa-1".into(),
                name: "code-reviewer".into(),
                display_name: Some("Code Reviewer".into()),
                model: "gpt-4o".into(),
                role: "specialist".into(),
                description: Some("Reviews code".into()),
                skills_json: None,
                enabled: true,
                session_count: 3,
            },
        )
        .unwrap();

        // Insert some tasks
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO tasks (id, title, status, priority) VALUES ('t1', 'Fix bug', 'pending', 2)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO tasks (id, title, status, priority) VALUES ('t2', 'Write docs', 'in_progress', 1)",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO tasks (id, title, status, priority) VALUES ('t3', 'Done task', 'completed', 0)",
                [],
            ).unwrap();
        }

        let ctx = ToolContext {
            session_id: "test-session".into(),
            agent_id: "test-agent".into(),
            authority: InputAuthority::Creator,
            workspace_root: std::env::current_dir().unwrap(),
            channel: None,
            db: Some(db),
        };

        let tool = GetSubagentStatusTool;
        let result = tool.execute(serde_json::json!({}), &ctx).await.unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();

        // Should have 1 subagent
        assert_eq!(v["subagent_count"], 1);
        assert_eq!(v["subagents"][0]["name"], "code-reviewer");
        assert_eq!(v["subagents"][0]["enabled"], true);

        // Should have 2 open tasks (pending + in_progress), not the completed one
        assert_eq!(v["open_task_count"], 2);
        // Priority DESC, so "Fix bug" (priority 2) comes first
        assert_eq!(v["tasks"][0]["title"], "Fix bug");
        assert_eq!(v["tasks"][1]["title"], "Write docs");
    }

    // ── Data tools tests ───────────────────────────────────────────────

    fn test_ctx_with_db() -> ToolContext {
        let db = ironclad_db::Database::new(":memory:").expect("in-memory db");
        ToolContext {
            session_id: "test-session".into(),
            agent_id: "testagent".into(), // no hyphens — SQL identifiers
            authority: InputAuthority::Creator,
            workspace_root: std::env::current_dir().unwrap(),
            channel: None,
            db: Some(db),
        }
    }

    #[tokio::test]
    async fn create_table_basic() {
        let ctx = test_ctx_with_db();
        let tool = CreateTableTool;
        let params = serde_json::json!({
            "name": "notes",
            "description": "Agent scratchpad",
            "columns": [
                {"name": "title", "type": "TEXT"},
                {"name": "body", "type": "TEXT"},
            ]
        });
        let result = tool.execute(params, &ctx).await.unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["table_name"], "testagent_notes");
        assert_eq!(v["columns_created"], 2);
    }

    #[tokio::test]
    async fn create_table_rejects_reserved_column() {
        let ctx = test_ctx_with_db();
        let tool = CreateTableTool;
        let params = serde_json::json!({
            "name": "bad",
            "description": "test",
            "columns": [{"name": "rowid", "type": "INTEGER"}]
        });
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.message.contains("reserved"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn create_table_rejects_invalid_type() {
        let ctx = test_ctx_with_db();
        let tool = CreateTableTool;
        let params = serde_json::json!({
            "name": "bad",
            "description": "test",
            "columns": [{"name": "val", "type": "JSON"}]
        });
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.message.contains("type"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn create_table_enforces_max_columns() {
        let ctx = test_ctx_with_db();
        let tool = CreateTableTool;
        let cols: Vec<Value> = (0..MAX_COLUMNS_PER_TABLE + 1)
            .map(|i| serde_json::json!({"name": format!("c{i}"), "type": "TEXT"}))
            .collect();
        let params = serde_json::json!({
            "name": "wide",
            "description": "too many columns",
            "columns": cols
        });
        let err = tool.execute(params, &ctx).await.unwrap_err();
        assert!(err.message.contains("columns"), "got: {}", err.message);
    }

    #[tokio::test]
    async fn alter_table_add_and_drop_column() {
        let ctx = test_ctx_with_db();
        // First create a table
        CreateTableTool
            .execute(
                serde_json::json!({
                    "name": "tasks",
                    "description": "task list",
                    "columns": [{"name": "title", "type": "TEXT"}]
                }),
                &ctx,
            )
            .await
            .unwrap();

        let alter = AlterTableTool;
        // Add a column
        let result = alter
            .execute(
                serde_json::json!({
                    "table_name": "testagent_tasks",
                    "operation": "add_column",
                    "column": {"name": "priority", "type": "INTEGER"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["operation"], "add_column");
        assert_eq!(v["column_name"], "priority");

        // Drop a column
        let result = alter
            .execute(
                serde_json::json!({
                    "table_name": "testagent_tasks",
                    "operation": "drop_column",
                    "column": {"name": "priority"}
                }),
                &ctx,
            )
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["operation"], "drop_column");
        assert_eq!(v["column_name"], "priority");
    }

    #[tokio::test]
    async fn alter_table_rejects_non_owned_table() {
        let ctx = test_ctx_with_db();
        let alter = AlterTableTool;
        let err = alter
            .execute(
                serde_json::json!({
                    "table_name": "sessions",
                    "operation": "add_column",
                    "column": {"name": "hack", "type": "TEXT"}
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(
            err.message.contains("not owned") || err.message.contains("not found"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn drop_table_basic() {
        let ctx = test_ctx_with_db();
        CreateTableTool
            .execute(
                serde_json::json!({
                    "name": "temp",
                    "description": "throwaway",
                    "columns": [{"name": "data", "type": "BLOB"}]
                }),
                &ctx,
            )
            .await
            .unwrap();

        let drop = DropTableTool;
        let result = drop
            .execute(serde_json::json!({"table_name": "testagent_temp"}), &ctx)
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&result.output).unwrap();
        assert_eq!(v["status"], "dropped");
    }

    #[tokio::test]
    async fn drop_table_rejects_system_table() {
        let ctx = test_ctx_with_db();
        let drop = DropTableTool;
        let err = drop
            .execute(serde_json::json!({"table_name": "sessions"}), &ctx)
            .await
            .unwrap_err();
        assert!(
            err.message.contains("not owned") || err.message.contains("drop"),
            "got: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn data_tools_require_db() {
        let ctx = test_ctx(); // no db
        let err = CreateTableTool
            .execute(
                serde_json::json!({
                    "name": "x",
                    "description": "y",
                    "columns": [{"name": "a", "type": "TEXT"}]
                }),
                &ctx,
            )
            .await
            .unwrap_err();
        assert!(err.message.contains("database"), "got: {}", err.message);
    }
}
