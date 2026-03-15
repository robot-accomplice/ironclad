    use super::*;

    #[test]
    fn merge_json_flat_replacement() {
        let mut base = json!({"a": 1, "b": 2});
        merge_json(&mut base, &json!({"b": 99}));
        assert_eq!(base["a"], 1);
        assert_eq!(base["b"], 99);
    }

    #[test]
    fn merge_json_adds_new_keys() {
        let mut base = json!({"a": 1});
        merge_json(&mut base, &json!({"b": 2, "c": 3}));
        assert_eq!(base["a"], 1);
        assert_eq!(base["b"], 2);
        assert_eq!(base["c"], 3);
    }

    #[test]
    fn merge_json_deep_nested() {
        let mut base = json!({"outer": {"inner": 1, "keep": true}});
        merge_json(&mut base, &json!({"outer": {"inner": 99, "new": "added"}}));
        assert_eq!(base["outer"]["inner"], 99);
        assert_eq!(base["outer"]["keep"], true);
        assert_eq!(base["outer"]["new"], "added");
    }

    #[test]
    fn merge_json_array_replaces() {
        let mut base = json!({"list": [1, 2, 3]});
        merge_json(&mut base, &json!({"list": [4, 5]}));
        assert_eq!(base["list"], json!([4, 5]));
    }

    #[test]
    fn merge_json_null_replacement() {
        let mut base = json!({"a": 1});
        merge_json(&mut base, &json!({"a": null}));
        assert!(base["a"].is_null());
    }

    #[test]
    fn merge_json_empty_patch_is_noop() {
        let mut base = json!({"a": 1, "b": 2});
        merge_json(&mut base, &json!({}));
        assert_eq!(base, json!({"a": 1, "b": 2}));
    }

    #[test]
    fn merge_json_scalar_to_object() {
        let mut base = json!({"a": "string"});
        merge_json(&mut base, &json!({"a": {"nested": true}}));
        assert_eq!(base["a"]["nested"], true);
    }

    #[test]
    fn merge_json_object_to_scalar() {
        let mut base = json!({"a": {"nested": true}});
        merge_json(&mut base, &json!({"a": 42}));
        assert_eq!(base["a"], 42);
    }

    #[test]
    fn merge_json_three_levels() {
        let mut base = json!({"l1": {"l2": {"l3": "old"}}});
        merge_json(
            &mut base,
            &json!({"l1": {"l2": {"l3": "new", "extra": true}}}),
        );
        assert_eq!(base["l1"]["l2"]["l3"], "new");
        assert_eq!(base["l1"]["l2"]["extra"], true);
    }

    #[test]
    fn workstation_for_tool_uses_token_match_for_rg() {
        assert_eq!(workstation_for_tool("rg"), ("files", "tool_execution"));
        assert_eq!(
            workstation_for_tool("plugin-rg-runner"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("search_files"),
            ("files", "tool_execution")
        );
        assert_eq!(
            workstation_for_tool("target-analyzer"),
            ("exec", "tool_execution")
        );
    }

    #[test]
    fn plugin_permissions_do_not_treat_urls_or_model_ids_as_filesystem() {
        let url_required = plugin_tool_required_permissions(
            "api_call",
            &json!({"endpoint": "https://example.com/v1/chat"}),
        );
        assert!(url_required.contains(&"network"));
        assert!(!url_required.contains(&"filesystem"));

        let model_required =
            plugin_tool_required_permissions("select_model", &json!({"model": "openai/gpt-4o"}));
        assert!(!model_required.contains(&"filesystem"));
        let nested_model_required = plugin_tool_required_permissions(
            "select_model",
            &json!({"model": {"id": "openai/gpt-4o"}}),
        );
        assert!(!nested_model_required.contains(&"filesystem"));
        let model_path_required =
            plugin_tool_required_permissions("select_model", &json!({"model": "/etc/passwd"}));
        assert!(model_path_required.contains(&"filesystem"));

        let generic_relative_path_required = plugin_tool_required_permissions(
            "process_input",
            &json!({"input": "secrets/config.yaml"}),
        );
        assert!(generic_relative_path_required.contains(&"filesystem"));

        let nested_path_required =
            plugin_tool_required_permissions("process_input", &json!({"path": {"name": "secret"}}));
        assert!(nested_path_required.contains(&"filesystem"));

        let file_required =
            plugin_tool_required_permissions("load_file", &json!({"path": "C:\\tmp\\input.txt"}));
        assert!(file_required.contains(&"filesystem"));

        let regex_required =
            plugin_tool_required_permissions("process_input", &json!({"pattern": "\\d+"}));
        assert!(!regex_required.contains(&"filesystem"));
    }

    #[test]
    fn plugin_permissions_match_shared_scan_for_input_matrix() {
        let cases = vec![
            json!({}),
            json!({"endpoint": "https://example.com/v1"}),
            json!({"socket": "wss://example.com/stream"}),
            json!({"model": "openai/gpt-4o"}),
            json!({"model": "/etc/passwd"}),
            json!({"path": "src/main.rs"}),
            json!({"input": "secrets/config.yaml"}),
            json!({"pattern": "\\d+\\w+\\s*"}),
            json!({"env_var": "SECRET_TOKEN"}),
        ];

        for input in cases {
            let scan = input_capability_scan::scan_input_capabilities(&input);
            let required = plugin_tool_required_permissions("neutral_tool", &input);
            assert_eq!(
                required.contains(&"filesystem"),
                scan.requires_filesystem,
                "filesystem mismatch for input: {input}"
            );
            assert_eq!(
                required.contains(&"network"),
                scan.requires_network,
                "network mismatch for input: {input}"
            );
        }
    }

    #[test]
    fn plugin_permissions_do_not_infer_network_from_tool_name_alone() {
        let required = plugin_tool_required_permissions("api_call", &json!({}));
        assert!(!required.contains(&"network"));
    }

    #[test]
    fn model_discovery_uses_ollama_tags_only_for_ollama_like_providers() {
        let (localish_ollama, url_ollama) =
            model_discovery_mode("ollama-gpu", "http://192.168.50.253:11434", true);
        assert!(localish_ollama);
        assert_eq!(url_ollama, "http://192.168.50.253:11434/api/tags");

        let (localish_proxy, url_proxy) =
            model_discovery_mode("anthropic", "http://127.0.0.1:8788/anthropic", false);
        assert!(!localish_proxy);
        assert_eq!(url_proxy, "http://127.0.0.1:8788/anthropic/v1/models");
    }

    #[test]
    fn apply_provider_auth_supports_query_key_mode() {
        let client = reqwest::Client::new();
        let req = client.get("http://example.test/v1/models");
        let built = apply_provider_auth(req, "query:key", "secret")
            .build()
            .expect("request builds");
        assert_eq!(
            built.url().as_str(),
            "http://example.test/v1/models?key=secret"
        );
    }

    #[test]
    fn parse_db_timestamp_supports_rfc3339_and_sqlite_formats() {
        let rfc = parse_db_timestamp_utc("2026-02-26T10:11:12Z").unwrap();
        assert_eq!(rfc.to_rfc3339(), "2026-02-26T10:11:12+00:00");

        let sqlite = parse_db_timestamp_utc("2026-02-26 10:11:12").unwrap();
        assert_eq!(sqlite.to_rfc3339(), "2026-02-26T10:11:12+00:00");

        assert!(parse_db_timestamp_utc("not-a-time").is_none());
    }

    #[test]
    fn is_recent_activity_respects_window() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-02-26T10:12:00+00:00")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert!(is_recent_activity("2026-02-26T10:11:10Z", now));
        assert!(!is_recent_activity("2026-02-26T10:00:00Z", now));
    }

    #[test]
    fn has_tool_token_matches_exact_split_tokens_only() {
        assert!(has_tool_token("plugin-rg-runner", "rg"));
        assert!(!has_tool_token("merge", "rg"));
        assert!(!has_tool_token("larger", "rg"));
    }

    #[test]
    fn format_balance_rounds_and_appends_symbol() {
        assert_eq!(format_balance(1.2345, "USDC"), "1.23");
        assert_eq!(format_balance(0.0, "ETH"), "0.000000");
        assert_eq!(format_balance(0.123456789, "WBTC"), "0.12345679");
        assert_eq!(format_balance(0.123456, "OTHER"), "0.1235");
    }

    #[test]
    fn workspace_files_snapshot_filters_hidden_and_sorts() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("z.txt"), "z").unwrap();
        std::fs::write(dir.path().join("a.txt"), "a").unwrap();
        std::fs::write(dir.path().join(".hidden"), "h").unwrap();
        std::fs::create_dir_all(dir.path().join("sub")).unwrap();

        let snap = workspace_files_snapshot(dir.path());
        let entries = snap["top_level_entries"].as_array().unwrap();
        let names: Vec<String> = entries
            .iter()
            .map(|e| e["name"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(names, vec!["a.txt", "sub", "z.txt"]);
        assert_eq!(snap["entry_count"].as_u64(), Some(3));
    }
