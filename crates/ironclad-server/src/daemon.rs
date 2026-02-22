use std::path::{Path, PathBuf};

use ironclad_core::{IroncladError, Result};

pub fn launchd_plist(binary_path: &str, config_path: &str, port: u16) -> String {
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
    <string>/tmp/ironclad.stdout.log</string>
    <key>StandardErrorPath</key>
    <string>/tmp/ironclad.stderr.log</string>
</dict>
</plist>"#,
        binary_path = binary_path,
        config_path = config_path,
        port = port
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

pub fn install_daemon(binary_path: &str, config_path: &str, port: u16) -> Result<PathBuf> {
    let os = std::env::consts::OS;
    let (content, path) = match os {
        "macos" => (launchd_plist(binary_path, config_path, port), plist_path()),
        "linux" => (systemd_unit(binary_path, config_path, port), systemd_path()),
        other => {
            return Err(IroncladError::Config(format!(
                "daemon install not supported on {other}"
            )));
        }
    };

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    std::fs::write(&path, content)?;
    Ok(path)
}

pub fn uninstall_daemon() -> Result<()> {
    let os = std::env::consts::OS;
    let path = match os {
        "macos" => plist_path(),
        "linux" => systemd_path(),
        other => {
            return Err(IroncladError::Config(format!(
                "daemon uninstall not supported on {other}"
            )));
        }
    };

    if path.exists() {
        std::fs::remove_file(&path)?;
    }

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
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("ironclad");
        std::fs::write(&bin, "").unwrap();
        let cfg = dir.path().join("ironclad.toml");
        std::fs::write(&cfg, "").unwrap();

        let result = install_daemon(
            bin.to_str().unwrap(),
            cfg.to_str().unwrap(),
            18789,
        );
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
}
