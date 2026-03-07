    #[test]
    fn derive_workspace_activity_prefers_recent_tool_call_then_turn_then_idle() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let conn = db.conn();
        conn.execute(
            "INSERT INTO sessions (id, agent_id, scope_key, status) VALUES (?1, ?2, 'agent', 'active')",
            rusqlite::params!["s1", "agent-1"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO turns (id, session_id, created_at) VALUES (?1, ?2, ?3)",
            rusqlite::params!["t1", "s1", "2026-02-26T10:11:50Z"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO tool_calls (id, turn_id, tool_name, input, status, created_at) VALUES (?1, ?2, ?3, '{}', 'ok', ?4)",
            rusqlite::params!["tc1", "t1", "read_file", "2026-02-26T10:11:59Z"],
        )
        .unwrap();
        drop(conn);

        let now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:12:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let active = derive_workspace_activity(&db, "agent-1", true, now);
        assert_eq!(active.0, Some("files"));
        assert_eq!(active.1, "tool_execution");
        assert_eq!(active.2.as_deref(), Some("read_file"));

        let conn = db.conn();
        conn.execute("DELETE FROM tool_calls", []).unwrap();
        drop(conn);
        let turn_only = derive_workspace_activity(&db, "agent-1", true, now);
        assert_eq!(turn_only.0, Some("llm"));
        assert_eq!(turn_only.1, "inference");

        let idle_now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:30:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let idle = derive_workspace_activity(&db, "agent-1", true, idle_now);
        assert_eq!(idle.0, Some("standby"));
        assert_eq!(idle.1, "idle");
    }

    // ── sanitize_decided_by tests ────────────────────────────────

    #[test]
    fn sanitize_decided_by_accepts_normal_input() {
        let result = sanitize_decided_by("admin-user").unwrap();
        assert_eq!(result, "admin-user");
    }

    #[test]
    fn sanitize_decided_by_strips_control_characters() {
        let result = sanitize_decided_by("user\x00\x01\x02name").unwrap();
        assert_eq!(result, "username");
    }

    #[test]
    fn sanitize_decided_by_rejects_too_long_input() {
        let long_input = "a".repeat(MAX_DECIDED_BY_LEN + 1);
        let result = sanitize_decided_by(&long_input);
        assert!(result.is_err());
        let JsonError(status, msg) = result.unwrap_err();
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(msg.contains("max length"));
    }

    #[test]
    fn sanitize_decided_by_accepts_max_length() {
        let exact = "a".repeat(MAX_DECIDED_BY_LEN);
        let result = sanitize_decided_by(&exact);
        assert!(result.is_ok());
    }

    #[test]
    fn sanitize_decided_by_empty_is_ok() {
        let result = sanitize_decided_by("").unwrap();
        assert_eq!(result, "");
    }

    // ── merge_json depth limit tests ─────────────────────────────

    #[test]
    fn merge_json_depth_limit_replaces_at_max_depth() {
        // Build a deeply nested structure beyond MERGE_JSON_MAX_DEPTH
        let mut patch = json!("leaf");
        for _ in 0..12 {
            patch = json!({"nested": patch});
        }
        let mut base = json!({"nested": {"nested": {"nested": "old"}}});
        merge_json(&mut base, &patch);
        // Should not panic and should merge/replace
        assert!(base.is_object());
    }

    // ── format_balance additional tests ──────────────────────────

    #[test]
    fn format_balance_dai_two_decimals() {
        assert_eq!(format_balance(100.999, "DAI"), "101.00");
    }

    #[test]
    fn format_balance_usdt_two_decimals() {
        assert_eq!(format_balance(0.5, "USDT"), "0.50");
    }

    #[test]
    fn format_balance_matic_six_decimals() {
        assert_eq!(format_balance(1.0, "MATIC"), "1.000000");
    }

    #[test]
    fn format_balance_weth_six_decimals() {
        assert_eq!(format_balance(0.1, "WETH"), "0.100000");
    }

    #[test]
    fn format_balance_cbbtc_eight_decimals() {
        assert_eq!(format_balance(0.5, "cbBTC"), "0.50000000");
    }

    // ── model_discovery_mode additional tests ────────────────────

    #[test]
    fn model_discovery_mode_local_flag_makes_keyless() {
        let (keyless, url) = model_discovery_mode("custom-local", "http://192.168.1.5:8080", true);
        assert!(keyless);
        assert_eq!(url, "http://192.168.1.5:8080/v1/models");
    }

    #[test]
    fn model_discovery_mode_remote_not_keyless() {
        let (keyless, url) = model_discovery_mode("openai", "https://api.openai.com", false);
        assert!(!keyless);
        assert_eq!(url, "https://api.openai.com/v1/models");
    }

    #[test]
    fn model_discovery_mode_port_11434_is_ollama_like() {
        let (keyless, url) =
            model_discovery_mode("my-provider", "http://192.168.50.253:11434", false);
        assert!(keyless);
        assert_eq!(url, "http://192.168.50.253:11434/api/tags");
    }

    // ── workstation_for_tool additional categories ────────────────

    #[test]
    fn workstation_for_tool_web_tools() {
        assert_eq!(workstation_for_tool("web_fetch"), ("web", "tool_execution"));
        assert_eq!(
            workstation_for_tool("http_request"),
            ("web", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_memory() {
        assert_eq!(workstation_for_tool("memory_store"), ("memory", "working"));
    }

    #[test]
    fn workstation_for_tool_blockchain() {
        assert_eq!(
            workstation_for_tool("wallet_balance"),
            ("blockchain", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("contract_call"),
            ("blockchain", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_file_operations() {
        assert_eq!(
            workstation_for_tool("read_file"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("write_output"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("glob_search"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("edit_code"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("patch_file"),
            ("files", "tool_execution")
        );
    }

    #[test]
    fn workstation_for_tool_unknown_falls_to_exec() {
        assert_eq!(
            workstation_for_tool("completely_unknown"),
            ("exec", "tool_execution")
        );
    }

    // ── has_tool_token additional tests ───────────────────────────

    #[test]
    fn has_tool_token_matches_at_boundaries() {
        assert!(has_tool_token("rg", "rg"));
        assert!(has_tool_token("my-rg-tool", "rg"));
        assert!(has_tool_token("rg-runner", "rg"));
    }

    #[test]
    fn has_tool_token_no_partial_match() {
        assert!(!has_tool_token("debugging", "bug"));
    }

    // ── workspace_files_snapshot edge case ────────────────────────

    #[test]
    fn workspace_files_snapshot_handles_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let snap = workspace_files_snapshot(dir.path());
        let entries = snap["top_level_entries"].as_array().unwrap();
        assert!(entries.is_empty());
        assert_eq!(snap["entry_count"].as_u64(), Some(0));
    }

    #[test]
    fn workspace_files_snapshot_handles_nonexistent_directory() {
        let snap = workspace_files_snapshot(std::path::Path::new("/nonexistent/path"));
        let entries = snap["top_level_entries"].as_array().unwrap();
        assert!(entries.is_empty());
    }

    // ── derive_workspace_activity standby ────────────────────────

    #[test]
    fn derive_workspace_activity_returns_standby_when_not_running() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let now = chrono::Utc::now();
        let (workstation, phase, _tool) = derive_workspace_activity(&db, "agent-1", false, now);
        assert_eq!(workstation, Some("standby"));
        assert_eq!(phase, "idle");
    }

    // ── default_decided_by test ──────────────────────────────────

    #[test]
    fn default_decided_by_returns_api() {
        assert_eq!(default_decided_by(), "api");
    }

    // ── parse_db_timestamp_utc edge cases ────────────────────────

    #[test]
    fn parse_db_timestamp_utc_handles_offset_timestamps() {
        use chrono::Timelike;
        let dt = parse_db_timestamp_utc("2026-01-15T08:30:00+05:30").unwrap();
        assert_eq!(dt.hour(), 3); // 08:30 +05:30 = 03:00 UTC
    }

    #[test]
    fn parse_db_timestamp_utc_empty_string() {
        assert!(parse_db_timestamp_utc("").is_none());
    }

    // ── KeySource status_pair tests ──────────────────────────────

    #[test]
    fn key_source_status_pairs() {
        assert_eq!(
            KeySource::NotRequired.status_pair(),
            ("not_required", "local")
        );
        assert_eq!(KeySource::OAuth.status_pair(), ("configured", "oauth"));
        assert_eq!(
            KeySource::Keystore("test".into()).status_pair(),
            ("configured", "keystore")
        );
        assert_eq!(
            KeySource::EnvVar("TEST_KEY".into()).status_pair(),
            ("configured", "env")
        );
        assert_eq!(KeySource::Missing.status_pair(), ("missing", "none"));
    }
