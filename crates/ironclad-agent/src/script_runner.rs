use std::path::Path;

use tokio::process::Command;

use ironclad_core::config::SkillsConfig;
use ironclad_core::{IroncladError, Result};

#[derive(Debug, Clone)]
pub struct ScriptResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration_ms: u64,
}

pub struct ScriptRunner {
    config: SkillsConfig,
}

impl ScriptRunner {
    pub fn new(config: SkillsConfig) -> Self {
        Self { config }
    }

    pub async fn execute(&self, script_path: &Path, args: &[&str]) -> Result<ScriptResult> {
        let script_path = self.resolve_script_path(script_path)?;
        let interpreter = check_interpreter(&script_path, &self.config.allowed_interpreters)?;

        let working_dir = script_path.parent().unwrap_or(Path::new("."));

        let mut cmd = Command::new(&interpreter);
        cmd.arg(&script_path);
        cmd.args(args);
        cmd.current_dir(working_dir);

        if self.config.sandbox_env {
            cmd.env_clear();
            if let Ok(path) = std::env::var("PATH") {
                cmd.env("PATH", path);
            }
            if let Some(home) = default_home_env() {
                cmd.env("HOME", home);
            }
            for key in ["USERPROFILE", "TMPDIR", "TMP", "TEMP", "LANG", "TERM"] {
                if let Ok(val) = std::env::var(key) {
                    cmd.env(key, val);
                }
            }
        }

        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());

        let timeout_dur = std::time::Duration::from_secs(self.config.script_timeout_seconds);
        let start = std::time::Instant::now();

        let child = cmd.spawn().map_err(|e| IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("failed to spawn {interpreter}: {e}"),
        })?;

        let output = match tokio::time::timeout(timeout_dur, child.wait_with_output()).await {
            Ok(Ok(output)) => output,
            Ok(Err(e)) => {
                return Err(IroncladError::Tool {
                    tool: "script_runner".into(),
                    message: format!("process error: {e}"),
                });
            }
            Err(_) => {
                return Err(IroncladError::Tool {
                    tool: "script_runner".into(),
                    message: format!(
                        "script timed out after {}s",
                        self.config.script_timeout_seconds
                    ),
                });
            }
        };

        let duration_ms = start.elapsed().as_millis() as u64;
        let max = self.config.script_max_output_bytes;

        let stdout_raw = String::from_utf8_lossy(&output.stdout);
        let stderr_raw = String::from_utf8_lossy(&output.stderr);

        let stdout = truncate_str(&stdout_raw, max);
        let stderr = truncate_str(&stderr_raw, max);

        Ok(ScriptResult {
            stdout,
            stderr,
            exit_code: output.status.code().unwrap_or(-1),
            duration_ms,
        })
    }

    fn resolve_script_path(&self, requested: &Path) -> Result<std::path::PathBuf> {
        let root =
            std::fs::canonicalize(&self.config.skills_dir).map_err(|e| IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!(
                    "failed to resolve skills_dir '{}': {e}",
                    self.config.skills_dir.display()
                ),
            })?;
        let joined = if requested.is_absolute() {
            requested.to_path_buf()
        } else {
            root.join(requested)
        };
        let canonical = std::fs::canonicalize(&joined).map_err(|e| IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("failed to resolve script path '{}': {e}", joined.display()),
        })?;
        if !canonical.starts_with(&root) {
            return Err(IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!(
                    "script path '{}' escapes skills_dir '{}'",
                    canonical.display(),
                    root.display()
                ),
            });
        }
        if !canonical.is_file() {
            return Err(IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!("script path '{}' is not a file", canonical.display()),
            });
        }
        Ok(canonical)
    }
}

fn truncate_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        s.to_string()
    } else {
        let mut end = max_bytes;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        s[..end].to_string()
    }
}

fn default_home_env() -> Option<String> {
    std::env::var("HOME")
        .ok()
        .or_else(|| std::env::var("USERPROFILE").ok())
}

fn default_python_interpreter() -> &'static str {
    #[cfg(windows)]
    {
        "python"
    }
    #[cfg(not(windows))]
    {
        "python3"
    }
}

/// Determines the interpreter for a script by reading its shebang line
/// or inferring from the file extension, then checks against the whitelist.
pub fn check_interpreter(script_path: &Path, allowed: &[String]) -> Result<String> {
    if let Ok(content) = std::fs::read_to_string(script_path)
        && let Some(first_line) = content.lines().next()
        && first_line.starts_with("#!")
    {
        let shebang = first_line[2..].trim();
        let interpreter = shebang
            .split('/')
            .next_back()
            .unwrap_or(shebang)
            .split_whitespace()
            .next()
            .unwrap_or(shebang);

        let interp = if interpreter == "env" {
            shebang.split_whitespace().nth(1).unwrap_or(interpreter)
        } else {
            interpreter
        };

        if allowed.iter().any(|a| a == interp) {
            return Ok(interp.to_string());
        } else {
            return Err(IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!("interpreter '{interp}' not in whitelist: {allowed:?}"),
            });
        }
    }

    let ext = script_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("");

    let inferred = match ext {
        "py" => default_python_interpreter(),
        "sh" | "bash" => "bash",
        "js" => "node",
        _ => {
            return Err(IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!("cannot infer interpreter for extension '.{ext}'"),
            });
        }
    };

    if allowed.iter().any(|a| a == inferred) {
        Ok(inferred.to_string())
    } else {
        Err(IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("interpreter '{inferred}' not in whitelist: {allowed:?}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    fn test_config() -> SkillsConfig {
        SkillsConfig {
            script_timeout_seconds: 5,
            script_max_output_bytes: 1024,
            allowed_interpreters: vec!["bash".into(), "python3".into(), "node".into()],
            sandbox_env: true,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn successful_script_execution() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("test.sh");
        fs::write(&script, "#!/bin/bash\necho \"hello from script\"").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner.execute(Path::new("test.sh"), &[]).await.unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello from script"));
    }

    #[test]
    fn interpreter_whitelist_rejection() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("evil.rb");
        fs::write(&script, "#!/usr/bin/ruby\nputs 'hi'").unwrap();

        let allowed = vec!["bash".into(), "python3".into()];
        let result = check_interpreter(&script, &allowed);
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("not in whitelist"));
    }

    #[tokio::test]
    async fn timeout_handling() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("slow.sh");
        fs::write(&script, "#!/bin/bash\nsleep 60").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut config = test_config();
        config.script_timeout_seconds = 1;
        config.skills_dir = dir.path().to_path_buf();

        let runner = ScriptRunner::new(config);
        let result = runner.execute(Path::new("slow.sh"), &[]).await;

        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(err_msg.contains("timed out"));
    }

    #[tokio::test]
    async fn rejects_script_outside_skills_dir() {
        let skills_dir = tempfile::tempdir().unwrap();
        let outside_dir = tempfile::tempdir().unwrap();
        let script = outside_dir.path().join("escape.sh");
        fs::write(&script, "#!/bin/bash\necho hi").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = skills_dir.path().to_path_buf();

        let runner = ScriptRunner::new(cfg);
        let result = runner.execute(&script, &[]).await;
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains("escapes skills_dir"));
    }

    #[test]
    fn infer_interpreter_from_extension() {
        let dir = tempfile::tempdir().unwrap();

        let py_script = dir.path().join("test.py");
        fs::write(&py_script, "print('hi')").unwrap();

        let allowed = vec!["bash".into(), "python3".into(), "node".into()];
        assert_eq!(check_interpreter(&py_script, &allowed).unwrap(), "python3");

        let sh_script = dir.path().join("test.sh");
        fs::write(&sh_script, "echo hi").unwrap();
        assert_eq!(check_interpreter(&sh_script, &allowed).unwrap(), "bash");

        let js_script = dir.path().join("test.js");
        fs::write(&js_script, "console.log('hi')").unwrap();
        assert_eq!(check_interpreter(&js_script, &allowed).unwrap(), "node");
    }
}
