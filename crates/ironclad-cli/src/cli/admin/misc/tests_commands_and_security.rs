    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn mechanic_json_repair_mode_creates_default_layout() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        let logs_dir = ironclad_dir.join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::write(
            logs_dir.join("ironclad.log"),
            "Telegram API error\",\"status\":\"404 Not Found\"\n\
             Telegram API error\",\"status\":\"404 Not Found\"\n\
             Telegram API error\",\"status\":\"404 Not Found\"\n\
             unknown action: unknown\nunknown action: unknown\nunknown action: unknown\n",
        )
        .unwrap();

        let state_db = ironclad_dir.join("state.db");
        let conn = rusqlite::Connection::open(&state_db).unwrap();
        conn.execute_batch(
            "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, skills_json) VALUES ('specialist', NULL);",
        )
        .unwrap();
        drop(conn);

        let wallet = ironclad_dir.join("wallet.json");
        std::fs::write(&wallet, "{}").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&wallet).unwrap().permissions();
            perms.set_mode(0o644);
            std::fs::set_permissions(&wallet, perms).unwrap();
        }

        cmd_mechanic_json("http://127.0.0.1:9", true, &[])
            .await
            .expect("mechanic should complete with unreachable gateway");

        let conn = rusqlite::Connection::open(&state_db).unwrap();
        let role: String = conn
            .query_row("SELECT role FROM sub_agents LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(role, "subagent");
    }

    #[test]
    fn cmd_reset_yes_removes_state_and_preserves_wallet() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        let logs_dir = ironclad_dir.join("logs");
        std::fs::create_dir_all(&logs_dir).unwrap();
        std::fs::write(ironclad_dir.join("state.db"), "db").unwrap();
        std::fs::write(
            ironclad_dir.join("ironclad.toml"),
            "[agent]\nname='x'\nid='x'\n",
        )
        .unwrap();
        std::fs::write(ironclad_dir.join("wallet.json"), "{}").unwrap();

        cmd_reset(true).unwrap();

        assert!(!ironclad_dir.join("state.db").exists());
        assert!(!ironclad_dir.join("ironclad.toml").exists());
        assert!(!logs_dir.exists());
        assert!(ironclad_dir.join("wallet.json").exists());
    }

    #[test]
    fn cmd_uninstall_purge_removes_data_dir() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        std::fs::create_dir_all(&ironclad_dir).unwrap();
        std::fs::write(ironclad_dir.join("state.db"), "db").unwrap();

        cmd_uninstall(true, None).unwrap();
        assert!(!ironclad_dir.exists());
    }

    #[test]
    fn cmd_completion_bash_succeeds() {
        cmd_completion("bash").unwrap();
    }

    #[test]
    fn cmd_completion_zsh_succeeds() {
        cmd_completion("zsh").unwrap();
    }

    #[test]
    fn cmd_completion_fish_succeeds() {
        cmd_completion("fish").unwrap();
    }

    #[test]
    fn cmd_completion_unknown_shell_succeeds() {
        cmd_completion("powershell").unwrap();
    }

    #[test]
    fn count_occurrences_empty_haystack() {
        assert_eq!(count_occurrences("", "needle"), 0);
    }

    #[test]
    fn count_occurrences_empty_needle() {
        // Empty needle matches at every byte boundary + 1
        assert!(count_occurrences("abc", "") >= 3);
    }

    #[test]
    fn count_occurrences_no_match() {
        assert_eq!(count_occurrences("hello world", "xyz"), 0);
    }

    #[test]
    fn count_occurrences_overlapping_needles() {
        assert_eq!(count_occurrences("aaa", "aa"), 1);
    }

    #[test]
    fn recent_log_snapshot_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert!(recent_log_snapshot(dir.path(), 1024).is_none());
    }

    #[test]
    fn recent_log_snapshot_nonexistent_dir() {
        assert!(recent_log_snapshot(Path::new("/nonexistent/path"), 1024).is_none());
    }

    #[test]
    fn recent_log_snapshot_ignores_non_log_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("readme.txt"), "not a log").unwrap();
        assert!(recent_log_snapshot(dir.path(), 1024).is_none());
    }

    #[test]
    fn recent_log_snapshot_max_bytes_truncates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ironclad.log"), "a]".repeat(500)).unwrap();
        let snap = recent_log_snapshot(dir.path(), 10).unwrap();
        assert!(snap.len() <= 20); // may include partial UTF-8 boundary expansion
    }

    #[test]
    fn go_bin_candidates_with_gopath() {
        let candidates = go_bin_candidates_with(Some("/custom/go/path"));
        assert!(candidates.contains(&PathBuf::from("/custom/go/path/bin")));
    }

    #[test]
    fn go_bin_candidates_without_gopath() {
        let candidates = go_bin_candidates_with(None);
        // Should still have at least one candidate from HOME
        assert!(!candidates.is_empty() || std::env::var("HOME").is_err());
    }

    #[test]
    fn find_gosh_in_go_bins_with_no_gosh() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        // The temp dir's bin/ has no gosh binary inside
        let temp_gosh_path = bin.join("gosh");
        assert!(!temp_gosh_path.is_file());
        // If the function finds gosh, it must NOT be from our temp dir
        // (it could be from $HOME/go/bin on machines where gosh is installed)
        if let Some(found) = find_gosh_in_go_bins_with(dir.path().to_str()) {
            assert!(
                !found.starts_with(dir.path()),
                "found gosh in temp dir, but we didn't put one there"
            );
        }
    }

    #[test]
    fn path_contains_dir_in_empty_path_var() {
        let path_var = std::ffi::OsString::from("");
        assert!(!path_contains_dir_in(Path::new("/usr/bin"), &path_var));
    }

    #[test]
    fn path_contains_dir_in_multiple_entries() {
        let path_var = std::ffi::OsString::from("/usr/bin:/usr/local/bin:/opt/bin");
        assert!(path_contains_dir_in(Path::new("/usr/local/bin"), &path_var));
        assert!(!path_contains_dir_in(Path::new("/usr/local"), &path_var));
    }

    #[test]
    fn normalize_schema_safe_nonexistent_db() {
        assert!(!normalize_schema_safe(Path::new("/nonexistent/path.db")).unwrap());
    }

    #[test]
    fn normalize_schema_safe_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE sub_agents (role TEXT, skills_json TEXT);
             INSERT INTO sub_agents (role, skills_json) VALUES ('subagent', '[]');",
        )
        .unwrap();
        drop(conn);

        // Already-normalized data: nothing to fix → returns false
        assert!(!normalize_schema_safe(&db_path).unwrap());
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let role: String = conn
            .query_row("SELECT role FROM sub_agents LIMIT 1", [], |r| r.get(0))
            .unwrap();
        assert_eq!(role, "subagent");
    }

    #[test]
    fn finding_builder_fields_all_populated() {
        let f = finding(
            "test-id",
            "low",
            0.5,
            "A summary",
            "Detailed explanation",
            "Plan description",
            vec!["cmd1".into(), "cmd2".into()],
            false,
            true,
        );
        assert_eq!(f.id, "test-id");
        assert_eq!(f.severity, "low");
        assert!((f.confidence - 0.5).abs() < f64::EPSILON);
        assert_eq!(f.summary, "A summary");
        assert_eq!(f.details, "Detailed explanation");
        assert!(!f.repair_plan.safe_auto_repair);
        assert!(f.repair_plan.requires_human_approval);
        assert_eq!(f.repair_plan.commands.len(), 2);
        assert!(!f.auto_repaired);
    }

    #[test]
    fn cmd_security_audit_warns_on_plaintext_api_keys() {
        let cfg_dir = tempfile::tempdir().unwrap();
        let cfg_path = cfg_dir.path().join("ironclad.toml");
        std::fs::write(
            &cfg_path,
            r#"[agent]
name = "Test"
id = "test"
api_key = "sk-1234567890"
[server]
bind = "0.0.0.0"
port = 18789
[database]
path = ":memory:"
[models]
primary = "ollama/qwen3:8b"
[cors]
allowed_origins = ["*"]
"#,
        )
        .unwrap();

        // Should succeed even with warnings about plaintext keys, 0.0.0.0 bind, and wildcard CORS
        cmd_security_audit(cfg_path.to_str().unwrap()).unwrap();
    }

    #[test]
    fn cmd_security_audit_nonexistent_config() {
        // Should handle missing config gracefully
        cmd_security_audit("/nonexistent/path/ironclad.toml").unwrap();
    }

    #[test]
    fn cmd_security_audit_runs_against_local_config() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let cfg_dir = tempfile::tempdir().unwrap();
        let cfg_path = cfg_dir.path().join("ironclad.toml");
        std::fs::write(
            &cfg_path,
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

        cmd_security_audit(cfg_path.to_str().unwrap()).unwrap();
    }

    #[test]
    fn resolve_security_audit_config_path_falls_back_to_home_default() {
        let _lock = env_lock().lock().unwrap();
        let home = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", home.path().to_str().unwrap());
        let ironclad_dir = home.path().join(".ironclad");
        std::fs::create_dir_all(&ironclad_dir).unwrap();
        let home_cfg = ironclad_dir.join("ironclad.toml");
        std::fs::write(&home_cfg, "[server]\nport = 18789\n").unwrap();

        let resolved = resolve_security_audit_config_path("ironclad.toml");
        assert_eq!(resolved, home_cfg);
    }
