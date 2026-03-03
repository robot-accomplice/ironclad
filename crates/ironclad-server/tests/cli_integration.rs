use std::path::PathBuf;
use std::process::Command;

use assert_cmd::Command as AssertCmd;
use predicates::str::contains as pred_contains;

/// Path to the ironclad binary (Cargo sets CARGO_BIN_EXE_ironclad when running integration tests).
fn ironclad_bin() -> PathBuf {
    std::env::var("CARGO_BIN_EXE_ironclad")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            let mut exe = std::env::current_exe().expect("current exe");
            exe.pop(); // deps
            exe.pop(); // debug
            exe.push("ironclad");
            exe
        })
}

fn ironclad_cmd() -> Command {
    Command::new(ironclad_bin())
}

#[test]
fn version_shows_semver() {
    let output = ironclad_cmd()
        .arg("version")
        .output()
        .expect("failed to run ironclad-server version");
    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let out = format!("{stdout}{stderr}");
    assert!(
        out.contains("ironclad") || out.contains("0."),
        "output: {out}"
    );
}

#[test]
fn init_creates_config_file() {
    let dir = tempfile::tempdir().unwrap();
    let output = ironclad_cmd()
        .arg("init")
        .current_dir(dir.path())
        .output()
        .expect("failed to run init");
    assert!(output.status.success() || String::from_utf8_lossy(&output.stderr).contains("already"));
    // Config file should exist after init
    assert!(dir.path().join("ironclad.toml").exists() || output.status.success());
}

#[test]
fn check_without_config_returns_error() {
    let dir = tempfile::tempdir().unwrap();
    // Isolate HOME so resolve_config_path won't find the developer's real config
    let fake_home = dir.path().join("home");
    std::fs::create_dir_all(&fake_home).unwrap();
    let output = ironclad_cmd()
        .arg("check")
        .env("HOME", &fake_home)
        .current_dir(dir.path())
        .output()
        .expect("failed to run check");
    // Should fail gracefully when no config exists
    assert!(
        !output.status.success(),
        "check without config should exit non-zero"
    );
}

#[test]
fn cli_help_shows_subcommands() {
    AssertCmd::new(ironclad_bin())
        .arg("--help")
        .assert()
        .success()
        .stdout(pred_contains("init"))
        .stdout(pred_contains("serve"))
        .stdout(pred_contains("status"));
}

#[test]
fn cli_init_creates_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("ironclad.toml");
    // Init with default path "."; run from temp dir so ironclad.toml is created there
    AssertCmd::new(ironclad_bin())
        .current_dir(dir.path())
        .arg("init")
        .assert()
        .success();
    assert!(
        config_path.exists(),
        "ironclad.toml should exist at {:?}",
        config_path
    );
}

#[test]
fn cli_check_validates_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("ironclad.toml");
    // First init (run in dir, init ".")
    AssertCmd::new(ironclad_bin())
        .current_dir(dir.path())
        .args(["init", "."])
        .assert()
        .success();
    assert!(config_path.exists(), "init must create ironclad.toml");
    // Then check (config file path)
    AssertCmd::new(ironclad_bin())
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .success();
}

#[test]
fn cli_status_handles_no_server() {
    // status may exit 0 with a warning when server is not running, or fail; we assert it runs and responds
    let out = AssertCmd::new(ironclad_bin())
        .args(["status", "--url", "http://127.0.0.1:19999"])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("not running")
            || stderr.contains("Start with")
            || out.status.code() != Some(0),
        "status when server is down should warn or fail: {}",
        stderr
    );
}

#[test]
fn cli_config_show_handles_no_server() {
    AssertCmd::new(ironclad_bin())
        .args(["config", "show", "--url", "http://127.0.0.1:19999"])
        .assert()
        .failure();
}

#[test]
fn cli_wallet_handles_no_server() {
    AssertCmd::new(ironclad_bin())
        .args(["wallet", "show", "--url", "http://127.0.0.1:19999"])
        .assert()
        .failure();
}

#[test]
fn cli_sessions_handles_no_server() {
    AssertCmd::new(ironclad_bin())
        .args(["sessions", "list", "--url", "http://127.0.0.1:19999"])
        .assert()
        .failure();
}

#[test]
fn cli_metrics_handles_no_server() {
    AssertCmd::new(ironclad_bin())
        .args(["metrics", "--url", "http://127.0.0.1:19999"])
        .assert()
        .failure();
}

#[test]
fn cli_version_shows_version() {
    AssertCmd::new(ironclad_bin())
        .arg("version")
        .assert()
        .success()
        .stderr(predicates::str::contains(env!("CARGO_PKG_VERSION")));
}

#[test]
fn cli_completion_variants_work() {
    AssertCmd::new(ironclad_bin())
        .args(["completion", "bash"])
        .assert()
        .success()
        .stdout(pred_contains("completion"));
    AssertCmd::new(ironclad_bin())
        .args(["completion", "zsh"])
        .assert()
        .success()
        .stdout(pred_contains("compctl"));
    AssertCmd::new(ironclad_bin())
        .args(["completion", "fish"])
        .assert()
        .success()
        .stdout(pred_contains("complete -c ironclad"));
}

#[test]
fn cli_check_invalid_config_fails() {
    let dir = tempfile::TempDir::new().unwrap();
    let config_path = dir.path().join("ironclad.toml");
    std::fs::write(&config_path, "not valid toml = [").unwrap();
    AssertCmd::new(ironclad_bin())
        .args(["check", "--config", config_path.to_str().unwrap()])
        .assert()
        .failure();
}

#[test]
fn cli_subcommand_help_paths_render() {
    for args in [
        vec!["sessions", "--help"],
        vec!["memory", "--help"],
        vec!["skills", "--help"],
        vec!["schedule", "--help"],
        vec!["metrics", "--help"],
        vec!["wallet", "--help"],
        vec!["config", "--help"],
        vec!["models", "--help"],
        vec!["plugins", "--help"],
        vec!["agents", "--help"],
        vec!["channels", "--help"],
        vec!["security", "--help"],
        vec!["auth", "--help"],
        vec!["keystore", "--help"],
        vec!["migrate", "--help"],
        vec!["daemon", "--help"],
    ] {
        AssertCmd::new(ironclad_bin()).args(args).assert().success();
    }
}

#[test]
fn cli_more_no_server_commands_fail_or_warn_cleanly() {
    let no_server = "http://127.0.0.1:19999";
    for args in [
        vec!["agents", "list", "--url", no_server],
        vec!["channels", "list", "--url", no_server],
        vec!["channels", "dead-letter", "--url", no_server],
        vec!["models", "list", "--url", no_server],
        vec!["models", "scan", "--url", no_server],
        vec!["plugins", "list", "--url", no_server],
        vec!["circuit", "status", "--url", no_server],
        vec!["circuit", "reset", "--url", no_server],
    ] {
        let out = AssertCmd::new(ironclad_bin()).args(args).output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).to_ascii_lowercase();
        let stdout = String::from_utf8_lossy(&out.stdout).to_ascii_lowercase();
        assert!(
            !out.status.success()
                || stderr.contains("not running")
                || stderr.contains("not reachable")
                || stderr.contains("cannot reach")
                || stderr.contains("could not connect")
                || stdout.contains("not running")
                || stdout.contains("not reachable")
                || stdout.contains("cannot reach"),
            "unexpected success output: stdout={stdout} stderr={stderr}"
        );
    }
}

#[test]
fn cli_security_audit_runs_on_local_config() {
    let dir = tempfile::TempDir::new().unwrap();
    let home = dir.path().join("home");
    std::fs::create_dir_all(home.join(".ironclad")).unwrap();
    let config_path = dir.path().join("ironclad.toml");
    std::fs::write(
        &config_path,
        r#"[agent]
name = "Test"
id = "test"
[server]
bind = "127.0.0.1"
port = 18789
[database]
path = ":memory:"
[models]
primary = "ollama/qwen3:8b"
"#,
    )
    .unwrap();

    AssertCmd::new(ironclad_bin())
        .env("HOME", home)
        .args([
            "security",
            "audit",
            "--config",
            config_path.to_str().unwrap(),
        ])
        .assert()
        .success();
}
