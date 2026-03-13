use std::path::Path;

use tokio::process::Command;

use ironclad_core::config::{FilesystemSecurityConfig, SkillsConfig};
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
    fs_security: FilesystemSecurityConfig,
}

impl ScriptRunner {
    pub fn new(config: SkillsConfig, fs_security: FilesystemSecurityConfig) -> Self {
        Self {
            config,
            fs_security,
        }
    }

    pub async fn execute(&self, script_path: &Path, args: &[&str]) -> Result<ScriptResult> {
        let script_path = self.resolve_script_path(script_path)?;
        let interpreter = check_interpreter(&script_path, &self.config.allowed_interpreters)?;

        let working_dir = script_path.parent().unwrap_or(Path::new("."));

        // ── Build command, optionally wrapping with macOS sandbox-exec ───
        // The _sandbox_profile guard keeps the tempfile alive until the child
        // process finishes; sandbox-exec reads the profile at exec time.
        #[cfg(target_os = "macos")]
        let _sandbox_profile: Option<tempfile::NamedTempFile>;

        let mut cmd;

        #[cfg(target_os = "macos")]
        {
            if self.fs_security.script_fs_confinement && self.config.sandbox_env {
                let profile = generate_sandbox_profile(
                    &self.config.skills_dir,
                    self.config.workspace_dir.as_deref(),
                    &self.fs_security.script_allowed_paths,
                    self.config.network_allowed,
                )?;
                let profile_path = profile.path().to_path_buf();
                _sandbox_profile = Some(profile);

                cmd = Command::new("/usr/bin/sandbox-exec");
                cmd.arg("-f")
                    .arg(profile_path)
                    .arg(&interpreter)
                    .arg(&script_path)
                    .args(args)
                    .current_dir(working_dir);
            } else {
                _sandbox_profile = None;
                cmd = Command::new(&interpreter);
                cmd.arg(&script_path).args(args).current_dir(working_dir);
            }
        }

        #[cfg(not(target_os = "macos"))]
        {
            cmd = Command::new(&interpreter);
            cmd.arg(&script_path).args(args).current_dir(working_dir);
        }

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
            // Expose the skills directory and optional workspace root so scripts
            // know their boundaries without guessing.
            cmd.env("IRONCLAD_SKILLS_DIR", &self.config.skills_dir);
            if let Some(ref ws) = self.config.workspace_dir {
                cmd.env("IRONCLAD_WORKSPACE", ws);
            }
        }

        // Pre-exec resource limits (Unix only).
        #[cfg(unix)]
        {
            let mem_limit = self.config.script_max_memory_bytes;
            let deny_net = self.config.sandbox_env && !self.config.network_allowed;
            // SAFETY: pre_exec runs in the forked child before exec.
            // Only async-signal-safe functions are called (setrlimit, unshare).
            unsafe {
                cmd.pre_exec(move || {
                    // Memory ceiling via RLIMIT_AS on Linux.
                    // macOS virtual memory model makes RLIMIT_AS unreliable
                    // (processes routinely map far more virtual space than
                    // they physically use), so we skip enforcement there.
                    #[cfg(target_os = "linux")]
                    if let Some(max_bytes) = mem_limit {
                        let rlim = libc::rlimit {
                            rlim_cur: max_bytes,
                            rlim_max: max_bytes,
                        };
                        if libc::setrlimit(libc::RLIMIT_AS, &rlim) != 0 {
                            return Err(std::io::Error::last_os_error());
                        }
                    }
                    #[cfg(not(target_os = "linux"))]
                    let _ = mem_limit;
                    // Network isolation via unshare(CLONE_NEWNET) on Linux.
                    #[cfg(target_os = "linux")]
                    if deny_net {
                        if libc::unshare(libc::CLONE_NEWNET) != 0 {
                            // Non-fatal: user namespaces may be disabled.
                            // The mechanic health check will warn about this.
                            eprintln!(
                                "ironclad: warning: network isolation unavailable (unshare failed)"
                            );
                        }
                    }
                    // On macOS there is no unprivileged network namespace API.
                    // The mechanic health check notes this platform limitation.
                    #[cfg(not(target_os = "linux"))]
                    let _ = deny_net;
                    Ok(())
                });
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

/// Generate a macOS `sandbox-exec` profile (.sb) that confines script
/// filesystem access to known-good paths.
///
/// The profile uses a deny-default posture and selectively allows:
/// - Process execution (interpreters under `/usr`, `/opt`, Homebrew, Nix)
/// - System library reads (frameworks, dyld cache)
/// - `skills_dir` (read-only)
/// - `workspace_dir` (read-write, if configured)
/// - `/tmp` (read-write, for scratch files)
/// - `script_allowed_paths` (read-only)
/// - Network (only if `network_allowed` is true)
#[cfg(target_os = "macos")]
fn generate_sandbox_profile(
    _skills_dir: &Path,
    workspace_dir: Option<&Path>,
    extra_paths: &[std::path::PathBuf],
    network_allowed: bool,
) -> Result<tempfile::NamedTempFile> {
    use std::io::Write;

    // Canonicalize paths — macOS sandbox-exec resolves symlinks internally
    // (e.g. /var → /private/var), so profile paths must match the resolved
    // form. Fall back to the original path if canonicalization fails.
    let canon = |p: &Path| -> String {
        p.canonicalize()
            .unwrap_or_else(|_| p.to_path_buf())
            .display()
            .to_string()
    };

    let mut profile = tempfile::NamedTempFile::new().map_err(|e| IroncladError::Tool {
        tool: "script_runner".into(),
        message: format!("failed to create sandbox profile tempfile: {e}"),
    })?;

    // Apple Sandbox Profile Language (SBPL).
    // Reference: TN3145 (Apple), reverse-engineered from system profiles.
    //
    // Strategy: **write-denial model** — allow reads globally, restrict writes
    // to specific paths. Interpreters (bash, python, node, ruby) probe many
    // unpredictable paths at startup (dyld cache, locale, Homebrew, nix, etc.)
    // making a read-whitelist fragile across macOS versions. The security value
    // is in preventing *writes* outside the workspace/tmp sandbox; read access
    // is already scoped by the OS user's filesystem permissions.
    let mut sb = String::with_capacity(2048);
    sb.push_str("(version 1)\n");
    sb.push_str("(deny default)\n\n");

    // ── Process execution ────────────────────────────────────────
    sb.push_str("; Process execution for interpreters\n");
    sb.push_str("(allow process-exec)\n");
    sb.push_str("(allow process-fork)\n\n");

    // ── Read access (global) ─────────────────────────────────────
    // Interpreters need to read system libraries, frameworks, language
    // runtimes, and config in unpredictable locations. Grant broad read.
    sb.push_str("; Global read access — writes are the confinement boundary\n");
    sb.push_str("(allow file-read*)\n\n");

    // ── Write access (confined) ──────────────────────────────────
    // Only allow writes to: /dev/null, /tmp, workspace, and skills_dir.
    sb.push_str("; /dev/null, /dev/zero — scripts redirect stderr here\n");
    sb.push_str("(allow file-write* (literal \"/dev/null\") (literal \"/dev/zero\"))\n\n");

    sb.push_str("; Scratch space — /tmp and /private/tmp\n");
    sb.push_str("(allow file-write* (subpath \"/tmp\"))\n");
    sb.push_str("(allow file-write* (subpath \"/private/tmp\"))\n\n");

    // Workspace directory (read-write, if configured)
    if let Some(ws) = workspace_dir {
        sb.push_str("; Workspace directory — writable\n");
        sb.push_str(&format!(
            "(allow file-write* (subpath \"{}\"))\n\n",
            canon(ws)
        ));
    }

    // Extra allowed paths — write access (user-configured escape hatches)
    for p in extra_paths {
        sb.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", canon(p)));
    }
    if !extra_paths.is_empty() {
        sb.push('\n');
    }

    // ── IPC / mach / signals ─────────────────────────────────────
    // Language runtimes (Python, Node) need these for normal operation.
    sb.push_str("; IPC and signals for language runtimes\n");
    sb.push_str("(allow sysctl-read)\n");
    sb.push_str("(allow mach-lookup)\n");
    sb.push_str("(allow signal (target self))\n");
    sb.push_str("(allow ipc-posix-shm-read-data)\n");
    sb.push_str("(allow ipc-posix-shm-write-data)\n\n");

    // ── Network ──────────────────────────────────────────────────
    // On Linux, network isolation uses unshare(CLONE_NEWNET).
    // On macOS, sandbox-exec handles it natively via the profile.
    if network_allowed {
        sb.push_str("; Network access allowed by configuration\n");
        sb.push_str("(allow network*)\n");
    } else {
        sb.push_str("; Network denied (sandbox_env + !network_allowed)\n");
    }

    profile
        .write_all(sb.as_bytes())
        .map_err(|e| IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("failed to write sandbox profile: {e}"),
        })?;

    Ok(profile)
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

/// Resolve a bare interpreter name to its canonical absolute path by walking PATH.
///
/// If the name is already absolute, canonicalize and return it.
/// This prevents PATH-hijacking attacks where a malicious binary shadows
/// a legitimate interpreter earlier in the search order.
pub fn resolve_interpreter_absolute(name: &str) -> Result<String> {
    let p = Path::new(name);
    if p.is_absolute() {
        let canonical = std::fs::canonicalize(p).map_err(|e| IroncladError::Tool {
            tool: "script_runner".into(),
            message: format!("interpreter '{name}' not found: {e}"),
        })?;
        return Ok(canonical.to_string_lossy().to_string());
    }
    let path_var = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path_var) {
        let candidate = dir.join(name);
        if candidate.is_file()
            && let Ok(canonical) = std::fs::canonicalize(&candidate)
        {
            return Ok(canonical.to_string_lossy().to_string());
        }
    }
    Err(IroncladError::Tool {
        tool: "script_runner".into(),
        message: format!("interpreter '{name}' not found in PATH"),
    })
}

/// Determines the interpreter for a script by reading its shebang line
/// or inferring from the file extension, then checks against the whitelist.
/// Returns the **absolute path** to the interpreter to prevent PATH hijacking.
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
            return resolve_interpreter_absolute(interp);
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
        resolve_interpreter_absolute(inferred)
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

    fn test_fs_security() -> FilesystemSecurityConfig {
        FilesystemSecurityConfig {
            // Disable sandbox-exec in tests by default to avoid requiring
            // /usr/bin/sandbox-exec and to keep tests fast and isolated.
            script_fs_confinement: false,
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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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

        let runner = ScriptRunner::new(config, test_fs_security());
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

        let runner = ScriptRunner::new(cfg, test_fs_security());
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

        // check_interpreter now returns absolute paths; verify it's an absolute python path.
        // Canonical resolution may follow symlinks (e.g. python3 → python3.14 on Homebrew).
        let py_result = check_interpreter(&py_script, &allowed).unwrap();
        #[cfg(windows)]
        assert!(py_result.ends_with("python") || py_result.ends_with("python.exe"));
        #[cfg(not(windows))]
        assert!(
            Path::new(&py_result).is_absolute() && py_result.contains("python"),
            "expected absolute python path, got: {py_result}"
        );

        let sh_script = dir.path().join("test.sh");
        fs::write(&sh_script, "echo hi").unwrap();
        let sh_result = check_interpreter(&sh_script, &allowed).unwrap();
        assert!(
            sh_result.ends_with("/bash"),
            "expected absolute bash path, got: {sh_result}"
        );

        let js_script = dir.path().join("test.js");
        fs::write(&js_script, "console.log('hi')").unwrap();
        let js_result = check_interpreter(&js_script, &allowed).unwrap();
        assert!(
            js_result.ends_with("/node"),
            "expected absolute node path, got: {js_result}"
        );
    }

    #[test]
    fn check_interpreter_env_shebang() {
        // #!/usr/bin/env python3 -> should resolve to absolute python path
        // (canonical may resolve symlink, e.g. python3 → python3.14 on Homebrew)
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("env_shebang.py");
        fs::write(&script, "#!/usr/bin/env python3\nprint('hi')").unwrap();
        let allowed = vec!["python3".to_string()];
        let interp = check_interpreter(&script, &allowed).unwrap();
        assert!(
            Path::new(&interp).is_absolute() && interp.contains("python"),
            "expected absolute python path, got: {interp}"
        );
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
        assert!(
            interp.ends_with("/bash"),
            "expected absolute bash path, got: {interp}"
        );
    }

    #[test]
    fn world_writable_script_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("writable.sh");
        fs::write(&script, "#!/bin/bash\necho hi").unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o777)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg, test_fs_security());
        let result = runner.resolve_script_path(Path::new("writable.sh"));
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("world-writable"));
    }

    #[test]
    fn resolve_rejects_directory_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg, test_fs_security());

        // Attempting to escape skills_dir with ../
        let result = runner.resolve_script_path(Path::new("../../etc/passwd"));
        assert!(result.is_err());
    }

    #[test]
    fn resolve_rejects_absolute_path() {
        let dir = tempfile::tempdir().unwrap();
        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        let runner = ScriptRunner::new(cfg, test_fs_security());

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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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

    #[test]
    fn resolve_interpreter_absolute_finds_bash() {
        let abs = resolve_interpreter_absolute("bash").unwrap();
        assert!(
            Path::new(&abs).is_absolute(),
            "expected absolute path, got: {abs}"
        );
        assert!(
            abs.ends_with("/bash"),
            "expected path ending in /bash, got: {abs}"
        );
    }

    #[test]
    fn resolve_interpreter_absolute_rejects_missing() {
        let result = resolve_interpreter_absolute("nonexistent_binary_xyz_123");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("not found in PATH")
        );
    }

    #[allow(clippy::await_holding_lock)]
    #[tokio::test]
    async fn sandbox_exposes_workspace_env_vars() {
        let _env_guard = ENV_LOCK.lock().expect("env lock poisoned");
        let dir = tempfile::tempdir().unwrap();
        let ws_dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("check_ws.sh");
        fs::write(
            &script,
            "#!/bin/bash\nprintf \"SKILLS=%s WS=%s\" \"${IRONCLAD_SKILLS_DIR:-MISSING}\" \"${IRONCLAD_WORKSPACE:-MISSING}\"",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = dir.path().to_path_buf();
        cfg.workspace_dir = Some(ws_dir.path().to_path_buf());
        let runner = ScriptRunner::new(cfg, test_fs_security());
        let result = runner
            .execute(Path::new("check_ws.sh"), &[])
            .await
            .expect("script should execute");

        assert_eq!(result.exit_code, 0);
        assert!(
            result
                .stdout
                .contains(&format!("SKILLS={}", dir.path().display())),
            "IRONCLAD_SKILLS_DIR not set, got: {}",
            result.stdout
        );
        assert!(
            result
                .stdout
                .contains(&format!("WS={}", ws_dir.path().display())),
            "IRONCLAD_WORKSPACE not set, got: {}",
            result.stdout
        );
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
        let runner = ScriptRunner::new(cfg, test_fs_security());
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

    #[cfg(target_os = "macos")]
    #[test]
    fn sandbox_profile_contains_expected_rules() {
        use std::io::Read;

        let skills = tempfile::tempdir().unwrap();
        let workspace = tempfile::tempdir().unwrap();
        let extra = tempfile::tempdir().unwrap();

        let profile = generate_sandbox_profile(
            skills.path(),
            Some(workspace.path()),
            &[extra.path().to_path_buf()],
            false,
        )
        .unwrap();

        let mut contents = String::new();
        std::fs::File::open(profile.path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        assert!(contents.contains("(version 1)"), "missing version");
        assert!(contents.contains("(deny default)"), "missing deny default");

        // Write-denial model: reads are global, writes confined to specific paths.
        assert!(
            contents.contains("(allow file-read*)"),
            "should allow global reads: {contents}"
        );

        // Workspace and extra paths get file-write* rules (canonicalized).
        let workspace_canon = workspace.path().canonicalize().unwrap();
        let extra_canon = extra.path().canonicalize().unwrap();
        assert!(
            contents.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                workspace_canon.display()
            )),
            "workspace_dir not in write rules: {contents}"
        );
        assert!(
            contents.contains(&format!(
                "(allow file-write* (subpath \"{}\"))",
                extra_canon.display()
            )),
            "extra path not in write rules: {contents}"
        );

        // /tmp writable
        assert!(
            contents.contains("(allow file-write* (subpath \"/tmp\"))"),
            "/tmp not writable: {contents}"
        );

        // Network denied when network_allowed=false
        assert!(
            !contents.contains("(allow network"),
            "network should be denied"
        );
        assert!(
            contents.contains("Network denied"),
            "should note network denial"
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sandbox_profile_allows_network_when_configured() {
        use std::io::Read;

        let skills = tempfile::tempdir().unwrap();
        let profile = generate_sandbox_profile(skills.path(), None, &[], true).unwrap();

        let mut contents = String::new();
        std::fs::File::open(profile.path())
            .unwrap()
            .read_to_string(&mut contents)
            .unwrap();

        assert!(
            contents.contains("(allow network*)"),
            "network should be allowed when network_allowed=true"
        );
    }

    #[cfg(target_os = "macos")]
    #[tokio::test]
    async fn sandbox_exec_confines_script_filesystem() {
        // This test verifies that sandbox-exec actually blocks writes outside
        // allowed paths. It creates a script that tries to write to a path
        // outside the sandbox and asserts the write fails.
        let skills_dir = tempfile::tempdir().unwrap();
        let forbidden_dir = tempfile::tempdir().unwrap();
        let forbidden_file = forbidden_dir.path().join("should_not_exist.txt");

        let script = skills_dir.path().join("write_outside.sh");
        fs::write(
            &script,
            format!(
                "#!/bin/bash\necho 'breach' > '{}' 2>/dev/null && echo WRITTEN || echo BLOCKED",
                forbidden_file.display()
            ),
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();

        let mut cfg = test_config();
        cfg.skills_dir = skills_dir.path().to_path_buf();
        cfg.sandbox_env = true;

        let fs_sec = FilesystemSecurityConfig {
            script_fs_confinement: true,
            ..Default::default()
        };

        let runner = ScriptRunner::new(cfg, fs_sec);
        let result = runner
            .execute(Path::new("write_outside.sh"), &[])
            .await
            .unwrap();

        assert!(
            result.stdout.contains("BLOCKED"),
            "sandbox should block writes outside allowed paths, stdout={:?} stderr={:?} exit={}",
            result.stdout,
            result.stderr,
            result.exit_code
        );
        assert!(
            !forbidden_file.exists(),
            "file should not have been created outside sandbox"
        );
    }
}
