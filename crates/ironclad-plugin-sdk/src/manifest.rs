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
    /// Declared permissions for this plugin (e.g., "network", "filesystem").
    /// Runtime currently enforces allow-list policy and input capability checks,
    /// but does not yet provide full syscall-level sandbox guarantees.
    #[serde(default)]
    pub permissions: Vec<String>,
    /// Per-plugin script execution timeout in seconds. Defaults to 30.
    /// Plugins that invoke long-running external processes (e.g., AI coding agents)
    /// should set this higher.
    #[serde(default)]
    pub timeout_seconds: Option<u64>,
    /// External requirements that must be present for the plugin to function.
    /// Checked at install time; missing required dependencies block installation.
    #[serde(default)]
    pub requirements: Vec<Requirement>,
    /// Relative paths to companion skill files bundled within the plugin directory.
    /// These are installed into the skills directory alongside the plugin.
    #[serde(default)]
    pub companion_skills: Vec<String>,
    #[serde(default)]
    pub tools: Vec<ManifestToolDef>,
}

/// An external dependency required by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    /// Human-readable name of the requirement (e.g., "Claude Code CLI").
    pub name: String,
    /// Binary name to check via `command -v` / `which` (e.g., "claude").
    pub command: String,
    /// URL or instructions for installing the requirement.
    #[serde(default)]
    pub install_hint: Option<String>,
    /// If true, the plugin can function without this requirement (with degraded capability).
    #[serde(default)]
    pub optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestToolDef {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub dangerous: bool,
    /// Optional per-tool permissions. If omitted, plugin-level permissions apply.
    #[serde(default)]
    pub permissions: Vec<String>,
}

/// Cross-platform check for whether a command is available on PATH.
#[cfg(unix)]
fn is_command_available(command: &str) -> bool {
    std::process::Command::new("sh")
        .args(["-c", &format!("command -v {}", command)])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Cross-platform check for whether a command is available on PATH.
#[cfg(windows)]
fn is_command_available(command: &str) -> bool {
    std::process::Command::new("where.exe")
        .arg(command)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
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
        Self::validate_plugin_name(&self.name)?;
        Self::validate_plugin_version(&self.version)?;
        for perm in &self.permissions {
            Self::validate_permission_name(perm)?;
        }
        for tool in &self.tools {
            Self::validate_tool_name(&tool.name)?;
            for perm in &tool.permissions {
                Self::validate_permission_name(perm)?;
            }
        }
        for req in &self.requirements {
            Self::validate_requirement(req)?;
        }
        for skill_path in &self.companion_skills {
            Self::validate_companion_skill_path(skill_path)?;
        }
        Ok(())
    }

    fn validate_plugin_name(name: &str) -> Result<()> {
        if name.is_empty() {
            return Err(IroncladError::Config("plugin name is required".into()));
        }
        if name.len() > 128 {
            return Err(IroncladError::Config(format!(
                "plugin name exceeds 128 characters (got {})",
                name.len()
            )));
        }
        if name.contains('/') || name.contains('\\') || name.contains('\0') || name.contains("..") {
            return Err(IroncladError::Config(format!(
                "plugin name '{name}' contains forbidden characters"
            )));
        }
        if !name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
        {
            return Err(IroncladError::Config(format!(
                "plugin name '{name}' must contain only alphanumeric, underscore, or hyphen characters"
            )));
        }
        Ok(())
    }

    fn validate_plugin_version(version: &str) -> Result<()> {
        if version.is_empty() {
            return Err(IroncladError::Config("plugin version is required".into()));
        }
        if version.len() > 64 {
            return Err(IroncladError::Config(format!(
                "plugin version exceeds 64 characters (got {})",
                version.len()
            )));
        }
        if !version
            .chars()
            .all(|c| c.is_alphanumeric() || c == '.' || c == '-')
        {
            return Err(IroncladError::Config(format!(
                "plugin version '{version}' must contain only alphanumeric, dot, or hyphen characters"
            )));
        }
        Ok(())
    }

    pub fn declared_permissions(&self) -> &[String] {
        &self.permissions
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
        if name.len() > 64 {
            return Err(IroncladError::Config(format!(
                "tool name exceeds 64 characters (got {})",
                name.len()
            )));
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

    fn validate_permission_name(name: &str) -> Result<()> {
        match name.to_ascii_lowercase().as_str() {
            "filesystem" | "network" | "process" | "environment" => Ok(()),
            other => Err(IroncladError::Config(format!(
                "unsupported permission '{other}' (allowed: filesystem, network, process, environment)"
            ))),
        }
    }

    fn validate_requirement(req: &Requirement) -> Result<()> {
        if req.name.is_empty() {
            return Err(IroncladError::Config(
                "requirement name cannot be empty".into(),
            ));
        }
        if req.command.is_empty() {
            return Err(IroncladError::Config(format!(
                "requirement '{}' must specify a command to check",
                req.name
            )));
        }
        // Prevent shell injection in the command name
        if !req
            .command
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-' || c == '.')
        {
            return Err(IroncladError::Config(format!(
                "requirement command '{}' contains invalid characters",
                req.command
            )));
        }
        Ok(())
    }

    fn validate_companion_skill_path(path: &str) -> Result<()> {
        if path.is_empty() {
            return Err(IroncladError::Config(
                "companion skill path cannot be empty".into(),
            ));
        }
        if path.contains("..") || path.starts_with('/') || path.starts_with('\\') {
            return Err(IroncladError::Config(format!(
                "companion skill path '{path}' must be a relative path without traversal"
            )));
        }
        if !path.ends_with(".md") {
            return Err(IroncladError::Config(format!(
                "companion skill path '{path}' must end with .md"
            )));
        }
        Ok(())
    }

    /// Check which requirements are satisfied on the current system.
    /// Returns a list of `(requirement, found)` tuples.
    pub fn check_requirements(&self) -> Vec<(&Requirement, bool)> {
        self.requirements
            .iter()
            .map(|req| {
                let found = is_command_available(&req.command);
                (req, found)
            })
            .collect()
    }

    /// Returns true if all required (non-optional) requirements are satisfied.
    pub fn all_required_satisfied(&self) -> bool {
        self.check_requirements()
            .iter()
            .all(|(req, found)| *found || req.optional)
    }

    /// Vet an installed plugin directory for integrity and readiness.
    ///
    /// Checks:
    /// - All companion skill files exist within the plugin directory
    /// - All required external dependencies are present
    /// - Tool scripts exist for each declared tool
    ///
    /// Returns a [`VetReport`] with categorized issues.
    pub fn vet(&self, plugin_dir: &Path) -> VetReport {
        let mut report = VetReport {
            plugin_name: self.name.clone(),
            errors: Vec::new(),
            warnings: Vec::new(),
        };

        // ── Companion skills exist in bundle ─────────────────────
        for skill_path in &self.companion_skills {
            let full = plugin_dir.join(skill_path);
            if !full.exists() {
                report
                    .errors
                    .push(format!("companion skill not found in bundle: {skill_path}"));
            }
        }

        // ── External requirements satisfied ──────────────────────
        for (req, found) in self.check_requirements() {
            if !found {
                if req.optional {
                    report.warnings.push(format!(
                        "optional requirement '{}' ({}) not found",
                        req.name, req.command
                    ));
                } else {
                    report.errors.push(format!(
                        "required dependency '{}' ({}) not found",
                        req.name, req.command
                    ));
                }
            }
        }

        // ── Tool scripts exist ───────────────────────────────────
        let script_extensions = ["sh", "py", "rb", "js", "go", "gosh"];
        for tool in &self.tools {
            let has_script = script_extensions
                .iter()
                .any(|ext| plugin_dir.join(format!("{}.{}", tool.name, ext)).exists())
                || plugin_dir.join(&tool.name).exists(); // extensionless shebang script
            if !has_script {
                report.warnings.push(format!(
                    "no script file found for tool '{}' (checked: {})",
                    tool.name,
                    script_extensions
                        .iter()
                        .map(|e| format!("{}.{e}", tool.name))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        // ── Dangerous permissions flagged ────────────────────────
        if self.permissions.iter().any(|p| p == "process") {
            report.warnings.push(
                "plugin declares 'process' permission (can execute arbitrary commands)".into(),
            );
        }

        report
    }
}

/// Result of vetting an installed plugin for integrity and readiness.
#[derive(Debug, Clone)]
pub struct VetReport {
    pub plugin_name: String,
    /// Hard errors that should block activation (missing dependencies, broken files).
    pub errors: Vec<String>,
    /// Soft warnings that should be logged but don't block activation.
    pub warnings: Vec<String>,
}

impl VetReport {
    /// Returns true if there are no blocking errors.
    pub fn is_ok(&self) -> bool {
        self.errors.is_empty()
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
    fn tool_name_too_long_rejected() {
        let long_tool = "a".repeat(65);
        let toml = format!(
            "name = \"test\"\nversion = \"1.0.0\"\n[[tools]]\nname = \"{long_tool}\"\ndescription = \"too long\"\n"
        );
        let result = PluginManifest::from_str(&toml);
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
    fn plugin_name_with_path_traversal_rejected() {
        let toml = r#"
name = "../escape"
version = "1.0.0"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_name_with_spaces_rejected() {
        let toml = r#"
name = "my plugin"
version = "1.0.0"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_name_too_long_rejected() {
        let long_name = "a".repeat(129);
        let toml = format!("name = \"{long_name}\"\nversion = \"1.0.0\"\n");
        let result = PluginManifest::from_str(&toml);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_version_too_long_rejected() {
        let long_version = "1.".repeat(33);
        let toml = format!("name = \"test\"\nversion = \"{long_version}\"\n");
        let result = PluginManifest::from_str(&toml);
        assert!(result.is_err());
    }

    #[test]
    fn plugin_version_with_invalid_chars_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0; rm -rf /"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn timeout_seconds_defaults_to_none() {
        let toml = r#"
name = "test"
version = "1.0.0"
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert!(manifest.timeout_seconds.is_none());
    }

    #[test]
    fn timeout_seconds_parsed() {
        let toml = r#"
name = "test"
version = "1.0.0"
timeout_seconds = 300
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert_eq!(manifest.timeout_seconds, Some(300));
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

    #[test]
    fn claude_code_plugin_manifest_parses() {
        let toml = r#"
name = "claude-code"
version = "0.1.0"
description = "Delegate complex coding tasks to Claude Code CLI"
author = "Ironclad"
permissions = ["filesystem", "process"]
timeout_seconds = 300
companion_skills = ["skills/claude-code.md"]

[[requirements]]
name = "Claude Code CLI"
command = "claude"
install_hint = "https://docs.anthropic.com/en/docs/claude-code"

[[requirements]]
name = "jq"
command = "jq"
install_hint = "https://stedolan.github.io/jq/download/"

[[tools]]
name = "claude-code"
description = "Invoke Claude Code CLI to perform a coding task."
dangerous = true
permissions = ["filesystem", "process"]
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert_eq!(manifest.name, "claude-code");
        assert_eq!(manifest.timeout_seconds, Some(300));
        assert_eq!(manifest.tools.len(), 1);
        assert!(manifest.tools[0].dangerous);
        assert_eq!(manifest.permissions, vec!["filesystem", "process"]);
        assert_eq!(manifest.requirements.len(), 2);
        assert_eq!(manifest.requirements[0].command, "claude");
        assert!(!manifest.requirements[0].optional);
        assert_eq!(manifest.companion_skills, vec!["skills/claude-code.md"]);
    }

    #[test]
    fn requirements_default_to_empty() {
        let toml = r#"
name = "test"
version = "1.0.0"
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert!(manifest.requirements.is_empty());
        assert!(manifest.companion_skills.is_empty());
    }

    #[test]
    fn optional_requirement_parsed() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "Optional Tool"
command = "some-tool"
optional = true
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert_eq!(manifest.requirements.len(), 1);
        assert!(manifest.requirements[0].optional);
    }

    #[test]
    fn requirement_with_shell_injection_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "evil"
command = "echo; rm -rf /"
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn requirement_empty_command_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "missing"
command = ""
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn companion_skill_path_traversal_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0"
companion_skills = ["../../etc/passwd.md"]
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn companion_skill_absolute_path_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0"
companion_skills = ["/etc/skill.md"]
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn companion_skill_non_md_rejected() {
        let toml = r#"
name = "test"
version = "1.0.0"
companion_skills = ["skills/evil.sh"]
"#;
        let result = PluginManifest::from_str(toml);
        assert!(result.is_err());
    }

    #[test]
    fn check_requirements_finds_sh() {
        // `sh` should exist on any UNIX system
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "shell"
command = "sh"
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        let results = manifest.check_requirements();
        assert_eq!(results.len(), 1);
        assert!(results[0].1, "sh should be found on PATH");
        assert!(manifest.all_required_satisfied());
    }

    #[test]
    fn check_requirements_missing_command() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "nonexistent"
command = "this-command-does-not-exist-xyz"
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        let results = manifest.check_requirements();
        assert_eq!(results.len(), 1);
        assert!(!results[0].1, "nonexistent command should not be found");
        assert!(!manifest.all_required_satisfied());
    }

    #[test]
    fn optional_missing_requirement_still_satisfies() {
        let toml = r#"
name = "test"
version = "1.0.0"
[[requirements]]
name = "optional-thing"
command = "this-command-does-not-exist-xyz"
optional = true
"#;
        let manifest = PluginManifest::from_str(toml).unwrap();
        assert!(
            manifest.all_required_satisfied(),
            "optional missing requirement should not block"
        );
    }
}
