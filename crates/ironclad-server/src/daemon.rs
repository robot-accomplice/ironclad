use std::path::{Path, PathBuf};

use ironclad_core::{IroncladError, Result};

const WINDOWS_DAEMON_NAME: &str = "IroncladAgent";

pub fn launchd_plist(binary_path: &str, config_path: &str, port: u16) -> String {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/var/log".into());
    let log_dir = PathBuf::from(&home).join(".ironclad").join("logs");
    let stdout_log = log_dir.join("ironclad.stdout.log");
    let stderr_log = log_dir.join("ironclad.stderr.log");

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>Label</key>
    <string>com.ironclad.agent</string>
    <key>ProgramArguments</key>
    <array>
        <string>{binary_path}</string>
        <string>serve</string>
        <string>-c</string>
        <string>{config_path}</string>
        <string>-p</string>
        <string>{port}</string>
    </array>
    <key>RunAtLoad</key>
    <true/>
    <key>KeepAlive</key>
    <true/>
    <key>StandardOutPath</key>
    <string>{stdout}</string>
    <key>StandardErrorPath</key>
    <string>{stderr}</string>
</dict>
</plist>"#,
        binary_path = binary_path,
        config_path = config_path,
        port = port,
        stdout = stdout_log.display(),
        stderr = stderr_log.display(),
    )
}

pub fn systemd_unit(binary_path: &str, config_path: &str, port: u16) -> String {
    format!(
        r#"[Unit]
Description=Ironclad Autonomous Agent Runtime
After=network.target

[Service]
Type=simple
ExecStart={binary_path} serve -c {config_path} -p {port}
Restart=on-failure
RestartSec=5
Environment=RUST_LOG=info

[Install]
WantedBy=default.target
"#,
        binary_path = binary_path,
        config_path = config_path,
        port = port
    )
}

pub fn plist_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join("Library/LaunchAgents/com.ironclad.agent.plist")
}

pub fn systemd_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".config/systemd/user/ironclad.service")
}

fn windows_service_marker_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".ironclad")
        .join("windows-service-install.txt")
}

#[derive(Debug, Clone)]
struct WindowsDaemonInstall {
    binary: String,
    config: String,
    port: u16,
    pid: Option<u32>,
}

fn parse_windows_daemon_marker(content: &str) -> Option<WindowsDaemonInstall> {
    let mut binary = None;
    let mut config = None;
    let mut port = None;
    let mut pid = None;

    for line in content.lines() {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "binary" => binary = Some(v.trim().to_string()),
                "config" => config = Some(v.trim().to_string()),
                "port" => {
                    port = v.trim().parse::<u16>().ok();
                }
                "pid" => {
                    pid = v.trim().parse::<u32>().ok();
                }
                _ => {}
            }
        }
    }

    Some(WindowsDaemonInstall {
        binary: binary?,
        config: config?,
        port: port?,
        pid,
    })
}

fn write_windows_daemon_marker(install: &WindowsDaemonInstall) -> Result<()> {
    let marker = windows_service_marker_path();
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut content = format!(
        "name={WINDOWS_DAEMON_NAME}\nmode=user_process\nbinary={}\nconfig={}\nport={}\n",
        install.binary, install.config, install.port
    );
    if let Some(pid) = install.pid {
        content.push_str(&format!("pid={pid}\n"));
    }
    std::fs::write(&marker, content)?;
    Ok(())
}

fn read_windows_daemon_marker() -> Result<Option<WindowsDaemonInstall>> {
    let marker = windows_service_marker_path();
    if !marker.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(marker)?;
    Ok(parse_windows_daemon_marker(&content))
}

fn windows_pid_running(pid: u32) -> Result<bool> {
    if std::env::consts::OS != "windows" {
        return Ok(false);
    }
    let pid_filter = format!("PID eq {pid}");
    let out = command_output("tasklist", &["/FI", &pid_filter, "/FO", "CSV", "/NH"])?;
    if !out.status.success() {
        return Ok(false);
    }
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    Ok(stdout.contains(&format!("\"{pid}\"")))
}

fn spawn_windows_daemon_process(install: &WindowsDaemonInstall) -> Result<u32> {
    let mut cmd = std::process::Command::new(&install.binary);
    cmd.args([
        "serve",
        "-c",
        &install.config,
        "-p",
        &install.port.to_string(),
    ])
    .stdin(std::process::Stdio::null())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x00000008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    let child = cmd
        .spawn()
        .map_err(|e| IroncladError::Config(format!("failed to spawn daemon process: {e}")))?;
    Ok(child.id())
}

fn cleanup_legacy_windows_service() {
    if std::env::consts::OS != "windows" {
        return;
    }
    let _ = run_cmd("sc.exe", &["stop", WINDOWS_DAEMON_NAME]);
    let _ = run_cmd("sc.exe", &["delete", WINDOWS_DAEMON_NAME]);
}

pub fn install_daemon(binary_path: &str, config_path: &str, port: u16) -> Result<PathBuf> {
    let os = std::env::consts::OS;
    let (content, path) = match os {
        "macos" => (launchd_plist(binary_path, config_path, port), plist_path()),
        "linux" => (systemd_unit(binary_path, config_path, port), systemd_path()),
        "windows" => {
            cleanup_legacy_windows_service();
            let marker = windows_service_marker_path();
            let install = WindowsDaemonInstall {
                binary: binary_path.to_string(),
                config: config_path.to_string(),
                port,
                pid: None,
            };
            write_windows_daemon_marker(&install)?;
            return Ok(marker);
        }
        other => {
            return Err(IroncladError::Config(format!(
                "daemon install not supported on {other}"
            )));
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, &content)?;
    Ok(path)
}

pub fn start_daemon() -> Result<()> {
    let os = std::env::consts::OS;
    match os {
        "macos" => {
            let output = std::process::Command::new("launchctl")
                .args(["load", "-w"])
                .arg(plist_path())
                .output()
                .map_err(|e| IroncladError::Config(format!("failed to run launchctl: {e}")))?;

            let stderr = String::from_utf8_lossy(&output.stderr);
            if !output.status.success() {
                return Err(IroncladError::Config(format!(
                    "launchctl load failed (exit {}): {}",
                    output.status.code().unwrap_or(-1),
                    stderr.trim()
                )));
            }

            std::thread::sleep(std::time::Duration::from_secs(1));
            verify_launchd_running()?;
            Ok(())
        }
        "linux" => {
            run_cmd("systemctl", &["--user", "daemon-reload"])?;
            run_cmd(
                "systemctl",
                &["--user", "enable", "--now", "ironclad.service"],
            )
        }
        "windows" => {
            let mut install = read_windows_daemon_marker()?.ok_or_else(|| {
                IroncladError::Config("daemon not installed on windows".to_string())
            })?;
            if let Some(pid) = install.pid
                && windows_pid_running(pid)?
            {
                return Ok(());
            }
            let pid = spawn_windows_daemon_process(&install)?;
            install.pid = Some(pid);
            write_windows_daemon_marker(&install)
        }
        other => Err(IroncladError::Config(format!(
            "daemon start not supported on {other}"
        ))),
    }
}

pub fn stop_daemon() -> Result<()> {
    let os = std::env::consts::OS;
    match os {
        "macos" => run_cmd("launchctl", &["unload", &plist_path().to_string_lossy()]),
        "linux" => run_cmd("systemctl", &["--user", "stop", "ironclad.service"]),
        "windows" => {
            let mut install = match read_windows_daemon_marker()? {
                Some(i) => i,
                None => return Ok(()),
            };
            let Some(pid) = install.pid else {
                return Ok(());
            };
            let pid_s = pid.to_string();
            if windows_pid_running(pid)? {
                run_cmd("taskkill", &["/PID", &pid_s, "/T", "/F"])?;
            }
            install.pid = None;
            write_windows_daemon_marker(&install)
        }
        other => Err(IroncladError::Config(format!(
            "daemon stop not supported on {other}"
        ))),
    }
}

pub fn restart_daemon() -> Result<()> {
    let os = std::env::consts::OS;
    match os {
        "macos" => {
            if let Err(e) = stop_daemon()
                && !is_benign_stop_error(&e)
            {
                return Err(e);
            }
            start_daemon()
        }
        "linux" => run_cmd("systemctl", &["--user", "restart", "ironclad.service"]),
        "windows" => {
            if let Err(e) = stop_daemon()
                && !is_benign_stop_error(&e)
            {
                return Err(e);
            }
            start_daemon()
        }
        other => Err(IroncladError::Config(format!(
            "daemon restart not supported on {other}"
        ))),
    }
}

const LAUNCHD_LABEL: &str = "com.ironclad.agent";

fn run_cmd(program: &str, args: &[&str]) -> Result<()> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| IroncladError::Config(format!("failed to run {program}: {e}")))?;

    if output.status.success() {
        Ok(())
    } else {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = if !stderr.trim().is_empty() {
            stderr.trim().to_string()
        } else {
            stdout.trim().to_string()
        };
        Err(IroncladError::Config(format!(
            "{program} failed (exit {}): {}",
            output.status.code().unwrap_or(-1),
            detail
        )))
    }
}

fn command_output(program: &str, args: &[&str]) -> Result<std::process::Output> {
    std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| IroncladError::Config(format!("failed to run {program}: {e}")))
}

fn windows_service_exists() -> Result<bool> {
    if std::env::consts::OS != "windows" {
        return Ok(false);
    }
    let marker = windows_service_marker_path();
    Ok(marker.exists())
}

pub fn daemon_status() -> Result<String> {
    match std::env::consts::OS {
        "macos" => {
            if !is_installed() {
                return Ok("Daemon not installed".into());
            }
            match command_output("launchctl", &["list", LAUNCHD_LABEL]) {
                Ok(out) if out.status.success() => {
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    if stdout.contains("\"PID\"") {
                        Ok("Daemon running (launchd loaded)".into())
                    } else {
                        Ok("Daemon installed but not running".into())
                    }
                }
                Ok(_) => Ok("Daemon installed but not running".into()),
                Err(e) => Err(e),
            }
        }
        "linux" => {
            if !is_installed() {
                return Ok("Daemon not installed".into());
            }
            let out = command_output("systemctl", &["--user", "is-active", "ironclad.service"])?;
            if out.status.success() {
                Ok("Daemon running (systemd active)".into())
            } else {
                Ok("Daemon installed but not running".into())
            }
        }
        "windows" => {
            if !windows_service_exists()? {
                return Ok("Daemon not installed".into());
            }
            let install = read_windows_daemon_marker()?;
            match install {
                Some(i) => {
                    if let Some(pid) = i.pid
                        && windows_pid_running(pid)?
                    {
                        return Ok(format!("Daemon running (Windows process pid={pid})"));
                    }
                    Ok("Daemon installed but stopped (Windows user process)".into())
                }
                None => Ok("Daemon not installed".into()),
            }
        }
        other => Ok(format!("Daemon status unsupported on {other}")),
    }
}

fn verify_launchd_running() -> Result<()> {
    let output = std::process::Command::new("launchctl")
        .args(["list", LAUNCHD_LABEL])
        .output()
        .map_err(|e| IroncladError::Config(format!("failed to query launchctl: {e}")))?;

    if !output.status.success() {
        return Err(IroncladError::Config(
            "daemon service is not loaded — check the plist path and binary".into(),
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("\"LastExitStatus\"") {
            let code = rest
                .trim_start_matches(|c: char| !c.is_ascii_digit() && c != '-')
                .trim_end_matches(';')
                .trim();
            if code != "0" {
                let stderr_path = PathBuf::from(std::env::var("HOME").unwrap_or_default())
                    .join(".ironclad/logs/ironclad.stderr.log");
                let hint = if stderr_path.exists() {
                    format!(" (see {})", stderr_path.display())
                } else {
                    String::new()
                };
                return Err(IroncladError::Config(format!(
                    "daemon exited immediately with code {code}{hint}"
                )));
            }
        }
    }

    for line in stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("\"PID\"") {
            let pid = rest
                .trim_start_matches(|c: char| !c.is_ascii_digit())
                .trim_end_matches(';')
                .trim();
            if !pid.is_empty() {
                return Ok(());
            }
        }
    }

    Err(IroncladError::Config(
        "daemon loaded but no PID found — service may have crashed on startup".into(),
    ))
}

fn is_benign_stop_error(e: &IroncladError) -> bool {
    let msg = e.to_string().to_ascii_lowercase();
    msg.contains("1062")
        || msg.contains("service has not been started")
        || msg.contains("the service has not been started")
        || msg.contains("inactive")
        || msg.contains("not loaded")
}

fn is_installed_result() -> Result<bool> {
    let os = std::env::consts::OS;
    if os == "windows" {
        return windows_service_exists();
    }
    let path = match os {
        "macos" => plist_path(),
        "linux" => systemd_path(),
        _ => return Ok(false),
    };
    Ok(path.exists())
}

pub fn is_installed() -> bool {
    is_installed_result().unwrap_or(false)
}

pub fn uninstall_daemon() -> Result<()> {
    if !is_installed_result()? {
        return Ok(());
    }
    if let Err(e) = stop_daemon()
        && !is_benign_stop_error(&e)
    {
        return Err(e);
    }
    if std::env::consts::OS == "windows" {
        cleanup_legacy_windows_service();
        let marker = windows_service_marker_path();
        if marker.exists()
            && let Err(e) = std::fs::remove_file(&marker)
            && e.kind() != std::io::ErrorKind::NotFound
        {
            return Err(IroncladError::Config(format!(
                "failed to remove windows service marker {}: {e}",
                marker.display()
            )));
        }
        return Ok(());
    }
    let path = match std::env::consts::OS {
        "macos" => plist_path(),
        "linux" => systemd_path(),
        _ => return Ok(()),
    };
    std::fs::remove_file(&path)?;
    Ok(())
}

pub fn write_pid_file(path: &Path) -> Result<()> {
    let pid = std::process::id();
    std::fs::write(path, pid.to_string())?;
    Ok(())
}

pub fn read_pid_file(path: &Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let contents = std::fs::read_to_string(path)?;
    let pid = contents
        .trim()
        .parse::<u32>()
        .map_err(|e| IroncladError::Config(format!("invalid PID file: {e}")))?;
    Ok(Some(pid))
}

pub fn remove_pid_file(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launchd_plist_format() {
        let plist = launchd_plist("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
        assert!(plist.contains("com.ironclad.agent"));
        assert!(plist.contains("/usr/local/bin/ironclad"));
        assert!(plist.contains("/etc/ironclad.toml"));
        assert!(plist.contains("18789"));
        assert!(plist.contains("KeepAlive"));
    }

    #[test]
    fn systemd_unit_format() {
        let unit = systemd_unit("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
        assert!(unit.contains("ExecStart="));
        assert!(unit.contains("/usr/local/bin/ironclad"));
        assert!(unit.contains("Restart=on-failure"));
        assert!(unit.contains("[Install]"));
    }

    #[test]
    fn pid_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("test.pid");

        write_pid_file(&pid_path).unwrap();
        let pid = read_pid_file(&pid_path).unwrap();
        assert!(pid.is_some());
        assert_eq!(pid.unwrap(), std::process::id());

        remove_pid_file(&pid_path).unwrap();
        assert!(!pid_path.exists());
    }

    #[test]
    fn read_missing_pid_file() {
        let result = read_pid_file(Path::new("/nonexistent/pid"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[test]
    fn remove_missing_pid_file() {
        let result = remove_pid_file(Path::new("/nonexistent/pid"));
        assert!(result.is_ok());
    }

    #[test]
    fn plist_path_is_under_launch_agents() {
        let path = plist_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("LaunchAgents"));
        assert!(path_str.ends_with("com.ironclad.agent.plist"));
    }

    #[test]
    fn systemd_path_is_under_systemd_user() {
        let path = systemd_path();
        let path_str = path.to_string_lossy();
        assert!(path_str.contains("systemd/user"));
        assert!(path_str.ends_with("ironclad.service"));
    }

    #[test]
    fn read_pid_file_with_invalid_content_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("bad.pid");
        std::fs::write(&pid_path, "not-a-number").unwrap();
        assert!(read_pid_file(&pid_path).is_err());
    }

    #[test]
    fn read_pid_file_with_whitespace_trims() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("ws.pid");
        std::fs::write(&pid_path, "  12345  \n").unwrap();
        let result = read_pid_file(&pid_path).unwrap();
        assert_eq!(result, Some(12345));
    }

    #[test]
    fn launchd_plist_is_valid_xml() {
        let plist = launchd_plist("/usr/bin/ironclad", "/etc/ironclad.toml", 9999);
        assert!(plist.starts_with("<?xml"));
        assert!(plist.contains("<plist version=\"1.0\">"));
        assert!(plist.contains("</plist>"));
        assert!(plist.contains("<string>9999</string>"));
        assert!(plist.contains("<string>serve</string>"));
    }

    #[test]
    fn systemd_unit_has_required_sections() {
        let unit = systemd_unit("/usr/bin/ironclad", "/etc/ironclad.toml", 8080);
        assert!(unit.contains("[Unit]"));
        assert!(unit.contains("[Service]"));
        assert!(unit.contains("[Install]"));
        assert!(unit.contains("ExecStart=/usr/bin/ironclad serve -c /etc/ironclad.toml -p 8080"));
        assert!(unit.contains("Type=simple"));
    }

    #[test]
    fn install_daemon_creates_file() {
        if std::env::consts::OS == "windows" {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("ironclad");
        std::fs::write(&bin, "").unwrap();
        let cfg = dir.path().join("ironclad.toml");
        std::fs::write(&cfg, "").unwrap();

        let result = install_daemon(bin.to_str().unwrap(), cfg.to_str().unwrap(), 18789);
        assert!(result.is_ok());
        let path = result.unwrap();
        assert!(path.exists());
    }

    #[test]
    fn write_and_read_pid_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("test.pid");
        write_pid_file(&pid_path).unwrap();
        assert!(pid_path.exists());
        let pid = read_pid_file(&pid_path).unwrap().unwrap();
        assert_eq!(pid, std::process::id());
        remove_pid_file(&pid_path).unwrap();
        assert!(!pid_path.exists());
    }

    #[test]
    fn parse_windows_daemon_marker_basic() {
        let input = "name=IroncladAgent\nmode=user_process\nbinary=C:\\x\\ironclad.exe\nconfig=C:\\x\\ironclad.toml\nport=18789\npid=1234\n";
        let parsed = parse_windows_daemon_marker(input).unwrap();
        assert_eq!(parsed.binary, "C:\\x\\ironclad.exe");
        assert_eq!(parsed.config, "C:\\x\\ironclad.toml");
        assert_eq!(parsed.port, 18789);
        assert_eq!(parsed.pid, Some(1234));
    }

    #[test]
    fn parse_windows_daemon_marker_without_pid() {
        let input = "name=IroncladAgent\nmode=user_process\nbinary=C:\\x\\ironclad.exe\nconfig=C:\\x\\ironclad.toml\nport=18789\n";
        let parsed = parse_windows_daemon_marker(input).unwrap();
        assert_eq!(parsed.binary, "C:\\x\\ironclad.exe");
        assert_eq!(parsed.config, "C:\\x\\ironclad.toml");
        assert_eq!(parsed.port, 18789);
        assert_eq!(parsed.pid, None);
    }
}
