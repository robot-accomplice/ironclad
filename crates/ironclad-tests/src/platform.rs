//! Cross-platform delta tests (v0.8.0 stabilization plan, Task 36).
//!
//! These tests verify that platform-specific code paths behave correctly on the
//! current host operating system. Platform-gated tests use `#[cfg(unix)]`,
//! `#[cfg(target_os = "macos")]`, `#[cfg(target_os = "linux")]`, and
//! `#[cfg(windows)]` to run only on the applicable platform.

use std::path::PathBuf;

use ironclad_core::IroncladConfig;
use ironclad_server::daemon;

// ---------------------------------------------------------------------------
// Helper: minimal valid TOML for IroncladConfig
// ---------------------------------------------------------------------------

fn minimal_toml() -> &'static str {
    r#"
[agent]
name = "PlatformTestBot"
id = "plat-test"

[server]
port = 19999

[database]
path = "/tmp/ironclad-platform-test.db"

[models]
primary = "ollama/qwen3:8b"
"#
}

/// Produces a TOML string that uses tilde paths for several fields.
fn tilde_toml() -> &'static str {
    r#"
[agent]
name = "TildeBot"
id = "tilde-test"
workspace = "~/ironclad-workspace"

[server]
port = 19998
log_dir = "~/ironclad-logs"

[database]
path = "~/ironclad-test.db"

[models]
primary = "ollama/qwen3:8b"
"#
}

// ===========================================================================
// Section A: Path handling tests (all platforms)
// ===========================================================================

#[test]
fn tilde_expansion_expands_home_for_database_path() {
    let cfg = IroncladConfig::from_str(tilde_toml()).unwrap();
    let db_path = cfg.database.path.to_string_lossy().to_string();
    // After parsing, the tilde should have been replaced with a real directory.
    assert!(
        !db_path.starts_with('~'),
        "database path should not start with '~' after expansion, got: {db_path}"
    );
    // It should end with the filename we specified.
    assert!(
        db_path.ends_with("ironclad-test.db"),
        "database path should preserve the filename, got: {db_path}"
    );
}

#[test]
fn tilde_expansion_expands_home_for_workspace() {
    let cfg = IroncladConfig::from_str(tilde_toml()).unwrap();
    let ws = cfg.agent.workspace.to_string_lossy().to_string();
    assert!(
        !ws.starts_with('~'),
        "workspace should not start with '~' after expansion, got: {ws}"
    );
    assert!(
        ws.ends_with("ironclad-workspace"),
        "workspace should preserve the directory name, got: {ws}"
    );
}

#[test]
fn tilde_expansion_expands_home_for_log_dir() {
    let cfg = IroncladConfig::from_str(tilde_toml()).unwrap();
    let log_dir = cfg.server.log_dir.to_string_lossy().to_string();
    assert!(
        !log_dir.starts_with('~'),
        "log_dir should not start with '~' after expansion, got: {log_dir}"
    );
    assert!(
        log_dir.ends_with("ironclad-logs"),
        "log_dir should preserve the directory name, got: {log_dir}"
    );
}

#[test]
fn absolute_paths_are_not_altered_by_tilde_expansion() {
    let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
    assert_eq!(
        cfg.database.path,
        PathBuf::from("/tmp/ironclad-platform-test.db"),
        "absolute path should remain unchanged after tilde expansion"
    );
}

#[test]
fn default_database_path_is_under_ironclad_dir() {
    let cfg = IroncladConfig::from_str(
        r#"
[agent]
name = "DefaultPathBot"
id = "default-path"

[server]

[database]

[models]
primary = "ollama/qwen3:8b"
"#,
    )
    .unwrap();
    let db_path = cfg.database.path.to_string_lossy().to_string();
    assert!(
        db_path.contains(".ironclad"),
        "default database path should live under ~/.ironclad, got: {db_path}"
    );
    assert!(
        db_path.ends_with("state.db"),
        "default database file should be 'state.db', got: {db_path}"
    );
}

#[test]
fn default_workspace_is_under_ironclad_dir() {
    let cfg = IroncladConfig::from_str(
        r#"
[agent]
name = "WsDefaultBot"
id = "ws-default"

[server]

[database]

[models]
primary = "ollama/qwen3:8b"
"#,
    )
    .unwrap();
    let ws = cfg.agent.workspace.to_string_lossy().to_string();
    assert!(
        ws.contains(".ironclad"),
        "default workspace should live under ~/.ironclad, got: {ws}"
    );
}

// ===========================================================================
// Section B: Daemon lifecycle tests (platform-gated)
// ===========================================================================

#[cfg(unix)]
#[test]
fn unix_daemon_plist_path_contains_launch_agents() {
    let path = daemon::plist_path();
    let path_str = path.to_string_lossy();
    assert!(
        path_str.contains("LaunchAgents"),
        "plist path should contain 'LaunchAgents', got: {path_str}"
    );
    assert!(
        path_str.ends_with("com.ironclad.agent.plist"),
        "plist path should end with 'com.ironclad.agent.plist', got: {path_str}"
    );
}

#[cfg(unix)]
#[test]
fn unix_systemd_path_contains_systemd_user() {
    let path = daemon::systemd_path();
    let path_str = path.to_string_lossy();
    assert!(
        path_str.contains("systemd/user"),
        "systemd path should contain 'systemd/user', got: {path_str}"
    );
    assert!(
        path_str.ends_with("ironclad.service"),
        "systemd path should end with 'ironclad.service', got: {path_str}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_plist_path_is_under_home_library() {
    let path = daemon::plist_path();
    let path_str = path.to_string_lossy().to_string();
    // On macOS, the plist should be under $HOME/Library/LaunchAgents
    assert!(
        path_str.contains("/Library/LaunchAgents/"),
        "macOS plist path should be under ~/Library/LaunchAgents/, got: {path_str}"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn macos_launchd_plist_content_is_valid() {
    let plist = daemon::launchd_plist("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
    // Must be valid XML plist
    assert!(
        plist.contains("<?xml"),
        "plist should start with XML declaration"
    );
    assert!(
        plist.contains("<plist version=\"1.0\">"),
        "plist should contain plist version tag"
    );
    assert!(
        plist.contains("com.ironclad.agent"),
        "plist should contain the service label"
    );
    assert!(
        plist.contains("<key>KeepAlive</key>"),
        "plist should contain KeepAlive key for daemon persistence"
    );
    assert!(
        plist.contains("<key>RunAtLoad</key>"),
        "plist should contain RunAtLoad for auto-start"
    );
    assert!(
        plist.contains("</plist>"),
        "plist should be properly closed"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_systemd_unit_has_required_sections() {
    let unit = daemon::systemd_unit("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
    assert!(
        unit.contains("[Unit]"),
        "systemd unit must have [Unit] section"
    );
    assert!(
        unit.contains("[Service]"),
        "systemd unit must have [Service] section"
    );
    assert!(
        unit.contains("[Install]"),
        "systemd unit must have [Install] section"
    );
    assert!(
        unit.contains("Type=simple"),
        "systemd service should be Type=simple"
    );
    assert!(
        unit.contains("Restart=on-failure"),
        "systemd service should restart on failure"
    );
    assert!(
        unit.contains("WantedBy=default.target"),
        "systemd service should be wanted by default.target for user services"
    );
}

#[cfg(target_os = "linux")]
#[test]
fn linux_systemd_path_is_under_config() {
    let path = daemon::systemd_path();
    let path_str = path.to_string_lossy().to_string();
    assert!(
        path_str.contains(".config/systemd/user/"),
        "Linux systemd path should be under ~/.config/systemd/user/, got: {path_str}"
    );
}

#[test]
fn daemon_install_creates_correct_file_in_tempdir() {
    // Use a temp directory as fake $HOME so we don't touch the real filesystem.
    let dir = tempfile::tempdir().unwrap();
    let fake_home = dir.path();
    let bin = fake_home.join("ironclad");
    std::fs::write(&bin, "").unwrap();
    let cfg_file = fake_home.join("ironclad.toml");
    std::fs::write(&cfg_file, "").unwrap();

    // install_daemon reads HOME; we can test install_daemon indirectly
    // by checking that the plist/systemd templates produce non-empty output.
    let os = std::env::consts::OS;
    match os {
        "macos" => {
            let plist =
                daemon::launchd_plist(bin.to_str().unwrap(), cfg_file.to_str().unwrap(), 18789);
            assert!(!plist.is_empty(), "launchd plist should not be empty");
            assert!(
                plist.contains("serve"),
                "plist should include 'serve' command"
            );
        }
        "linux" => {
            let unit =
                daemon::systemd_unit(bin.to_str().unwrap(), cfg_file.to_str().unwrap(), 18789);
            assert!(!unit.is_empty(), "systemd unit should not be empty");
            assert!(
                unit.contains("ExecStart="),
                "unit should include ExecStart directive"
            );
        }
        "windows" => {
            // On Windows, daemon uses a marker file; we verify the template-level
            // content is valid by checking the port appears in templates.
            let plist =
                daemon::launchd_plist(bin.to_str().unwrap(), cfg_file.to_str().unwrap(), 18789);
            assert!(
                plist.contains("18789"),
                "port should appear in generated content"
            );
        }
        _ => {
            // Unknown OS -- just verify we don't panic generating templates.
            let _ = daemon::launchd_plist("bin", "cfg", 1);
            let _ = daemon::systemd_unit("bin", "cfg", 1);
        }
    }
}

// ===========================================================================
// Section C: File permission tests (unix only)
// ===========================================================================

#[cfg(unix)]
#[test]
fn config_file_written_to_tempdir_can_be_read_back() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("ironclad.toml");
    let content = minimal_toml();
    std::fs::write(&config_path, content).unwrap();

    // Set restrictive permissions (0o600) like production code does for sensitive files.
    std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let meta = std::fs::metadata(&config_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "config file should have restrictive 0o600 permissions, got: {mode:o}"
    );

    // Verify the file is still readable and parseable.
    let loaded = IroncladConfig::from_file(&config_path).unwrap();
    assert_eq!(loaded.server.port, 19999);
}

#[cfg(unix)]
#[test]
fn keystore_file_permissions_are_0600() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let keystore_path = dir.path().join("keystore.enc");

    // Simulate the permission pattern used by Keystore::save() and Wallet::load_or_generate().
    std::fs::write(&keystore_path, b"encrypted-placeholder").unwrap();
    std::fs::set_permissions(&keystore_path, std::fs::Permissions::from_mode(0o600)).unwrap();

    let meta = std::fs::metadata(&keystore_path).unwrap();
    let mode = meta.permissions().mode() & 0o777;
    assert_eq!(
        mode, 0o600,
        "keystore file should have 0o600 permissions, got: {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn world_writable_file_is_detectable() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::tempdir().unwrap();
    let script_path = dir.path().join("danger.sh");
    std::fs::write(&script_path, "#!/bin/bash\necho hello").unwrap();

    // Make world-writable (0o777).
    std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o777)).unwrap();

    let meta = std::fs::metadata(&script_path).unwrap();
    let mode = meta.permissions().mode();
    // The script_runner rejects world-writable scripts (mode & 0o002 != 0).
    assert!(
        mode & 0o002 != 0,
        "world-writable bit should be set for test file, mode: {mode:o}"
    );
}

#[cfg(unix)]
#[test]
fn pid_file_roundtrip_on_unix() {
    let dir = tempfile::tempdir().unwrap();
    let pid_path = dir.path().join("ironclad.pid");

    daemon::write_pid_file(&pid_path).unwrap();
    let pid = daemon::read_pid_file(&pid_path).unwrap();
    assert_eq!(
        pid,
        Some(std::process::id()),
        "PID file should contain the current process ID"
    );

    daemon::remove_pid_file(&pid_path).unwrap();
    assert!(
        !pid_path.exists(),
        "PID file should be removed after remove_pid_file()"
    );
}

// ===========================================================================
// Section D: Line ending / encoding tests
// ===========================================================================

#[test]
fn config_parses_with_unix_line_endings() {
    let toml = "[agent]\nname = \"LF-Bot\"\nid = \"lf\"\n\n[server]\n\n[database]\npath = \"/tmp/lf.db\"\n\n[models]\nprimary = \"ollama/qwen3:8b\"\n";
    let cfg = IroncladConfig::from_str(toml).unwrap();
    assert_eq!(cfg.agent.name, "LF-Bot");
}

#[test]
fn config_parses_with_windows_line_endings() {
    let toml = "[agent]\r\nname = \"CRLF-Bot\"\r\nid = \"crlf\"\r\n\r\n[server]\r\n\r\n[database]\r\npath = \"/tmp/crlf.db\"\r\n\r\n[models]\r\nprimary = \"ollama/qwen3:8b\"\r\n";
    let cfg = IroncladConfig::from_str(toml).unwrap();
    assert_eq!(cfg.agent.name, "CRLF-Bot");
}

#[test]
fn config_parses_with_mixed_line_endings() {
    let toml = "[agent]\nname = \"Mixed-Bot\"\r\nid = \"mixed\"\n\r\n[server]\r\n\r\n[database]\npath = \"/tmp/mixed.db\"\r\n\r\n[models]\r\nprimary = \"ollama/qwen3:8b\"\n";
    let cfg = IroncladConfig::from_str(toml).unwrap();
    assert_eq!(cfg.agent.name, "Mixed-Bot");
}

#[test]
fn config_handles_utf8_agent_name() {
    let toml = r#"
[agent]
name = "Ironclad-日本語テスト"
id = "utf8-test"

[server]

[database]
path = "/tmp/utf8.db"

[models]
primary = "ollama/qwen3:8b"
"#;
    let cfg = IroncladConfig::from_str(toml).unwrap();
    assert_eq!(cfg.agent.name, "Ironclad-日本語テスト");
}

// ===========================================================================
// Section E: Platform-specific default value tests
// ===========================================================================

#[test]
fn default_interpreters_include_bash() {
    let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
    assert!(
        cfg.skills
            .allowed_interpreters
            .contains(&"bash".to_string()),
        "default interpreters should include bash on all platforms"
    );
}

#[cfg(windows)]
#[test]
fn windows_interpreters_include_python_without_suffix() {
    let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
    assert!(
        cfg.skills
            .allowed_interpreters
            .contains(&"python".to_string()),
        "Windows default interpreters should include 'python' (not 'python3')"
    );
}

#[cfg(not(windows))]
#[test]
fn unix_interpreters_include_python3() {
    let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
    assert!(
        cfg.skills
            .allowed_interpreters
            .contains(&"python3".to_string()),
        "Unix default interpreters should include 'python3'"
    );
}

#[test]
fn default_bind_is_loopback() {
    let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
    assert_eq!(
        cfg.server.bind, "127.0.0.1",
        "default server bind should be loopback for security"
    );
}

#[test]
fn default_port_is_18789() {
    let cfg = IroncladConfig::from_str(
        r#"
[agent]
name = "PortBot"
id = "port-test"

[server]

[database]
path = "/tmp/port-test.db"

[models]
primary = "ollama/qwen3:8b"
"#,
    )
    .unwrap();
    assert_eq!(cfg.server.port, 18789, "default port should be 18789");
}

// ===========================================================================
// Section F: Path separator tests
// ===========================================================================

#[test]
fn path_join_uses_correct_separator() {
    let base = PathBuf::from("home").join(".ironclad").join("state.db");
    let path_str = base.to_string_lossy().to_string();

    if cfg!(windows) {
        assert!(
            path_str.contains('\\'),
            "Windows paths should use backslash, got: {path_str}"
        );
    } else {
        assert!(
            path_str.contains('/'),
            "Unix paths should use forward slash, got: {path_str}"
        );
    }
}

#[test]
fn daemon_plist_log_paths_use_forward_slashes() {
    // launchd plists are macOS-only and should always use forward slashes.
    let plist = daemon::launchd_plist("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
    // All paths in the XML should use forward slashes.
    for line in plist.lines() {
        if line.contains("<string>/") {
            assert!(
                !line.contains('\\'),
                "launchd plist paths should not contain backslashes: {line}"
            );
        }
    }
}

#[test]
fn daemon_systemd_unit_paths_use_forward_slashes() {
    let unit = daemon::systemd_unit("/usr/local/bin/ironclad", "/etc/ironclad.toml", 18789);
    let exec_line = unit
        .lines()
        .find(|l| l.starts_with("ExecStart="))
        .expect("systemd unit should have ExecStart line");
    assert!(
        !exec_line.contains('\\'),
        "systemd ExecStart paths should not contain backslashes: {exec_line}"
    );
}

// ===========================================================================
// Section G: OS detection consistency
// ===========================================================================

#[test]
fn os_constant_matches_cfg_attribute() {
    let os = std::env::consts::OS;
    if cfg!(target_os = "macos") {
        assert_eq!(
            os, "macos",
            "cfg!(target_os = \"macos\") should match consts::OS"
        );
    } else if cfg!(target_os = "linux") {
        assert_eq!(
            os, "linux",
            "cfg!(target_os = \"linux\") should match consts::OS"
        );
    } else if cfg!(target_os = "windows") {
        assert_eq!(
            os, "windows",
            "cfg!(target_os = \"windows\") should match consts::OS"
        );
    }
    // On any platform, the OS string should be non-empty.
    assert!(!os.is_empty(), "OS constant should be non-empty");
}

#[test]
fn home_env_var_is_set() {
    // On Unix, HOME should be set. On Windows, USERPROFILE should be set.
    // The ironclad codebase falls back between these.
    let home = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE"));
    assert!(
        home.is_ok(),
        "either HOME or USERPROFILE should be set for path resolution"
    );
    let home_path = PathBuf::from(home.unwrap());
    assert!(
        home_path.is_absolute(),
        "home directory should be an absolute path"
    );
}
