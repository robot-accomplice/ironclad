use ironclad_core::{IroncladError, ModelTier, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};

/// Declarative agent manifest loaded from TOML files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub personality: PersonalitySpec,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub model_tier: Option<ModelTier>,
    #[serde(default)]
    pub tool_whitelist: Vec<String>,
    #[serde(default)]
    pub memory_budget_mb: Option<u64>,
    #[serde(default)]
    pub cron: Option<String>,
    #[serde(default)]
    pub max_concurrent_sessions: Option<usize>,
}

/// Personality fields for a specialist agent.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PersonalitySpec {
    #[serde(default)]
    pub role: String,
    #[serde(default)]
    pub style: String,
    #[serde(default)]
    pub directives: Vec<String>,
}

/// Loaded manifest with metadata for hot-reload detection.
#[derive(Debug, Clone)]
pub struct LoadedManifest {
    pub manifest: AgentManifest,
    pub path: PathBuf,
    pub content_hash: String,
}

/// Loads and manages agent manifests from a directory.
pub struct ManifestLoader {
    manifests: HashMap<String, LoadedManifest>,
    directory: PathBuf,
}

impl ManifestLoader {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            manifests: HashMap::new(),
            directory,
        }
    }

    /// Load all manifest files from the configured directory.
    pub fn load_all(&mut self) -> Result<usize> {
        if !self.directory.exists() {
            info!(dir = %self.directory.display(), "manifests directory does not exist, skipping");
            return Ok(0);
        }

        let entries = std::fs::read_dir(&self.directory)
            .map_err(|e| IroncladError::Config(format!("failed to read manifests dir: {e}")))?;

        let mut count = 0;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("toml") {
                match self.load_manifest(&path) {
                    Ok(_) => count += 1,
                    Err(e) => warn!(path = %path.display(), error = %e, "failed to load manifest"),
                }
            }
        }

        info!(count, dir = %self.directory.display(), "loaded agent manifests");
        Ok(count)
    }

    fn load_manifest(&mut self, path: &Path) -> Result<()> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            IroncladError::Config(format!("failed to read manifest {}: {e}", path.display()))
        })?;

        let hash = compute_hash(&content);

        let manifest: AgentManifest = toml::from_str(&content).map_err(|e| {
            IroncladError::Config(format!("invalid manifest {}: {e}", path.display()))
        })?;

        self.validate(&manifest)?;

        debug!(id = %manifest.id, name = %manifest.name, "loaded manifest");

        self.manifests.insert(
            manifest.id.clone(),
            LoadedManifest {
                manifest,
                path: path.to_path_buf(),
                content_hash: hash,
            },
        );

        Ok(())
    }

    fn validate(&self, manifest: &AgentManifest) -> Result<()> {
        if manifest.id.is_empty() {
            return Err(IroncladError::Config("manifest id cannot be empty".into()));
        }
        if manifest.name.is_empty() {
            return Err(IroncladError::Config(
                "manifest name cannot be empty".into(),
            ));
        }
        if manifest.id.contains(' ') || manifest.id.contains('/') {
            return Err(IroncladError::Config(format!(
                "manifest id '{}' contains invalid characters",
                manifest.id
            )));
        }
        Ok(())
    }

    /// Check for changes and reload modified manifests.
    /// Returns the IDs of manifests that were reloaded.
    pub fn check_for_changes(&mut self) -> Vec<String> {
        let mut changed = Vec::new();

        if !self.directory.exists() {
            return changed;
        }

        let entries: Vec<_> = std::fs::read_dir(&self.directory)
            .into_iter()
            .flat_map(|rd| rd.flatten())
            .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("toml"))
            .collect();

        let disk_paths: std::collections::HashSet<PathBuf> =
            entries.iter().map(|e| e.path()).collect();

        for entry in entries {
            let path = entry.path();
            if let Ok(content) = std::fs::read_to_string(&path) {
                let hash = compute_hash(&content);

                let needs_reload = self
                    .manifests
                    .values()
                    .find(|lm| lm.path == path)
                    .map(|lm| lm.content_hash != hash)
                    .unwrap_or(true);

                if needs_reload
                    && let Ok(manifest) = toml::from_str::<AgentManifest>(&content)
                    && self.validate(&manifest).is_ok()
                {
                    let id = manifest.id.clone();
                    self.manifests.insert(
                        id.clone(),
                        LoadedManifest {
                            manifest,
                            path: path.clone(),
                            content_hash: hash,
                        },
                    );
                    changed.push(id);
                }
            }
        }

        self.manifests.retain(|_, lm| disk_paths.contains(&lm.path));

        if !changed.is_empty() {
            info!(changed = ?changed, "reloaded manifests");
        }

        changed
    }

    /// Get a loaded manifest by ID.
    pub fn get(&self, id: &str) -> Option<&AgentManifest> {
        self.manifests.get(id).map(|lm| &lm.manifest)
    }

    /// List all loaded manifests.
    pub fn list(&self) -> Vec<&AgentManifest> {
        self.manifests.values().map(|lm| &lm.manifest).collect()
    }

    /// Find manifests that declare a specific capability.
    pub fn find_by_capability(&self, capability: &str) -> Vec<&AgentManifest> {
        self.manifests
            .values()
            .filter(|lm| lm.manifest.capabilities.iter().any(|c| c == capability))
            .map(|lm| &lm.manifest)
            .collect()
    }

    pub fn count(&self) -> usize {
        self.manifests.len()
    }

    pub fn directory(&self) -> &Path {
        &self.directory
    }
}

fn compute_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn sample_manifest() -> &'static str {
        r#"
id = "morning-briefing"
name = "Morning Briefing Agent"
description = "Produces daily morning summaries"
capabilities = ["summarization", "scheduling"]
model_tier = "T2"
tool_whitelist = ["memory_search", "http_get"]
memory_budget_mb = 64
cron = "0 7 * * *"

[personality]
role = "daily briefing specialist"
style = "concise and professional"
directives = ["Focus on actionable items", "Prioritize by urgency"]
"#
    }

    #[test]
    fn parse_manifest() {
        let manifest: AgentManifest = toml::from_str(sample_manifest()).unwrap();
        assert_eq!(manifest.id, "morning-briefing");
        assert_eq!(manifest.name, "Morning Briefing Agent");
        assert_eq!(manifest.capabilities, vec!["summarization", "scheduling"]);
        assert_eq!(manifest.model_tier, Some(ModelTier::T2));
        assert_eq!(manifest.tool_whitelist, vec!["memory_search", "http_get"]);
        assert_eq!(manifest.memory_budget_mb, Some(64));
        assert_eq!(manifest.cron, Some("0 7 * * *".to_string()));
    }

    #[test]
    fn parse_minimal_manifest() {
        let toml_str = r#"
id = "simple"
name = "Simple Agent"
"#;
        let manifest: AgentManifest = toml::from_str(toml_str).unwrap();
        assert_eq!(manifest.id, "simple");
        assert!(manifest.capabilities.is_empty());
        assert!(manifest.model_tier.is_none());
        assert!(manifest.cron.is_none());
    }

    #[test]
    fn load_from_directory() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("briefing.toml"), sample_manifest()).unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 1);
        assert_eq!(loader.count(), 1);

        let manifest = loader.get("morning-briefing").unwrap();
        assert_eq!(manifest.name, "Morning Briefing Agent");
    }

    #[test]
    fn load_nonexistent_directory() {
        let mut loader = ManifestLoader::new(PathBuf::from("/nonexistent/agents"));
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn reject_invalid_manifest() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("bad.toml"), "not valid toml {{{{").unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn reject_empty_id() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("empty.toml"), "id = \"\"\nname = \"No ID\"").unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn reject_invalid_id_characters() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("bad_id.toml"),
            "id = \"has spaces\"\nname = \"Bad\"",
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn find_by_capability() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.toml"),
            r#"
id = "agent-a"
name = "Agent A"
capabilities = ["summarization", "coding"]
"#,
        )
        .unwrap();
        fs::write(
            dir.path().join("b.toml"),
            r#"
id = "agent-b"
name = "Agent B"
capabilities = ["coding", "research"]
"#,
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();

        let coders = loader.find_by_capability("coding");
        assert_eq!(coders.len(), 2);

        let summarizers = loader.find_by_capability("summarization");
        assert_eq!(summarizers.len(), 1);
        assert_eq!(summarizers[0].id, "agent-a");
    }

    #[test]
    fn hot_reload_detects_changes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("agent.toml");
        fs::write(
            &path,
            r#"
id = "hot"
name = "Original"
"#,
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert_eq!(loader.get("hot").unwrap().name, "Original");

        fs::write(
            &path,
            r#"
id = "hot"
name = "Updated"
"#,
        )
        .unwrap();

        let changed = loader.check_for_changes();
        assert!(changed.contains(&"hot".to_string()));
        assert_eq!(loader.get("hot").unwrap().name, "Updated");
    }

    #[test]
    fn no_changes_returns_empty() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("stable.toml"),
            r#"
id = "stable"
name = "Stable Agent"
"#,
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();

        let changed = loader.check_for_changes();
        assert!(changed.is_empty());
    }

    #[test]
    fn compute_hash_deterministic() {
        let h1 = compute_hash("hello world");
        let h2 = compute_hash("hello world");
        let h3 = compute_hash("different");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn personality_spec_defaults() {
        let spec = PersonalitySpec::default();
        assert!(spec.role.is_empty());
        assert!(spec.style.is_empty());
        assert!(spec.directives.is_empty());
    }

    #[test]
    fn check_for_changes_file_deleted() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("ephemeral.toml");
        fs::write(
            &path,
            r#"
id = "ephemeral"
name = "Ephemeral Agent"
"#,
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert_eq!(loader.count(), 1);

        // Delete the file on disk
        fs::remove_file(&path).unwrap();

        let changed = loader.check_for_changes();
        // After change detection, the deleted manifest should be pruned
        assert_eq!(loader.count(), 0, "deleted manifest should be pruned");
        // changed may be empty (no new/modified files), but count should reflect removal
        let _ = changed; // suppress unused warning
    }

    #[test]
    fn reject_slash_in_id() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("slash.toml"),
            "id = \"has/slash\"\nname = \"Bad Slash\"",
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0, "ID with slash should be rejected by validation");
    }

    #[test]
    fn reject_empty_name() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("noname.toml"),
            "id = \"valid-id\"\nname = \"\"",
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 0, "empty name should be rejected");
    }

    #[test]
    fn list_all_manifests() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("a.toml"),
            "id = \"a\"\nname = \"Agent A\"",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.toml"),
            "id = \"b\"\nname = \"Agent B\"",
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert_eq!(loader.list().len(), 2);
    }

    #[test]
    fn directory_accessor() {
        let dir = TempDir::new().unwrap();
        let loader = ManifestLoader::new(dir.path().to_path_buf());
        assert_eq!(loader.directory(), dir.path());
    }

    #[test]
    fn get_nonexistent_manifest() {
        let dir = TempDir::new().unwrap();
        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert!(loader.get("nonexistent").is_none());
    }

    #[test]
    fn check_for_changes_nonexistent_directory() {
        let mut loader = ManifestLoader::new(PathBuf::from("/nonexistent/manifests"));
        let changed = loader.check_for_changes();
        assert!(changed.is_empty());
    }

    #[test]
    fn check_for_changes_new_file_added() {
        let dir = TempDir::new().unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        loader.load_all().unwrap();
        assert_eq!(loader.count(), 0);

        // Add a new manifest file after initial load
        fs::write(
            dir.path().join("new_agent.toml"),
            "id = \"new\"\nname = \"New Agent\"",
        )
        .unwrap();

        let changed = loader.check_for_changes();
        assert!(changed.contains(&"new".to_string()));
        assert_eq!(loader.count(), 1);
    }

    #[test]
    fn manifest_serde_roundtrip() {
        let manifest: AgentManifest = toml::from_str(sample_manifest()).unwrap();
        let serialized = toml::to_string(&manifest).unwrap();
        let back: AgentManifest = toml::from_str(&serialized).unwrap();
        assert_eq!(back.id, manifest.id);
        assert_eq!(back.name, manifest.name);
        assert_eq!(back.capabilities, manifest.capabilities);
    }

    #[test]
    fn non_toml_files_ignored() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("readme.md"),
            "# Not a manifest",
        )
        .unwrap();
        fs::write(
            dir.path().join("config.json"),
            "{}",
        )
        .unwrap();
        fs::write(
            dir.path().join("agent.toml"),
            "id = \"real\"\nname = \"Real Agent\"",
        )
        .unwrap();

        let mut loader = ManifestLoader::new(dir.path().to_path_buf());
        let count = loader.load_all().unwrap();
        assert_eq!(count, 1, "only .toml files should be loaded");
    }
}
