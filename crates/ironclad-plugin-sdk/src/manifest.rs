use std::path::Path;

use serde::{Deserialize, Serialize};

use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PluginManifest {
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub permissions: Vec<String>,
    #[serde(default)]
    pub tools: Vec<ManifestToolDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestToolDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub dangerous: bool,
}

impl PluginManifest {
    pub fn from_file(path: &Path) -> Result<Self> {
        let contents = std::fs::read_to_string(path)?;
        Self::from_str(&contents)
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(toml_str: &str) -> Result<Self> {
        let manifest: Self = toml::from_str(toml_str)
            .map_err(|e| IroncladError::Config(format!("plugin manifest parse error: {e}")))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(IroncladError::Config("plugin name is required".into()));
        }
        if self.version.is_empty() {
            return Err(IroncladError::Config("plugin version is required".into()));
        }
        for tool in &self.tools {
            Self::validate_tool_name(&tool.name)?;
        }
        Ok(())
    }

    pub fn is_tool_dangerous(&self, tool_name: &str) -> bool {
        self.tools
            .iter()
            .any(|t| t.name == tool_name && t.dangerous)
    }

    fn validate_tool_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(IroncladError::Config("tool name cannot be empty".into()));
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') || name.contains("..") {
            return Err(IroncladError::Config(format!(
                "tool name '{name}' contains forbidden characters"
            )));
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(IroncladError::Config(format!(
                "tool name '{name}' must contain only alphanumeric, underscore, or hyphen characters"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_minimal_manifest() {
        let toml = r#"
name = "test-plugin"
version = "1.0.0"
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert_eq!(manifest.name, "test-plugin");
        assert_eq!(manifest.version, "1.0.0");
        assert!(manifest.permissions.is_empty());
        assert!(manifest.tools.is_empty());
    }

    #[test]
    fn parse_full_manifest() {
        let toml = r#"
name = "github"
version = "0.2.0"
description = "GitHub integration"
author = "Ironclad"
permissions = ["network", "filesystem"]

[[tools]]
name = "list_repos"
description = "List GitHub repositories"

[[tools]]
name = "create_issue"
description = "Create a GitHub issue"
dangerous = true
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert_eq!(manifest.name, "github");
        assert_eq!(manifest.tools.len(), 2);
        assert!(!manifest.tools[0].dangerous);
        assert!(manifest.tools[1].dangerous);
        assert_eq!(manifest.permissions, vec!["network", "filesystem"]);
    }

    #[test]
    fn empty_name_fails() {
        let toml = r#"
name = ""
version = "1.0.0"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn empty_version_fails() {
        let toml = r#"
name = "test"
version = ""
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn invalid_toml_fails() {
        let result = PluginManifest::from_str("[[[[bad");
        assert!(result.is_err());
    }

    #[test]
    fn from_missing_file_fails() {
        let result = PluginManifest::from_file(Path::new("/nonexistent/plugin.toml"));
        assert!(result.is_err());
    }

    #[test]
    fn tool_name_with_path_separator_rejected() {
        let toml = r#"
name = "evil"
version = "1.0.0"
[[tools]]
name = "../../../etc/passwd"
description = "path traversal"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn tool_name_with_spaces_rejected() {
        let toml = r#"
name = "evil"
version = "1.0.0"
[[tools]]
name = "my tool"
description = "has spaces"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn is_tool_dangerous_flag() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[tools]]
name = "safe_tool"
description = "safe"
[[tools]]
name = "danger_tool"
description = "dangerous"
dangerous = true
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert!(!manifest.is_tool_dangerous("safe_tool"));
        assert!(manifest.is_tool_dangerous("danger_tool"));
        assert!(!manifest.is_tool_dangerous("nonexistent"));
    }
}
