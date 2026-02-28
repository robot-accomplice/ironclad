use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt;
use tracing::{debug, warn};

use ironclad_core::{IroncladError, Result};

use crate::manifest::PluginManifest;
use crate::{Plugin, ToolDef, ToolResult};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);
/// Maximum bytes to read from script stdout/stderr (10 MB).
const MAX_SCRIPT_OUTPUT: u64 = 10 * 1024 * 1024;
const SCRIPT_EXTENSIONS: &[&str] = &[
    "gosh", "go", "sh", "py", "rb", "js",
    // Empty string matches extensionless files (e.g., `tool_name` without `.sh`).
    // This is checked last so that recognized extensions take priority. Extensionless
    // files are only accepted if they begin with a recognized shebang line; see
    // `validate_shebang()`. Without the shebang check an attacker could place an
    // arbitrary binary in the plugin directory and have it executed.
    "",
];

/// A concrete `Plugin` implementation that executes external scripts.
///
/// Each tool declared in the plugin's `plugin.toml` maps to a script file
/// in the plugin directory. The script receives input as the `IRONCLAD_INPUT`
/// environment variable (JSON) and should write its output to stdout.
pub struct ScriptPlugin {
    manifest: PluginManifest,
    dir: PathBuf,
    scripts: HashMap<String, PathBuf>,
    timeout: Duration,
}

impl ScriptPlugin {
    pub fn new(manifest: PluginManifest, dir: PathBuf) -> Self {
        let scripts = Self::discover_scripts(&manifest, &dir);
        Self {
            manifest,
            dir,
            scripts,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    fn discover_scripts(manifest: &PluginManifest, dir: &Path) -> HashMap<String, PathBuf> {
        let mut scripts = HashMap::new();
        for tool in &manifest.tools {
            if let Some(path) = Self::find_script(dir, &tool.name) {
                debug!(tool = %tool.name, script = %path.display(), "mapped tool to script");
                scripts.insert(tool.name.clone(), path);
            } else {
                warn!(tool = %tool.name, dir = %dir.display(), "no script found for tool");
            }
        }
        scripts
    }

    fn find_script(dir: &Path, tool_name: &str) -> Option<PathBuf> {
        for ext in SCRIPT_EXTENSIONS {
            let filename = if ext.is_empty() {
                tool_name.to_string()
            } else {
                format!("{tool_name}.{ext}")
            };
            let path = dir.join(&filename);
            if path.exists() && path.is_file() {
                if let Err(e) = Self::validate_script_path(&path, dir) {
                    warn!(tool = %tool_name, error = %e, "script path rejected");
                    return None;
                }
                // Extensionless files must have a recognized shebang line so we
                // don't accidentally execute an arbitrary binary.
                if ext.is_empty() && !Self::has_recognized_shebang(&path) {
                    warn!(
                        tool = %tool_name,
                        path = %path.display(),
                        "extensionless script rejected: missing recognized shebang"
                    );
                    continue;
                }
                return Some(path);
            }
        }
        None
    }

    /// Returns `true` if the file starts with a shebang (`#!`) whose interpreter
    /// is one we recognize. This prevents extensionless arbitrary binaries from
    /// being executed as plugin scripts.
    fn has_recognized_shebang(path: &Path) -> bool {
        const RECOGNIZED_INTERPRETERS: &[&str] = &[
            "sh", "bash", "zsh", "python", "python3", "ruby", "node", "gosh", "go",
        ];

        let Ok(content) = std::fs::read_to_string(path) else {
            return false;
        };
        let Some(first_line) = content.lines().next() else {
            return false;
        };
        if !first_line.starts_with("#!") {
            return false;
        }
        // Extract the interpreter name from e.g. "#!/usr/bin/env python3" or "#!/bin/sh"
        let shebang = first_line.trim_start_matches("#!");
        let last_token = shebang.split_whitespace().last().unwrap_or("");
        let interpreter = last_token.rsplit('/').next().unwrap_or(last_token);
        RECOGNIZED_INTERPRETERS.contains(&interpreter)
    }

    /// Ensures a resolved script path is contained within the plugin directory.
    /// Prevents path traversal attacks via symlinks or `..` components.
    fn validate_script_path(script: &Path, plugin_dir: &Path) -> Result<()> {
        let canonical_script = script.canonicalize().map_err(|e| IroncladError::Tool {
            tool: script.display().to_string(),
            message: format!("cannot resolve script path: {e}"),
        })?;
        let canonical_dir = plugin_dir.canonicalize().map_err(|e| IroncladError::Tool {
            tool: plugin_dir.display().to_string(),
            message: format!("cannot resolve plugin directory: {e}"),
        })?;
        if !canonical_script.starts_with(&canonical_dir) {
            return Err(IroncladError::Tool {
                tool: script.display().to_string(),
                message: "script path escapes plugin directory".into(),
            });
        }
        Ok(())
    }

    fn interpreter_for(path: &Path) -> Option<(&'static str, &'static [&'static str])> {
        #[cfg(windows)]
        const PYTHON_BIN: &str = "python";
        #[cfg(not(windows))]
        const PYTHON_BIN: &str = "python3";

        match path.extension().and_then(|e| e.to_str()) {
            Some("gosh") => Some(("gosh", &[])),
            Some("go") => Some(("go", &["run"])),
            Some("py") => Some((PYTHON_BIN, &[])),
            Some("rb") => Some(("ruby", &[])),
            Some("js") => Some(("node", &[])),
            Some("sh") => Some(("sh", &[])),
            _ => None,
        }
    }

    pub fn has_script(&self, tool_name: &str) -> bool {
        self.scripts.contains_key(tool_name)
    }

    pub fn script_path(&self, tool_name: &str) -> Option<&Path> {
        self.scripts.get(tool_name).map(|p| p.as_path())
    }

    pub fn script_count(&self) -> usize {
        self.scripts.len()
    }

    pub fn is_tool_dangerous(&self, tool_name: &str) -> bool {
        self.manifest.is_tool_dangerous(tool_name)
    }

    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
}

#[async_trait]
impl Plugin for ScriptPlugin {
    fn name(&self) -> &str {
        &self.manifest.name
    }

    fn version(&self) -> &str {
        &self.manifest.version
    }

    fn tools(&self) -> Vec<ToolDef> {
        self.manifest
            .tools
            .iter()
            .map(|t| ToolDef {
                name: t.name.clone(),
                description: t.description.clone(),
                parameters: json!({"type": "object"}),
                risk_level: if t.dangerous {
                    ironclad_core::RiskLevel::Dangerous
                } else {
                    ironclad_core::RiskLevel::Caution
                },
                permissions: self.manifest.permissions.clone(),
            })
            .collect()
    }

    async fn init(&mut self) -> Result<()> {
        self.scripts = Self::discover_scripts(&self.manifest, &self.dir);
        debug!(
            plugin = self.manifest.name,
            scripts = self.scripts.len(),
            "ScriptPlugin initialized"
        );
        Ok(())
    }

    async fn execute_tool(&self, tool_name: &str, input: &Value) -> Result<ToolResult> {
        let script_path = self
            .scripts
            .get(tool_name)
            .ok_or_else(|| IroncladError::Tool {
                tool: tool_name.into(),
                message: format!(
                    "no script found for tool '{}' in {}",
                    tool_name,
                    self.dir.display()
                ),
            })?;

        let input_str = serde_json::to_string(input).unwrap_or_else(|_| "{}".to_string());

        let mut cmd = if let Some((program, extra_args)) = Self::interpreter_for(script_path) {
            let mut c = tokio::process::Command::new(program);
            c.args(extra_args);
            c.arg(script_path);
            c
        } else {
            tokio::process::Command::new(script_path)
        };

        cmd.env_clear()
            .env("IRONCLAD_INPUT", &input_str)
            .env("IRONCLAD_TOOL", tool_name)
            .env("IRONCLAD_PLUGIN", &self.manifest.name);

        for key in &["PATH", "HOME", "USER", "LANG", "TERM", "TMPDIR"] {
            if let Ok(val) = std::env::var(key) {
                cmd.env(key, val);
            }
        }

        cmd.current_dir(&self.dir)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());

        let mut child = cmd.spawn().map_err(|e| IroncladError::Tool {
            tool: tool_name.into(),
            message: format!("failed to spawn script: {e}"),
        })?;

        // Take stdout/stderr pipes for bounded reading.
        let mut child_stdout = child.stdout.take();
        let mut child_stderr = child.stderr.take();

        let timeout = self.timeout;
        let tool = tool_name.to_string();

        let result = tokio::time::timeout(timeout, async {
            // Read stdout and stderr concurrently, bounded to MAX_SCRIPT_OUTPUT.
            let stdout_fut = async {
                let mut buf = Vec::new();
                if let Some(out) = child_stdout.take() {
                    out.take(MAX_SCRIPT_OUTPUT)
                        .read_to_end(&mut buf)
                        .await
                        .inspect_err(
                            |e| tracing::debug!(error = %e, "failed to read script stdout"),
                        )
                        .ok();
                }
                // `out` dropped here — closes the pipe, which sends SIGPIPE
                // to the child if it tries to write more.
                buf
            };
            let stderr_fut = async {
                let mut buf = Vec::new();
                if let Some(err) = child_stderr.take() {
                    err.take(MAX_SCRIPT_OUTPUT)
                        .read_to_end(&mut buf)
                        .await
                        .inspect_err(
                            |e| tracing::debug!(error = %e, "failed to read script stderr"),
                        )
                        .ok();
                }
                buf
            };

            let (stdout_bytes, stderr_bytes) = tokio::join!(stdout_fut, stderr_fut);

            // If the child is still running (e.g. output exceeded the cap
            // and the process hasn't received/handled SIGPIPE yet), kill it.
            let _ = child.kill().await;
            let status = child.wait().await;
            (stdout_bytes, stderr_bytes, status)
        })
        .await;

        match result {
            Ok((stdout_bytes, stderr_bytes, status)) => {
                let stdout = String::from_utf8_lossy(&stdout_bytes).to_string();
                let stderr = String::from_utf8_lossy(&stderr_bytes).to_string();
                let status = status.map_err(|e| IroncladError::Tool {
                    tool: tool.clone(),
                    message: format!("script execution failed: {e}"),
                })?;

                if status.success() {
                    Ok(ToolResult {
                        success: true,
                        output: stdout,
                        metadata: if stderr.is_empty() {
                            None
                        } else {
                            Some(json!({ "stderr": stderr }))
                        },
                    })
                } else {
                    let code = status.code().unwrap_or(-1);
                    Ok(ToolResult {
                        success: false,
                        output: if stderr.is_empty() {
                            format!("script exited with code {code}")
                        } else {
                            stderr
                        },
                        metadata: Some(json!({
                            "exit_code": code,
                            "stdout": stdout,
                        })),
                    })
                }
            }
            Err(_) => {
                // Timeout: kill the child process and reap it to prevent zombies.
                let _ = child.kill().await;
                let _ = child.wait().await;
                Err(IroncladError::Tool {
                    tool,
                    message: format!("script timed out after {timeout:?}"),
                })
            }
        }
    }

    async fn shutdown(&mut self) -> Result<()> {
        debug!(plugin = self.manifest.name, "ScriptPlugin shutdown");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::ManifestToolDef;
    use std::fs;

    fn test_manifest(name: &str, tools: Vec<(&str, &str)>) -> PluginManifest {
        PluginManifest {
            name: name.into(),
            version: "1.0.0".into(),
            description: "test plugin".into(),
            author: "test".into(),
            permissions: vec![],
            tools: tools
                .into_iter()
                .map(|(n, d)| ManifestToolDef {
                    name: n.into(),
                    description: d.into(),
                    dangerous: false,
                })
                .collect(),
        }
    }

    #[test]
    fn discover_scripts_finds_gosh() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("greet.gosh"), "echo hello").unwrap();

        let manifest = test_manifest("test", vec![("greet", "says hello")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.has_script("greet"));
        assert_eq!(plugin.script_count(), 1);
    }

    #[test]
    fn discover_scripts_finds_py() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("analyze.py"), "print('done')").unwrap();

        let manifest = test_manifest("test", vec![("analyze", "analyzes stuff")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.has_script("analyze"));
    }

    #[test]
    fn gosh_preferred_over_all_others() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("tool.gosh"), "echo gosh wins").unwrap();
        fs::write(dir.path().join("tool.go"), "package main\nfunc main() {}\n").unwrap();
        fs::write(dir.path().join("tool.sh"), "#!/bin/sh\necho hi").unwrap();
        fs::write(dir.path().join("tool.py"), "print('hi')").unwrap();

        let manifest = test_manifest("test", vec![("tool", "prefers gosh")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.has_script("tool"));
        let path = plugin.script_path("tool").unwrap();
        assert!(
            path.to_string_lossy().ends_with(".gosh"),
            "expected .gosh but got: {}",
            path.display()
        );
    }

    #[test]
    fn discover_scripts_missing_tool() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("test", vec![("missing_tool", "not here")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(!plugin.has_script("missing_tool"));
        assert_eq!(plugin.script_count(), 0);
    }

    #[test]
    fn interpreter_selection() {
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.gosh")),
            Some(("gosh", [].as_slice()))
        );
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.go")),
            Some(("go", ["run"].as_slice()))
        );
        #[cfg(windows)]
        let expected_python = Some(("python", [].as_slice()));
        #[cfg(not(windows))]
        let expected_python = Some(("python3", [].as_slice()));
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.py")),
            expected_python
        );
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.sh")),
            Some(("sh", [].as_slice()))
        );
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.rb")),
            Some(("ruby", [].as_slice()))
        );
        assert_eq!(
            ScriptPlugin::interpreter_for(Path::new("x.js")),
            Some(("node", [].as_slice()))
        );
        assert_eq!(ScriptPlugin::interpreter_for(Path::new("x")), None);
    }

    #[test]
    fn plugin_name_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("my-plugin", vec![]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert_eq!(plugin.name(), "my-plugin");
        assert_eq!(plugin.version(), "1.0.0");
    }

    #[test]
    fn tools_from_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("p", vec![("a", "tool a"), ("b", "tool b")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let tools = plugin.tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].name, "a");
        assert_eq!(tools[1].name, "b");
    }

    #[tokio::test]
    async fn execute_script_success() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("greet.sh"),
            "#!/bin/sh\necho \"hello from $IRONCLAD_TOOL\"",
        )
        .unwrap();

        let manifest = test_manifest("test", vec![("greet", "greets")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let result = plugin
            .execute_tool("greet", &json!({"name": "world"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello from greet"));
    }

    #[tokio::test]
    async fn execute_missing_tool_fails() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("test", vec![("missing", "not here")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let result = plugin.execute_tool("missing", &json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn execute_failing_script() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("fail.sh"), "#!/bin/sh\nexit 1").unwrap();

        let manifest = test_manifest("test", vec![("fail", "always fails")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let result = plugin.execute_tool("fail", &json!({})).await.unwrap();
        assert!(!result.success);
    }

    #[tokio::test]
    async fn execute_script_with_stderr() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("warn.sh"),
            "#!/bin/sh\necho 'result' && echo 'warning' >&2",
        )
        .unwrap();

        let manifest = test_manifest("test", vec![("warn", "has stderr")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let result = plugin.execute_tool("warn", &json!({})).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("result"));
        assert!(result.metadata.is_some());
        let meta = result.metadata.unwrap();
        assert!(meta["stderr"].as_str().unwrap().contains("warning"));
    }

    #[tokio::test]
    async fn init_rediscovers_scripts() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("test", vec![("late", "added later")]);
        let mut plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert_eq!(plugin.script_count(), 0);

        fs::write(dir.path().join("late.gosh"), "echo ok").unwrap();
        plugin.init().await.unwrap();
        assert_eq!(plugin.script_count(), 1);
        let path = plugin.script_path("late").unwrap();
        assert!(path.to_string_lossy().ends_with(".gosh"));
    }

    #[tokio::test]
    async fn execute_receives_ironclad_input_env() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("echo_input.sh"),
            "#!/bin/sh\necho $IRONCLAD_INPUT",
        )
        .unwrap();

        let manifest = test_manifest("test", vec![("echo_input", "echoes input")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let input = json!({"key": "value"});
        let result = plugin.execute_tool("echo_input", &input).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("key"));
        assert!(result.output.contains("value"));
    }

    #[test]
    fn with_timeout_sets_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("test", vec![]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf())
            .with_timeout(Duration::from_secs(5));
        assert_eq!(plugin.timeout, Duration::from_secs(5));
    }

    fn test_manifest_with_dangerous(name: &str, tools: Vec<(&str, &str, bool)>) -> PluginManifest {
        PluginManifest {
            name: name.into(),
            version: "1.0.0".into(),
            description: "test plugin".into(),
            author: "test".into(),
            permissions: vec![],
            tools: tools
                .into_iter()
                .map(|(n, d, dangerous)| ManifestToolDef {
                    name: n.into(),
                    description: d.into(),
                    dangerous,
                })
                .collect(),
        }
    }

    // ── has_recognized_shebang ──────────────────────────────────────

    #[test]
    fn shebang_recognized_env_python3() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "#!/usr/bin/env python3\nprint('hi')").unwrap();
        assert!(ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_recognized_direct_sh() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "#!/bin/sh\necho hi").unwrap();
        assert!(ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_recognized_bash() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "#!/usr/bin/bash\necho hi").unwrap();
        assert!(ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_unrecognized_interpreter() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "#!/usr/bin/perl\nprint 'hi'").unwrap();
        assert!(!ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_missing_no_shebang_line() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "just some text\nno shebang").unwrap();
        assert!(!ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool");
        fs::write(&path, "").unwrap();
        assert!(!ScriptPlugin::has_recognized_shebang(&path));
    }

    #[test]
    fn shebang_nonexistent_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nonexistent");
        assert!(!ScriptPlugin::has_recognized_shebang(&path));
    }

    // ── validate_script_path ────────────────────────────────────────

    #[test]
    fn validate_script_path_inside_dir_ok() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("tool.sh");
        fs::write(&script, "#!/bin/sh").unwrap();
        assert!(ScriptPlugin::validate_script_path(&script, dir.path()).is_ok());
    }

    #[test]
    fn validate_script_path_outside_dir_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let script = other.path().join("evil.sh");
        fs::write(&script, "#!/bin/sh").unwrap();
        let result = ScriptPlugin::validate_script_path(&script, dir.path());
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("escapes plugin directory"));
    }

    #[test]
    fn validate_script_path_nonexistent_script() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("nonexistent.sh");
        let result = ScriptPlugin::validate_script_path(&script, dir.path());
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("cannot resolve script path"));
    }

    #[cfg(unix)]
    #[test]
    fn validate_script_path_symlink_escape_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("payload.sh");
        fs::write(&target, "#!/bin/sh\necho pwned").unwrap();
        let link = dir.path().join("sneaky.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        let result = ScriptPlugin::validate_script_path(&link, dir.path());
        assert!(result.is_err());
    }

    // ── find_script rejection paths ─────────────────────────────────

    #[test]
    fn extensionless_file_without_shebang_rejected() {
        let dir = tempfile::tempdir().unwrap();
        // Create extensionless file with no shebang
        fs::write(dir.path().join("tool"), "just text, no shebang").unwrap();
        let manifest = test_manifest("test", vec![("tool", "extensionless")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(!plugin.has_script("tool"));
    }

    #[test]
    fn extensionless_file_with_recognized_shebang_accepted() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("tool"), "#!/bin/sh\necho hi").unwrap();
        let manifest = test_manifest("test", vec![("tool", "extensionless with shebang")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.has_script("tool"));
    }

    #[cfg(unix)]
    #[test]
    fn find_script_rejects_symlink_escape() {
        let dir = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let target = outside.path().join("evil.sh");
        fs::write(&target, "#!/bin/sh\necho pwned").unwrap();
        let link = dir.path().join("tool.sh");
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let manifest = test_manifest("test", vec![("tool", "symlinked")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        // Symlink escaping the plugin directory should be rejected
        assert!(!plugin.has_script("tool"));
    }

    // ── is_tool_dangerous / manifest getters ────────────────────────

    #[test]
    fn is_tool_dangerous_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest_with_dangerous("p", vec![("rm_all", "dangerous op", true)]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.is_tool_dangerous("rm_all"));
    }

    #[test]
    fn is_tool_dangerous_returns_false_for_safe() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest_with_dangerous("p", vec![("list", "safe op", false)]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(!plugin.is_tool_dangerous("list"));
    }

    #[test]
    fn manifest_getter() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("my-plugin", vec![("t", "test")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert_eq!(plugin.manifest().name, "my-plugin");
        assert_eq!(plugin.manifest().tools.len(), 1);
    }

    // ── tools() with dangerous flag ─────────────────────────────────

    #[test]
    fn tools_includes_dangerous_risk_level() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest_with_dangerous(
            "p",
            vec![("safe", "safe tool", false), ("danger", "risky tool", true)],
        );
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        let tools = plugin.tools();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0].risk_level, ironclad_core::RiskLevel::Caution);
        assert_eq!(tools[1].risk_level, ironclad_core::RiskLevel::Dangerous);
    }

    // ── shutdown ────────────────────────────────────────────────────

    #[tokio::test]
    async fn shutdown_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let manifest = test_manifest("test", vec![("t", "tool")]);
        let mut plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        assert!(plugin.shutdown().await.is_ok());
    }

    // ── execute_tool timeout ────────────────────────────────────────

    #[tokio::test]
    async fn execute_tool_timeout() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("slow.sh"), "#!/bin/sh\nsleep 60").unwrap();

        let manifest = test_manifest("test", vec![("slow", "sleeps forever")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf())
            .with_timeout(Duration::from_millis(100));
        let result = plugin.execute_tool("slow", &json!({})).await;
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(msg.contains("timed out"));
    }

    #[tokio::test]
    async fn execute_tool_output_bounded() {
        let dir = tempfile::tempdir().unwrap();
        // Script that writes ~12 MB of output (exceeds 10 MB limit)
        fs::write(
            dir.path().join("big.sh"),
            "#!/bin/sh\nhead -c 12582912 /dev/zero | tr '\\0' 'A'",
        )
        .unwrap();

        let manifest = test_manifest("test", vec![("big", "big output")]);
        let plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf())
            .with_timeout(Duration::from_secs(30));
        let result = plugin.execute_tool("big", &json!({})).await.unwrap();
        // The process is killed after exceeding the output cap, so success
        // may be false and stdout lands in metadata.stdout instead of output.
        let captured = if result.success {
            result.output.clone()
        } else {
            result
                .metadata
                .as_ref()
                .and_then(|m| m.get("stdout"))
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        };
        assert!(
            captured.len() <= MAX_SCRIPT_OUTPUT as usize,
            "output should be bounded to MAX_SCRIPT_OUTPUT, got {} bytes",
            captured.len()
        );
        // Verify we actually read a non-trivial amount.
        assert!(
            captured.len() > 1_000_000,
            "expected at least 1MB of output, got {} bytes",
            captured.len()
        );
    }

    // ── execute_tool spawn failure ──────────────────────────────────

    #[tokio::test]
    async fn execute_tool_spawn_failure_nonexecutable() {
        let dir = tempfile::tempdir().unwrap();
        // Create a script file but point to a nonexistent interpreter
        let script = dir.path().join("bad.sh");
        fs::write(&script, "#!/nonexistent/interpreter\necho hi").unwrap();

        let manifest = test_manifest("test", vec![("bad", "bad interpreter")]);
        let mut plugin = ScriptPlugin::new(manifest, dir.path().to_path_buf());
        // The script won't be found by discover_scripts because .sh extension
        // will use our built-in interpreter mapping. Instead, directly insert
        // a script pointing to a nonexistent binary for the extensionless case.
        let fake_path = dir.path().join("nonexistent_binary");
        fs::write(&fake_path, "").unwrap();
        plugin.scripts.insert("bad".into(), fake_path);
        let result = plugin.execute_tool("bad", &json!({})).await;
        assert!(result.is_err());
        let msg = format!("{}", result.unwrap_err());
        assert!(
            msg.contains("spawn")
                || msg.contains("permission")
                || msg.contains("denied")
                || msg.contains("failed"),
            "unexpected error: {msg}"
        );
    }
}
