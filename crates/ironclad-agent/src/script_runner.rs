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
        let max = self.config.script_max_output_bytes;
        let max_capture = (max as u64).saturating_add(1);

        let mut child = cmd.spawn().map_err(|e| IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("failed to spawn {interpreter}: {e}"),
        })?;
        let stdout = child.stdout.take().ok_or_else(|| IroncladError::Tool {
            tool: "script_runner".into(),
            message: "failed to capture script stdout".into(),
        })?;
        let stderr = child.stderr.take().ok_or_else(|| IroncladError::Tool {
            tool: "script_runner".into(),
            message: "failed to capture script stderr".into(),
        })?;
        let stdout_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let _ = stdout.take(max_capture).read_to_end(&mut buf).await;
            buf
        });
        let stderr_task = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut buf = Vec::new();
            let _ = stderr.take(max_capture).read_to_end(&mut buf).await;
            buf
        });

        let status = match tokio::time::timeout(timeout_dur, child.wait()).await {
            Ok(Ok(status)) => status,
            Ok(Err(e)) => {
                return Err(IroncladError::Tool {
                    tool: "script_runner".into(),
                    message: format!("process error: {e}"),
                });
            }
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
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
        let stdout_bytes = stdout_task.await.unwrap_or_default();
        let stderr_bytes = stderr_task.await.unwrap_or_default();
        let stdout_raw = String::from_utf8_lossy(&stdout_bytes);
        let stderr_raw = String::from_utf8_lossy(&stderr_bytes);

        let stdout = truncate_str(&stdout_raw, max);
        let stderr = truncate_str(&stderr_raw, max);

        Ok(ScriptResult {
            stdout,
            stderr,
            exit_code: status.code().unwrap_or(-1),
            duration_ms,
        })
    }

    /// Resolve a requested script path under the configured skills root.
    ///
    /// This canonicalizes both root and script path and enforces containment.
    pub fn resolve_script_path(&self, requested: &Path) -> Result<std::path::PathBuf> {
        if requested.is_absolute() {
            return Err(IroncladError::Config(
                "absolute script paths are not allowed".into(),
            ));
        }

        let root =
            std::fs::canonicalize(&self.config.skills_dir).map_err(|e| IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!(
                    "failed to resolve skills_dir '{}': {e}",
                    self.config.skills_dir.display()
                ),
            })?;
        let joined = root.join(requested);
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

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let metadata = std::fs::metadata(&canonical).map_err(|e| IroncladError::Tool {
                tool: "script_runner".into(),
                message: format!("failed to read metadata for '{}': {e}", canonical.display()),
            })?;
            let mode = metadata.permissions().mode();
            if mode & 0o002 != 0 {
                return Err(IroncladError::Tool {
                    tool: "script_runner".into(),
                    message: format!(
                        "script '{}' is world-writable (mode {:o})",
                        canonical.display(),
                        mode
                    ),
                });
            }
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
    if let Ok(first_line) = std::fs::File::open(script_path).and_then(|f| {
        use std::io::{BufRead, Read};
        let mut line = String::new();
        std::io::BufReader::new(f.take(512)).read_line(&mut line)?;
        Ok(line)
    }) && first_line.starts_with("#!")
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
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

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
    async fn rejects_absolute_script_path() {
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
        assert!(msg.contains("absolute script paths are not allowed"));
    }

    #[test]
    fn infer_interpreter_from_extension() {
        let dir = tempfile::tempdir().unwrap();

        let py_script = dir.path().join("test.py");
        fs::write(&py_script, "print('hi')").unwrap();

        #[cfg(windows)]
        let allowed = vec![
            "bash".to_string(),
            "python".to_string(),
            "python3".to_string(),
            "node".to_string(),
        ];
        #[cfg(not(windows))]
        let allowed = vec![
            "bash".to_string(),
            "python3".to_string(),
            "node".to_string(),
        ];
        #[cfg(windows)]
        assert_eq!(check_interpreter(&py_script, &allowed).unwrap(), "python");
        #[cfg(not(windows))]
        assert_eq!(check_interpreter(&py_script, &allowed).unwrap(), "python3");

        let sh_script = dir.path().join("test.sh");
        fs::write(&sh_script, "echo hi").unwrap();
        assert_eq!(check_interpreter(&sh_script, &allowed).unwrap(), "bash");

        let js_script = dir.path().join("test.js");
        fs::write(&js_script, "console.log('hi')").unwrap();
        assert_eq!(check_interpreter(&js_script, &allowed).unwrap(), "node");
    }

    #[test]
    fn check_interpreter_env_shebang() {
        // #!/usr/bin/env python3 -> should parse "python3"
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("env_shebang.py");
        fs::write(&script, "#!/usr/bin/env python3\nprint('hi')").unwrap();
        let allowed = vec!["python3".to_string()];
        let interp = check_interpreter(&script, &allowed).unwrap();
        assert_eq!(interp, "python3");
    }

    #[test]
    fn check_interpreter_env_shebang_not_allowed() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("env_ruby.rb");
        fs::write(&script, "#!/usr/bin/env ruby\nputs 'hi'").unwrap();
        let allowed = vec!["python3".to_string(), "bash".to_string()];
        let result = check_interpreter(&script, &allowed);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not in whitelist"));
    }

    #[test]
    fn check_interpreter_unknown_extension() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("test.xyz");
        fs::write(&script, "some content").unwrap();
        let allowed = vec!["bash".to_string()];
        let result = check_interpreter(&script, &allowed);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("cannot infer interpreter")
        );
    }

    #[test]
    fn check_interpreter_bash_extension() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("test.bash");
        fs::write(&script, "echo hi").unwrap();
        let allowed = vec!["bash".to_string()];
        let interp = check_interpreter(&script, &allowed).unwrap();
        assert_eq!(interp, "bash");
    }

    #[test]
    fn world_writable_script_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("writable.sh");
        fs::write(&script, "#!/bin/bash\necho hi").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o777)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner.resolve_script_path(Path::new("writable.sh"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("world-writable"));
    }

    #[test]
    fn resolve_rejects_directory_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);

        // Attempting to escape skills_dir with ../
        let result = runner.resolve_script_path(Path::new("../../etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_rejects_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);

        let result = runner.resolve_script_path(Path::new("/etc/passwd"));
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("absolute script paths")
        );
    }

    #[test]
    fn truncate_str_within_limit() {
        let s = "hello world";
        assert_eq!(truncate_str(s, 100), "hello world");
    }

    #[test]
    fn truncate_str_at_limit() {
        let s = "hello";
        assert_eq!(truncate_str(s, 5), "hello");
    }

    #[test]
    fn truncate_str_beyond_limit() {
        let s = "hello world";
        let truncated = truncate_str(s, 5);
        assert_eq!(truncated, "hello");
    }

    #[test]
    fn truncate_str_multibyte_boundary() {
        // "é" is 2 bytes in UTF-8; truncating at odd boundary should back up
        let s = "café";
        let truncated = truncate_str(s, 4);
        // "caf" is 3 bytes, "é" is 2 bytes (bytes 3-4)
        // truncating at 4 lands in the middle of é, should back up to 3
        assert_eq!(truncated, "caf");
    }

    #[tokio::test]
    async fn script_with_args() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("args.sh");
        fs::write(&script, "#!/bin/bash\necho \"$1 $2\"").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner
            .execute(Path::new("args.sh"), &["hello", "world"])
            .await
            .unwrap();

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("hello world"));
    }

    #[tokio::test]
    async fn script_nonzero_exit_code() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("fail.sh");
        fs::write(&script, "#!/bin/bash\nexit 42").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner.execute(Path::new("fail.sh"), &[]).await.unwrap();

        assert_eq!(result.exit_code, 42);
    }

    #[tokio::test]
    async fn script_output_truncation() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("verbose.sh");
        // Generate output > max_output_bytes (set to 1024 in test_config)
        fs::write(&script, "#!/bin/bash\nfor i in $(seq 1 500); do echo \"line $i with some padding text to fill up space\"; done").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner.execute(Path::new("verbose.sh"), &[]).await.unwrap();

        assert!(
            result.stdout.len() <= 1024,
            "stdout should be truncated to max_output_bytes"
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn sandbox_env_strips_secrets() {
        let _env_guard = ENV_LOCK.lock().expect("env lock poisoned");
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("print_secret.sh");
        fs::write(
            &script,
            "#!/bin/bash\nprintf \"%s\" \"${OPENAI_API_KEY:-MISSING}\"",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        // This variable should never leak to script env when sandbox_env=true.
        // SAFETY: test-only env mutation is serialized via ENV_LOCK.
        unsafe {
            std::env::set_var("OPENAI_API_KEY", "top-secret-test-value");
        }

        let mut cfg = test_config();
        cfg.sandbox_env = true;
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner
            .execute(Path::new("print_secret.sh"), &[])
            .await
            .expect("script should execute");

        assert_eq!(result.exit_code, 0);
        assert_eq!(
            result.stdout.trim(),
            "MISSING",
            "sandboxed script must not inherit secret env vars"
        );
        // SAFETY: test-only env mutation is serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var("OPENAI_API_KEY");
        }
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn sandbox_env_keeps_minimal_runtime_vars_only() {
        let _env_guard = ENV_LOCK.lock().expect("env lock poisoned");
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("print_env_subset.sh");
        fs::write(
            &script,
            "#!/bin/bash\nprintf \"PATH=%s\\nHOME=%s\\nTMP=%s\\nLANG=%s\\nTOKEN=%s\" \"${PATH:-}\" \"${HOME:-}\" \"${TMP:-}\" \"${LANG:-}\" \"${SECRET_TOKEN:-MISSING}\"",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        // SAFETY: test-only env mutation is serialized via ENV_LOCK.
        unsafe {
            std::env::set_var("SECRET_TOKEN", "definitely-secret");
            std::env::set_var("LANG", "en_US.UTF-8");
        }

        let mut cfg = test_config();
        cfg.sandbox_env = true;
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg);
        let result = runner
            .execute(Path::new("print_env_subset.sh"), &[])
            .await
            .expect("script should execute");

        assert_eq!(result.exit_code, 0);
        assert!(result.stdout.contains("PATH="));
        assert!(result.stdout.contains("HOME="));
        assert!(result.stdout.contains("TMP="));
        assert!(result.stdout.contains("LANG=en_US.UTF-8"));
        assert!(
            result.stdout.ends_with("TOKEN=MISSING"),
            "non-allowlisted secrets must not be present"
        );
        // SAFETY: test-only env mutation is serialized via ENV_LOCK.
        unsafe {
            std::env::remove_var("SECRET_TOKEN");
            std::env::remove_var("LANG");
        }
    }
}
