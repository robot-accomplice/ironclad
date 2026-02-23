use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::info;

/// Represents the structured workspace context for an agent.
#[derive(Debug, Clone)]
pub struct WorkspaceContext {
    pub root: PathBuf,
    pub manifest: Option<WorkspaceManifest>,
    pub file_index: HashMap<String, FileEntry>,
}

/// A TOML-parsed workspace manifest (workspace.toml).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkspaceManifest {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub schemas: Vec<SchemaRef>,
}

/// Reference to a data schema file.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SchemaRef {
    pub name: String,
    pub path: String,
}

/// An indexed file in the workspace.
#[derive(Debug, Clone)]
pub struct FileEntry {
    pub path: PathBuf,
    pub category: FileCategory,
    pub size_bytes: u64,
}

/// Categories of files in the workspace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileCategory {
    Personality,
    Config,
    Schema,
    Document,
    Data,
    Unknown,
}

impl FileCategory {
    pub fn from_path(path: &Path) -> Self {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        match name {
            "SOUL.md" | "FIRMWARE.md" | "OPERATOR.md" | "DIRECTIVES.md" => {
                FileCategory::Personality
            }
            "workspace.toml" | "config.toml" => FileCategory::Config,
            _ => match ext {
                "toml" | "yaml" | "yml" => FileCategory::Schema,
                "md" | "txt" | "rst" => FileCategory::Document,
                "json" | "csv" | "sqlite" | "db" => FileCategory::Data,
                _ => FileCategory::Unknown,
            },
        }
    }
}

impl WorkspaceContext {
    /// Load workspace context from a directory.
    pub fn from_path(root: &Path) -> Self {
        let manifest = Self::load_manifest(root);
        let file_index = Self::index_files(root);

        info!(
            root = %root.display(),
            files = file_index.len(),
            has_manifest = manifest.is_some(),
            "loaded workspace context"
        );

        Self {
            root: root.to_path_buf(),
            manifest,
            file_index,
        }
    }

    fn load_manifest(root: &Path) -> Option<WorkspaceManifest> {
        let manifest_path = root.join("workspace.toml");
        if !manifest_path.exists() {
            return None;
        }
        let content = std::fs::read_to_string(&manifest_path).ok()?;
        toml::from_str(&content).ok()
    }

    fn index_files(root: &Path) -> HashMap<String, FileEntry> {
        let mut index = HashMap::new();
        let Ok(entries) = std::fs::read_dir(root) else {
            return index;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_file() {
                let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                let key = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();
                let category = FileCategory::from_path(&path);
                index.insert(
                    key,
                    FileEntry {
                        path: path.clone(),
                        category,
                        size_bytes: size,
                    },
                );
            }
        }
        index
    }

    /// Generate a summary of the workspace for agent context.
    pub fn summary(&self) -> String {
        let mut parts = Vec::new();
        parts.push(format!("Workspace: {}", self.root.display()));

        if let Some(ref manifest) = self.manifest {
            if !manifest.name.is_empty() {
                parts.push(format!("Name: {}", manifest.name));
            }
            if !manifest.description.is_empty() {
                parts.push(format!("Description: {}", manifest.description));
            }
        }

        let personality_count = self.files_by_category(FileCategory::Personality).len();
        let config_count = self.files_by_category(FileCategory::Config).len();
        let doc_count = self.files_by_category(FileCategory::Document).len();
        let data_count = self.files_by_category(FileCategory::Data).len();

        parts.push(format!(
            "Files: {} personality, {} config, {} documents, {} data",
            personality_count, config_count, doc_count, data_count
        ));

        parts.join("\n")
    }

    /// Get files by category.
    pub fn files_by_category(&self, category: FileCategory) -> Vec<&FileEntry> {
        self.file_index
            .values()
            .filter(|f| f.category == category)
            .collect()
    }

    /// Check if a specific personality file exists.
    pub fn has_personality_file(&self, name: &str) -> bool {
        self.file_index
            .get(name)
            .map(|f| f.category == FileCategory::Personality)
            .unwrap_or(false)
    }

    /// Get the total workspace size in bytes.
    pub fn total_size(&self) -> u64 {
        self.file_index.values().map(|f| f.size_bytes).sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn file_category_personality() {
        assert_eq!(
            FileCategory::from_path(Path::new("SOUL.md")),
            FileCategory::Personality
        );
        assert_eq!(
            FileCategory::from_path(Path::new("FIRMWARE.md")),
            FileCategory::Personality
        );
    }

    #[test]
    fn file_category_config() {
        assert_eq!(
            FileCategory::from_path(Path::new("workspace.toml")),
            FileCategory::Config
        );
        assert_eq!(
            FileCategory::from_path(Path::new("config.toml")),
            FileCategory::Config
        );
    }

    #[test]
    fn file_category_document() {
        assert_eq!(
            FileCategory::from_path(Path::new("README.md")),
            FileCategory::Document
        );
        assert_eq!(
            FileCategory::from_path(Path::new("notes.txt")),
            FileCategory::Document
        );
    }

    #[test]
    fn file_category_schema() {
        assert_eq!(
            FileCategory::from_path(Path::new("schema.yaml")),
            FileCategory::Schema
        );
    }

    #[test]
    fn file_category_data() {
        assert_eq!(
            FileCategory::from_path(Path::new("export.json")),
            FileCategory::Data
        );
        assert_eq!(
            FileCategory::from_path(Path::new("records.csv")),
            FileCategory::Data
        );
    }

    #[test]
    fn workspace_from_nonexistent_path() {
        let ctx = WorkspaceContext::from_path(Path::new("/nonexistent/workspace"));
        assert!(ctx.manifest.is_none());
        assert!(ctx.file_index.is_empty());
    }

    #[test]
    fn workspace_from_empty_dir() {
        let dir = TempDir::new().unwrap();
        let ctx = WorkspaceContext::from_path(dir.path());
        assert!(ctx.manifest.is_none());
        assert!(ctx.file_index.is_empty());
    }

    #[test]
    fn workspace_indexes_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("SOUL.md"), "# Identity").unwrap();
        fs::write(dir.path().join("notes.txt"), "Some notes").unwrap();

        let ctx = WorkspaceContext::from_path(dir.path());
        assert_eq!(ctx.file_index.len(), 2);
        assert!(ctx.has_personality_file("SOUL.md"));
    }

    #[test]
    fn workspace_loads_manifest() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("workspace.toml"),
            r#"
name = "TestWorkspace"
description = "A test workspace"
"#,
        )
        .unwrap();

        let ctx = WorkspaceContext::from_path(dir.path());
        let manifest = ctx.manifest.as_ref().unwrap();
        assert_eq!(manifest.name, "TestWorkspace");
        assert_eq!(manifest.description, "A test workspace");
    }

    #[test]
    fn workspace_summary_contains_info() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("SOUL.md"), "# Soul").unwrap();
        fs::write(dir.path().join("config.toml"), "key = 'val'").unwrap();

        let ctx = WorkspaceContext::from_path(dir.path());
        let summary = ctx.summary();
        assert!(summary.contains("1 personality"));
        assert!(summary.contains("1 config"));
    }

    #[test]
    fn workspace_total_size() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file.md"), "hello").unwrap();

        let ctx = WorkspaceContext::from_path(dir.path());
        assert!(ctx.total_size() > 0);
    }

    #[test]
    fn files_by_category_filters() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("SOUL.md"), "soul").unwrap();
        fs::write(dir.path().join("README.md"), "readme").unwrap();
        fs::write(dir.path().join("data.json"), "{}").unwrap();

        let ctx = WorkspaceContext::from_path(dir.path());
        assert_eq!(ctx.files_by_category(FileCategory::Personality).len(), 1);
        assert_eq!(ctx.files_by_category(FileCategory::Document).len(), 1);
        assert_eq!(ctx.files_by_category(FileCategory::Data).len(), 1);
    }
}
