use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::RwLock;

use ironclad_core::RiskLevel;

use crate::obsidian::ObsidianVault;
use crate::tools::{Tool, ToolContext, ToolError, ToolResult};

// ---------------------------------------------------------------------------
// ObsidianReadTool
// ---------------------------------------------------------------------------

pub struct ObsidianReadTool {
    vault: Arc<RwLock<ObsidianVault>>,
}

impl ObsidianReadTool {
    pub fn new(vault: Arc<RwLock<ObsidianVault>>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl Tool for ObsidianReadTool {
    fn name(&self) -> &str {
        "obsidian_read"
    }

    fn description(&self) -> &str {
        "Read a note from the user's Obsidian vault by path or title. \
         Returns the note content with frontmatter metadata, tags, and backlink count."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path to the note within the vault (e.g. 'folder/note.md')"
                },
                "title": {
                    "type": "string",
                    "description": "Note title to search for (case-insensitive wikilink resolution)"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let vault = self.vault.read().await;

        let note = if let Some(path) = params.get("path").and_then(|v| v.as_str()) {
            vault.get_note(path).cloned()
        } else if let Some(title) = params.get("title").and_then(|v| v.as_str()) {
            vault
                .resolve_wikilink(title)
                .and_then(|p| vault.get_note(&p.to_string_lossy()).cloned())
        } else {
            return Err(ToolError {
                message: "either 'path' or 'title' parameter is required".into(),
            });
        };

        match note {
            Some(note) => {
                let rel_path = note
                    .path
                    .strip_prefix(&vault.root)
                    .unwrap_or(&note.path)
                    .to_string_lossy()
                    .to_string();

                let backlink_count = vault.backlinks_for(&rel_path).len();
                let uri = vault.obsidian_uri(&rel_path);

                let metadata = serde_json::json!({
                    "path": rel_path,
                    "title": note.title,
                    "tags": note.tags,
                    "backlink_count": backlink_count,
                    "obsidian_uri": uri,
                    "frontmatter": note.frontmatter,
                    "created_at": note.created_at,
                    "modified_at": note.modified_at,
                });

                Ok(ToolResult {
                    output: note.content,
                    metadata: Some(metadata),
                })
            }
            None => Err(ToolError {
                message: "note not found in vault".into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// ObsidianWriteTool
// ---------------------------------------------------------------------------

pub struct ObsidianWriteTool {
    vault: Arc<RwLock<ObsidianVault>>,
}

impl ObsidianWriteTool {
    pub fn new(vault: Arc<RwLock<ObsidianVault>>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl Tool for ObsidianWriteTool {
    fn name(&self) -> &str {
        "obsidian_write"
    }

    fn description(&self) -> &str {
        "Write a document to the user's Obsidian vault. This is the preferred destination \
         for producing documents, reports, notes, and any persistent written output. \
         Returns the file path and an obsidian:// URI the user can click to open it."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Caution
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Relative path for the note (e.g. 'projects/report.md'). \
                                    If no folder prefix, writes to the default agent folder."
                },
                "content": {
                    "type": "string",
                    "description": "Markdown content for the note"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags to include in YAML frontmatter"
                },
                "template": {
                    "type": "string",
                    "description": "Name of an Obsidian template to apply before writing"
                },
                "frontmatter": {
                    "type": "object",
                    "description": "Additional YAML frontmatter fields"
                }
            },
            "required": ["path", "content"]
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

        let content = params
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| ToolError {
                message: "missing 'content' parameter".into(),
            })?;

        let mut vault = self.vault.write().await;

        // Apply template if specified
        let final_content =
            if let Some(template_name) = params.get("template").and_then(|v| v.as_str()) {
                let mut vars = HashMap::new();
                vars.insert("title".into(), path_to_title(path));
                vars.insert("content".into(), content.to_string());

                match vault.apply_template(template_name, &vars) {
                    Ok(rendered) => rendered,
                    Err(e) => {
                        return Err(ToolError {
                            message: format!("template error: {e}"),
                        });
                    }
                }
            } else {
                content.to_string()
            };

        // Build frontmatter
        let fm = {
            let mut obj = if let Some(Value::Object(m)) = params.get("frontmatter") {
                serde_json::Value::Object(m.clone())
            } else {
                serde_json::json!({})
            };

            if let Some(Value::Array(arr)) = params.get("tags")
                && let Some(map) = obj.as_object_mut()
            {
                map.insert("tags".into(), Value::Array(arr.clone()));
            }

            Some(obj)
        };

        match vault.write_note(path, &final_content, fm) {
            Ok(abs_path) => {
                let rel = abs_path
                    .strip_prefix(&vault.root)
                    .unwrap_or(&abs_path)
                    .to_string_lossy()
                    .to_string();
                let uri = vault.obsidian_uri(&rel);

                Ok(ToolResult {
                    output: format!("Note written to {rel}\n\nOpen in Obsidian: {uri}"),
                    metadata: Some(serde_json::json!({
                        "path": rel,
                        "absolute_path": abs_path.display().to_string(),
                        "obsidian_uri": uri,
                    })),
                })
            }
            Err(e) => Err(ToolError {
                message: format!("failed to write note: {e}"),
            }),
        }
    }
}

fn path_to_title(path: &str) -> String {
    std::path::Path::new(path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(path)
        .to_string()
}

// ---------------------------------------------------------------------------
// ObsidianSearchTool
// ---------------------------------------------------------------------------

pub struct ObsidianSearchTool {
    vault: Arc<RwLock<ObsidianVault>>,
}

impl ObsidianSearchTool {
    pub fn new(vault: Arc<RwLock<ObsidianVault>>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl Tool for ObsidianSearchTool {
    fn name(&self) -> &str {
        "obsidian_search"
    }

    fn description(&self) -> &str {
        "Search the user's Obsidian vault by content query, tags, or folder. \
         Returns matching notes with titles, paths, tags, and relevance scores."
    }

    fn risk_level(&self) -> RiskLevel {
        RiskLevel::Safe
    }

    fn parameters_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Full-text search query"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Filter by tags (notes must have at least one matching tag)"
                },
                "folder": {
                    "type": "string",
                    "description": "Restrict search to a specific folder within the vault"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results (default 10)"
                }
            }
        })
    }

    async fn execute(
        &self,
        params: Value,
        _ctx: &ToolContext,
    ) -> std::result::Result<ToolResult, ToolError> {
        let vault = self.vault.read().await;

        let limit = params.get("limit").and_then(|v| v.as_u64()).unwrap_or(10) as usize;

        let query = params.get("query").and_then(|v| v.as_str());
        let tags: Vec<String> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();
        let folder = params.get("folder").and_then(|v| v.as_str());

        if query.is_none() && tags.is_empty() && folder.is_none() {
            return Err(ToolError {
                message: "at least one of 'query', 'tags', or 'folder' is required".into(),
            });
        }

        let mut results: Vec<Value> = Vec::new();

        if let Some(q) = query {
            let search_results = vault.search_by_content(q, limit);
            for (key, note, score) in search_results {
                if let Some(f) = folder
                    && !key.starts_with(f)
                {
                    continue;
                }

                if !tags.is_empty()
                    && !tags.iter().any(|t| {
                        note.tags
                            .iter()
                            .any(|nt| nt.to_lowercase() == t.to_lowercase())
                    })
                {
                    continue;
                }

                results.push(serde_json::json!({
                    "path": key,
                    "title": note.title,
                    "tags": note.tags,
                    "relevance": score,
                    "obsidian_uri": vault.obsidian_uri(key),
                    "preview": truncate_content(&note.content, 200),
                }));

                if results.len() >= limit {
                    break;
                }
            }
        } else {
            // Tag-only or folder-only search
            let mut matching: Vec<(&str, &crate::obsidian::ObsidianNote)> = if !tags.is_empty() {
                let tag_results: Vec<_> = tags
                    .iter()
                    .flat_map(|t| {
                        vault
                            .search_by_tag(t)
                            .into_iter()
                            .map(|n| n.title.clone())
                            .collect::<Vec<_>>()
                    })
                    .collect();

                vault
                    .notes_in_folder(folder.unwrap_or(""))
                    .into_iter()
                    .filter(|(_, n)| tag_results.contains(&n.title))
                    .collect()
            } else if let Some(f) = folder {
                vault.notes_in_folder(f)
            } else {
                Vec::new()
            };

            matching.truncate(limit);

            for (key, note) in matching {
                results.push(serde_json::json!({
                    "path": key,
                    "title": note.title,
                    "tags": note.tags,
                    "obsidian_uri": vault.obsidian_uri(key),
                    "preview": truncate_content(&note.content, 200),
                }));
            }
        }

        let output = serde_json::to_string_pretty(&serde_json::json!({
            "count": results.len(),
            "results": results,
        }))
        .unwrap_or_else(|_| "[]".into());

        Ok(ToolResult {
            output,
            metadata: Some(serde_json::json!({ "result_count": results.len() })),
        })
    }
}

fn truncate_content(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s.floor_char_boundary(max);
        format!("{}...", &s[..boundary])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::obsidian::ObsidianVault;
    use ironclad_core::InputAuthority;
    use ironclad_core::config::ObsidianConfig;
    use std::fs;
    use tempfile::TempDir;

    fn test_ctx() -> ToolContext {
        ToolContext {
            session_id: "test-session".into(),
            agent_id: "test-agent".into(),
            authority: InputAuthority::Creator,
        }
    }

    fn setup_vault() -> (TempDir, Arc<RwLock<ObsidianVault>>) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".obsidian")).unwrap();
        fs::create_dir(dir.path().join("ironclad")).unwrap();
        fs::write(
            dir.path().join("existing.md"),
            "---\ntags:\n  - test\n---\n\nExisting note content about Rust",
        )
        .unwrap();

        let config = ObsidianConfig {
            enabled: true,
            vault_path: Some(dir.path().to_path_buf()),
            index_on_start: true,
            ..Default::default()
        };

        let vault = ObsidianVault::from_config(&config).unwrap();
        (dir, Arc::new(RwLock::new(vault)))
    }

    #[tokio::test]
    async fn read_tool_by_path() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianReadTool::new(vault);
        let ctx = test_ctx();

        let result = tool
            .execute(serde_json::json!({ "path": "existing.md" }), &ctx)
            .await
            .unwrap();

        assert!(result.output.contains("Existing note content"));
        let meta = result.metadata.unwrap();
        assert_eq!(meta["title"], "existing");
    }

    #[tokio::test]
    async fn read_tool_by_title() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianReadTool::new(vault);
        let ctx = test_ctx();

        let result = tool
            .execute(serde_json::json!({ "title": "existing" }), &ctx)
            .await
            .unwrap();

        assert!(result.output.contains("Existing note content"));
    }

    #[tokio::test]
    async fn read_tool_not_found() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianReadTool::new(vault);
        let ctx = test_ctx();

        let err = tool
            .execute(serde_json::json!({ "path": "nonexistent.md" }), &ctx)
            .await
            .unwrap_err();

        assert!(err.message.contains("not found"));
    }

    #[tokio::test]
    async fn read_tool_missing_params() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianReadTool::new(vault);
        let ctx = test_ctx();

        let err = tool.execute(serde_json::json!({}), &ctx).await.unwrap_err();

        assert!(err.message.contains("required"));
    }

    #[tokio::test]
    async fn write_tool_creates_note() {
        let (dir, vault) = setup_vault();
        let tool = ObsidianWriteTool::new(vault);
        let ctx = test_ctx();

        let result = tool
            .execute(
                serde_json::json!({
                    "path": "new-note",
                    "content": "Hello from the write tool",
                    "tags": ["test", "automated"]
                }),
                &ctx,
            )
            .await
            .unwrap();

        assert!(result.output.contains("Note written to"));
        assert!(result.output.contains("obsidian://"));

        let meta = result.metadata.unwrap();
        assert!(
            meta["obsidian_uri"]
                .as_str()
                .unwrap()
                .starts_with("obsidian://")
        );

        let written = dir.path().join("ironclad/new-note.md");
        assert!(written.exists());
        let content = fs::read_to_string(&written).unwrap();
        assert!(content.contains("Hello from the write tool"));
        assert!(content.contains("created_by"));
    }

    #[tokio::test]
    async fn write_tool_missing_content() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianWriteTool::new(vault);
        let ctx = test_ctx();

        let err = tool
            .execute(serde_json::json!({ "path": "test" }), &ctx)
            .await
            .unwrap_err();

        assert!(err.message.contains("content"));
    }

    #[tokio::test]
    async fn search_tool_by_query() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianSearchTool::new(vault);
        let ctx = test_ctx();

        let result = tool
            .execute(serde_json::json!({ "query": "Rust" }), &ctx)
            .await
            .unwrap();

        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn search_tool_by_tag() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianSearchTool::new(vault);
        let ctx = test_ctx();

        let result = tool
            .execute(serde_json::json!({ "tags": ["test"] }), &ctx)
            .await
            .unwrap();

        let parsed: Value = serde_json::from_str(&result.output).unwrap();
        assert!(parsed["count"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn search_tool_no_params() {
        let (_dir, vault) = setup_vault();
        let tool = ObsidianSearchTool::new(vault);
        let ctx = test_ctx();

        let err = tool.execute(serde_json::json!({}), &ctx).await.unwrap_err();

        assert!(err.message.contains("required"));
    }
}
