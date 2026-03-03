//! Plugin catalog types for remote plugin discovery and distribution.
//!
//! These types map the `plugins` section of the Ironclad registry manifest.

use serde::{Deserialize, Serialize};

/// A single plugin entry in the remote catalog.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginCatalogEntry {
    /// Plugin name (must match the manifest's `name` field).
    pub name: String,
    /// Semver version string.
    pub version: String,
    /// Human-readable description.
    pub description: String,
    /// Author or organization.
    #[serde(default)]
    pub author: String,
    /// SHA-256 hex digest of the `.ic.zip` archive.
    pub sha256: String,
    /// Relative path to the archive within the registry (e.g., `plugins/claude-code-0.1.0.ic.zip`).
    pub path: String,
    /// Minimum Ironclad version required to run this plugin.
    #[serde(default)]
    pub min_version: Option<String>,
    /// Trust tier: "official", "community", "third-party".
    #[serde(default = "default_tier")]
    pub tier: String,
}

fn default_tier() -> String {
    "community".to_string()
}

/// The `plugins` section of the registry manifest.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PluginCatalog {
    /// Available plugins for installation.
    #[serde(default)]
    pub catalog: Vec<PluginCatalogEntry>,
}

impl PluginCatalog {
    /// Search catalog entries by name or description substring (case-insensitive).
    pub fn search(&self, query: &str) -> Vec<&PluginCatalogEntry> {
        let q = query.to_lowercase();
        self.catalog
            .iter()
            .filter(|e| {
                e.name.to_lowercase().contains(&q)
                    || e.description.to_lowercase().contains(&q)
                    || e.author.to_lowercase().contains(&q)
            })
            .collect()
    }

    /// Find a specific plugin by exact name.
    pub fn find(&self, name: &str) -> Option<&PluginCatalogEntry> {
        self.catalog.iter().find(|e| e.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> PluginCatalog {
        PluginCatalog {
            catalog: vec![
                PluginCatalogEntry {
                    name: "claude-code".into(),
                    version: "0.1.0".into(),
                    description: "Delegate coding tasks to Claude Code CLI".into(),
                    author: "Ironclad".into(),
                    sha256: "abc123".into(),
                    path: "plugins/claude-code-0.1.0.ic.zip".into(),
                    min_version: Some("0.9.4".into()),
                    tier: "official".into(),
                },
                PluginCatalogEntry {
                    name: "weather".into(),
                    version: "1.0.0".into(),
                    description: "Check weather forecasts".into(),
                    author: "Community".into(),
                    sha256: "def456".into(),
                    path: "plugins/weather-1.0.0.ic.zip".into(),
                    min_version: None,
                    tier: "community".into(),
                },
            ],
        }
    }

    #[test]
    fn search_by_name() {
        let cat = sample_catalog();
        let results = cat.search("claude");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "claude-code");
    }

    #[test]
    fn search_by_description() {
        let cat = sample_catalog();
        let results = cat.search("forecast");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "weather");
    }

    #[test]
    fn search_case_insensitive() {
        let cat = sample_catalog();
        let results = cat.search("CLAUDE");
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn find_exact() {
        let cat = sample_catalog();
        assert!(cat.find("weather").is_some());
        assert!(cat.find("nonexistent").is_none());
    }

    #[test]
    fn empty_search_returns_all() {
        let cat = sample_catalog();
        let results = cat.search("");
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn serde_roundtrip() {
        let cat = sample_catalog();
        let json = serde_json::to_string(&cat).unwrap();
        let back: PluginCatalog = serde_json::from_str(&json).unwrap();
        assert_eq!(back.catalog.len(), 2);
        assert_eq!(back.catalog[0].tier, "official");
    }
}
