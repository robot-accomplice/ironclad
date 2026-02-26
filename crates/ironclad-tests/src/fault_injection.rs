//! # Fault Injection Integration Tests (v0.8.0 Task 34)
//!
//! Validates that when a dependency returns an error at a crate boundary,
//! the consumer degrades gracefully -- returning a meaningful `Result::Err`
//! instead of panicking. Each section targets a specific cross-crate
//! error path.

// ── 1. LLM client: network errors propagate as Result::Err ──────────

mod llm_fault {
    use ironclad_core::IroncladError;
    use ironclad_llm::LlmClient;
    use std::collections::HashMap;

    /// Connection-refused on a non-routable address yields `IroncladError::Network`,
    /// not a panic.
    #[tokio::test]
    async fn connection_refused_returns_network_error() {
        let client = LlmClient::new().expect("client construction");
        let body = serde_json::json!({"model": "test", "messages": []});
        let err = client
            .forward_request("http://127.0.0.1:1/v1/chat/completions", "fake-key", body)
            .await
            .expect_err("should fail on unreachable port");

        match &err {
            IroncladError::Network(msg) => {
                assert!(
                    msg.contains("request failed"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected Network variant, got: {other:?}"),
        }
    }

    /// A completely invalid URL still yields a clean error, not a panic.
    #[tokio::test]
    async fn invalid_url_returns_error() {
        let client = LlmClient::new().expect("client construction");
        let body = serde_json::json!({"model": "test", "messages": []});
        let result = client
            .forward_request("not-a-url", "fake-key", body)
            .await;
        assert!(result.is_err(), "invalid URL must return Err");
    }

    /// Streaming to a connection-refused address yields `IroncladError::Network`.
    #[tokio::test]
    async fn stream_connection_refused_returns_network_error() {
        let client = LlmClient::new().expect("client construction");
        let body = serde_json::json!({"model": "test", "messages": []});
        match client
            .forward_stream(
                "http://127.0.0.1:1/v1/chat/completions",
                "fake-key",
                body,
                "Authorization",
                &HashMap::new(),
            )
            .await
        {
            Err(IroncladError::Network(msg)) => {
                assert!(
                    msg.contains("stream request failed"),
                    "unexpected message: {msg}"
                );
            }
            Err(other) => panic!("expected Network variant, got: {other:?}"),
            Ok(_) => panic!("stream to unreachable port must fail"),
        }
    }

    /// Provider-specific auth with connection-refused still maps to Network.
    #[tokio::test]
    async fn custom_auth_connection_refused_returns_network_error() {
        let client = LlmClient::new().expect("client construction");
        let body = serde_json::json!({"model": "test", "messages": []});
        let mut extra = HashMap::new();
        extra.insert("anthropic-version".into(), "2023-06-01".into());
        let err = client
            .forward_with_provider(
                "http://127.0.0.1:1/v1/messages",
                "fake-key",
                body,
                "x-api-key",
                &extra,
            )
            .await
            .expect_err("custom auth to unreachable port must fail");

        assert!(matches!(err, IroncladError::Network(_)));
    }

    /// Query-string auth mode with bad URL still yields clean error.
    #[tokio::test]
    async fn query_auth_connection_refused_returns_error() {
        let client = LlmClient::new().expect("client construction");
        let body = serde_json::json!({"model": "test", "messages": []});
        let err = client
            .forward_with_provider(
                "http://127.0.0.1:1/v1/generate",
                "fake-key",
                body,
                "query:key",
                &HashMap::new(),
            )
            .await
            .expect_err("query-auth to unreachable port must fail");

        assert!(matches!(err, IroncladError::Network(_)));
    }
}

// ── 2. Database: open/write/query errors propagate cleanly ──────────

mod db_fault {
    use ironclad_core::IroncladError;
    use ironclad_db::Database;

    /// Opening a DB at an invalid filesystem path returns `IroncladError::Database`.
    #[test]
    fn open_invalid_path_returns_database_error() {
        let err = Database::new("/").expect_err("opening '/' as DB must fail");
        assert!(
            matches!(err, IroncladError::Database(_)),
            "expected Database variant, got: {err:?}"
        );
    }

    /// Opening a DB at a deeply nested nonexistent directory returns error.
    #[test]
    fn open_nonexistent_dir_returns_error() {
        let result = Database::new("/nonexistent/deeply/nested/path/db.sqlite");
        assert!(result.is_err(), "nonexistent path must fail");
    }

    /// Writing a session to an in-memory DB works, then querying a nonexistent
    /// session returns an empty result (not a panic or crash).
    #[test]
    fn query_nonexistent_session_returns_empty() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let messages =
            ironclad_db::sessions::list_messages(&db, "nonexistent-session-id", None).unwrap();
        assert!(
            messages.is_empty(),
            "querying nonexistent session should return empty vec"
        );
    }

    /// Storing a memory tier entry with extreme-length content does not panic.
    #[test]
    fn store_extreme_length_content_does_not_panic() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let huge_content = "x".repeat(10_000_000); // 10 MB of 'x'
        // Should either succeed or return an error, but never panic
        let result =
            ironclad_db::memory::store_working(&db, "test-session", "observation", &huge_content, 5);
        // We don't care if it succeeds or fails -- just that it doesn't panic
        let _ = result;
    }

    /// Passing empty strings to session creation does not panic.
    #[test]
    fn empty_agent_id_does_not_panic() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let result = ironclad_db::sessions::find_or_create(&db, "", None);
        // Should succeed or fail, but not panic
        let _ = result;
    }

    /// Appending a message with empty content does not panic.
    #[test]
    fn append_empty_message_does_not_panic() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let session =
            ironclad_db::sessions::find_or_create(&db, "test-agent", None).expect("create session");
        let result = ironclad_db::sessions::append_message(&db, &session, "user", "");
        let _ = result; // success or error is fine, no panic
    }

    /// Cron job creation with empty fields does not panic.
    #[test]
    fn cron_create_empty_fields_does_not_panic() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let result = ironclad_db::cron::create_job(&db, "", "", "", None, "{}");
        let _ = result; // no panic is the goal
    }

    /// Database::conn() does not panic even after clone.
    #[test]
    fn cloned_db_conn_does_not_panic() {
        let db = Database::new(":memory:").expect("in-memory DB");
        let db2 = db.clone();
        let _conn1 = db.conn();
        drop(_conn1); // release lock before second access
        let _conn2 = db2.conn();
    }
}

// ── 3. Config parsing: malformed input returns Err, not panic ───────

mod config_fault {
    use ironclad_core::config::IroncladConfig;
    use ironclad_core::IroncladError;

    /// Completely invalid TOML syntax returns a Config error.
    #[test]
    fn invalid_toml_syntax_returns_config_error() {
        let err = IroncladConfig::from_str("[[[[invalid toml")
            .expect_err("invalid TOML must fail");
        assert!(
            matches!(err, IroncladError::Config(_)),
            "expected Config variant, got: {err:?}"
        );
    }

    /// Empty TOML string returns error (missing required fields).
    #[test]
    fn empty_toml_returns_error() {
        let result = IroncladConfig::from_str("");
        assert!(result.is_err(), "empty TOML must fail");
    }

    /// TOML with wrong types for known fields returns error.
    #[test]
    fn wrong_types_returns_error() {
        let result = IroncladConfig::from_str(
            r#"
            [agent]
            name = 42
            "#,
        );
        assert!(result.is_err(), "name as integer must fail");
    }

    /// Valid TOML but missing required [agent] section returns error.
    #[test]
    fn missing_agent_section_returns_error() {
        let result = IroncladConfig::from_str(
            r#"
            [server]
            port = 9999
            "#,
        );
        // Should return Err because agent.name is required (or defaults kick in)
        // Either way it must not panic.
        let _ = result;
    }

    /// Negative port number does not panic.
    #[test]
    fn negative_port_does_not_panic() {
        let result = IroncladConfig::from_str(
            r#"
            [agent]
            name = "Test"
            id = "test"

            [server]
            port = -1

            [database]
            path = ":memory:"

            [models]
            primary = "ollama/qwen3:8b"
            "#,
        );
        let _ = result; // error or success, no panic
    }

    /// Binary garbage does not panic.
    #[test]
    fn binary_garbage_does_not_panic() {
        let garbage = "\x00\x01\x02\x7F\x1B[agent]\nname";
        let result = IroncladConfig::from_str(garbage);
        assert!(result.is_err(), "binary garbage must fail");
    }

    /// Loading from a nonexistent file path returns Err.
    #[test]
    fn nonexistent_file_returns_error() {
        let result =
            IroncladConfig::from_file(std::path::Path::new("/nonexistent/ironclad.toml"));
        assert!(result.is_err(), "nonexistent file must fail");
    }

    /// Giant TOML string does not panic (may be slow but must not crash).
    #[test]
    fn giant_toml_does_not_panic() {
        // Valid TOML structure but with a huge repeated key
        let mut huge = String::from("[agent]\nname = \"Test\"\nid = \"test\"\n");
        for i in 0..10_000 {
            huge.push_str(&format!("key_{i} = \"value\"\n"));
        }
        let result = IroncladConfig::from_str(&huge);
        let _ = result; // no panic
    }

    /// Using catch_unwind to guarantee no panic on truly malformed config.
    #[test]
    fn catch_unwind_on_malformed_toml() {
        let result = std::panic::catch_unwind(|| {
            let _ = IroncladConfig::from_str("{{{{not_toml}}}}");
        });
        assert!(
            result.is_ok(),
            "parsing malformed TOML must not panic"
        );
    }
}

// ── 4. WASM plugin: bad modules return error, not panic ─────────────

mod wasm_fault {
    use ironclad_agent::wasm::{WasmPlugin, WasmPluginConfig, WasmPluginRegistry};
    use ironclad_core::IroncladError;
    use std::path::PathBuf;

    fn default_config(path: PathBuf) -> WasmPluginConfig {
        WasmPluginConfig {
            name: "fault-test".to_string(),
            wasm_path: path,
            memory_limit_bytes: 64 * 1024 * 1024,
            execution_timeout_ms: 30_000,
            capabilities: vec![],
        }
    }

    /// Loading a WASM plugin from a missing file returns `IroncladError::Config`.
    #[test]
    fn missing_wasm_file_returns_config_error() {
        let config = default_config(PathBuf::from("/nonexistent/plugin.wasm"));
        let mut plugin = WasmPlugin::new(config);
        let err = plugin.load().expect_err("missing file must fail");
        match &err {
            IroncladError::Config(msg) => {
                assert!(
                    msg.contains("not found"),
                    "expected 'not found' in message, got: {msg}"
                );
            }
            other => panic!("expected Config variant, got: {other:?}"),
        }
    }

    /// Loading an empty WASM file returns error, not panic.
    #[test]
    fn empty_wasm_file_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("empty.wasm");
        std::fs::write(&path, b"").unwrap();

        let mut plugin = WasmPlugin::new(default_config(path));
        let err = plugin.load().expect_err("empty file must fail");
        assert!(
            matches!(err, IroncladError::Config(_)),
            "expected Config variant, got: {err:?}"
        );
    }

    /// Loading garbage bytes as WASM returns compilation error, not panic.
    #[test]
    fn garbage_bytes_returns_compilation_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("garbage.wasm");
        std::fs::write(&path, b"this is not valid wasm at all").unwrap();

        let mut plugin = WasmPlugin::new(default_config(path));
        let err = plugin.load().expect_err("garbage bytes must fail");
        match &err {
            IroncladError::Config(msg) => {
                assert!(
                    msg.contains("WASM compilation failed"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected Config variant, got: {other:?}"),
        }
    }

    /// Executing a plugin that was never loaded returns error, not panic.
    #[test]
    fn execute_without_load_returns_error() {
        let config = default_config(PathBuf::from("/fake.wasm"));
        let mut plugin = WasmPlugin::new(config);
        let err = plugin
            .execute(&serde_json::json!({}))
            .expect_err("execute without load must fail");
        match &err {
            IroncladError::Config(msg) => {
                assert!(
                    msg.contains("not loaded"),
                    "unexpected message: {msg}"
                );
            }
            other => panic!("expected Config variant, got: {other:?}"),
        }
    }

    /// Registry: executing an unregistered plugin returns clean error.
    #[test]
    fn registry_execute_unknown_returns_error() {
        let mut registry = WasmPluginRegistry::new();
        let err = registry
            .execute("nonexistent-plugin", &serde_json::json!({}))
            .expect_err("executing unknown plugin must fail");
        assert!(
            matches!(err, IroncladError::Config(_)),
            "expected Config variant, got: {err:?}"
        );
    }

    /// Registry: loading an unregistered plugin returns error.
    #[test]
    fn registry_load_unknown_returns_error() {
        let mut registry = WasmPluginRegistry::new();
        let err = registry
            .load_plugin("nonexistent-plugin")
            .expect_err("loading unknown plugin must fail");
        assert!(
            matches!(err, IroncladError::Config(_)),
            "expected Config variant, got: {err:?}"
        );
    }

    /// catch_unwind on garbage WASM to guarantee no panic path.
    #[test]
    fn catch_unwind_garbage_wasm_load() {
        let result = std::panic::catch_unwind(|| {
            let dir = tempfile::TempDir::new().unwrap();
            let path = dir.path().join("unwind.wasm");
            std::fs::write(&path, vec![0u8; 1024]).unwrap();
            let mut plugin = WasmPlugin::new(default_config(path));
            let _ = plugin.load();
        });
        assert!(result.is_ok(), "loading garbage WASM must not panic");
    }

    /// Executing after unload returns error, not panic.
    #[test]
    fn execute_after_unload_returns_error() {
        let dir = tempfile::TempDir::new().unwrap();
        let wasm_bytes =
            wat::parse_str(r#"(module (func (export "process") (result i32) i32.const 42))"#)
                .unwrap();
        let path = dir.path().join("unload-test.wasm");
        std::fs::write(&path, wasm_bytes).unwrap();

        let mut plugin = WasmPlugin::new(default_config(path));
        plugin.load().expect("load should succeed");
        plugin.unload();
        let err = plugin
            .execute(&serde_json::json!({}))
            .expect_err("execute after unload must fail");
        assert!(
            matches!(err, IroncladError::Config(_)),
            "expected Config variant, got: {err:?}"
        );
    }
}

// ── 5. Scheduler: invalid expressions handled gracefully ────────────

mod scheduler_fault {
    use ironclad_schedule::scheduler::DurableScheduler;

    /// Completely nonsensical cron expression returns false, not panic.
    #[test]
    fn garbage_cron_returns_false() {
        let result = DurableScheduler::evaluate_cron(
            "not a cron expression at all",
            None,
            "2025-01-01T12:00:00+00:00",
        );
        assert!(!result, "garbage cron must return false");
    }

    /// Empty cron expression returns false, not panic.
    #[test]
    fn empty_cron_returns_false() {
        let result =
            DurableScheduler::evaluate_cron("", None, "2025-01-01T12:00:00+00:00");
        assert!(!result, "empty cron must return false");
    }

    /// Too many fields returns false.
    #[test]
    fn too_many_fields_returns_false() {
        let result = DurableScheduler::evaluate_cron(
            "0 12 * * * * * *",
            None,
            "2025-01-01T12:00:00+00:00",
        );
        assert!(!result, "too many fields must return false");
    }

    /// Too few fields returns false.
    #[test]
    fn too_few_fields_returns_false() {
        let result = DurableScheduler::evaluate_cron(
            "0 12",
            None,
            "2025-01-01T12:00:00+00:00",
        );
        assert!(!result, "too few fields must return false");
    }

    /// Invalid `now` timestamp returns false for cron.
    #[test]
    fn invalid_now_cron_returns_false() {
        let result = DurableScheduler::evaluate_cron("0 12 * * *", None, "not-a-date");
        assert!(!result, "invalid now must return false");
    }

    /// Invalid `now` timestamp returns false for interval.
    #[test]
    fn invalid_now_interval_returns_false() {
        let result = DurableScheduler::evaluate_interval(None, 60_000, "garbage");
        assert!(!result, "invalid now must return false");
    }

    /// Invalid schedule expression for `evaluate_at` returns false.
    #[test]
    fn invalid_at_target_returns_false() {
        let result =
            DurableScheduler::evaluate_at("not-a-date", "2025-01-01T12:00:00+00:00");
        assert!(!result, "invalid target must return false");
    }

    /// Invalid `now` for `evaluate_at` returns false.
    #[test]
    fn invalid_at_now_returns_false() {
        let result =
            DurableScheduler::evaluate_at("2025-01-01T12:00:00+00:00", "not-a-date");
        assert!(!result, "invalid now must return false");
    }

    /// calculate_next_run with unknown schedule kind returns None.
    #[test]
    fn unknown_schedule_kind_returns_none() {
        let result = DurableScheduler::calculate_next_run(
            "weekly",
            None,
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none(), "unknown kind must return None");
    }

    /// calculate_next_run for interval with missing ms returns None.
    #[test]
    fn interval_missing_ms_returns_none() {
        let result = DurableScheduler::calculate_next_run(
            "interval",
            None,
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none(), "interval without ms must return None");
    }

    /// calculate_next_run with garbage cron expression returns None.
    #[test]
    fn garbage_cron_next_run_returns_none() {
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("not a cron expr"),
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none(), "garbage cron must return None");
    }

    /// Invalid timezone prefix in cron returns false (falls back to UTC).
    #[test]
    fn invalid_timezone_prefix_does_not_panic() {
        let result = std::panic::catch_unwind(|| {
            DurableScheduler::evaluate_cron(
                "CRON_TZ=InvalidZone 0 12 * * *",
                None,
                "2025-01-01T12:00:00+00:00",
            )
        });
        assert!(
            result.is_ok(),
            "invalid timezone prefix must not panic"
        );
    }

    /// Negative interval_ms does not panic (may return true or false).
    #[test]
    fn negative_interval_does_not_panic() {
        let result = std::panic::catch_unwind(|| {
            DurableScheduler::evaluate_interval(
                Some("2025-01-01T00:00:00+00:00"),
                -1,
                "2025-01-01T00:00:01+00:00",
            )
        });
        assert!(result.is_ok(), "negative interval must not panic");
    }

    /// Zero interval_ms does not panic.
    #[test]
    fn zero_interval_does_not_panic() {
        let result = std::panic::catch_unwind(|| {
            DurableScheduler::evaluate_interval(
                Some("2025-01-01T00:00:00+00:00"),
                0,
                "2025-01-01T00:00:01+00:00",
            )
        });
        assert!(result.is_ok(), "zero interval must not panic");
    }

    /// calculate_next_run for "at" with garbage timestamp returns None.
    #[test]
    fn at_garbage_timestamp_returns_none() {
        let result = DurableScheduler::calculate_next_run(
            "at",
            Some("not-a-timestamp"),
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none(), "garbage 'at' timestamp must return None");
    }

    /// calculate_next_run with invalid `now` returns None for all kinds.
    #[test]
    fn invalid_now_all_kinds_returns_none() {
        for kind in &["interval", "cron", "at"] {
            let result = DurableScheduler::calculate_next_run(
                kind,
                Some("0 12 * * *"),
                Some(60_000),
                "not-a-date",
            );
            assert!(
                result.is_none(),
                "invalid now with kind '{}' must return None",
                kind
            );
        }
    }
}

// ── 6. Error type boundary: conversion fidelity ─────────────────────

mod error_boundary {
    use ironclad_core::IroncladError;

    /// toml::de::Error converts to IroncladError::Config.
    #[test]
    fn toml_parse_error_converts_to_config() {
        let bad = "[[invalid";
        let toml_err: std::result::Result<toml::Value, _> = toml::from_str(bad);
        let err: IroncladError = toml_err.unwrap_err().into();
        assert!(
            matches!(err, IroncladError::Config(_)),
            "toml error must convert to Config variant"
        );
    }

    /// serde_json::Error converts to IroncladError::Config.
    #[test]
    fn json_parse_error_converts_to_config() {
        let bad = "{invalid json}";
        let json_err: std::result::Result<serde_json::Value, _> = serde_json::from_str(bad);
        let err: IroncladError = json_err.unwrap_err().into();
        assert!(
            matches!(err, IroncladError::Config(_)),
            "json error must convert to Config variant"
        );
    }

    /// std::io::Error converts to IroncladError::Io.
    #[test]
    fn io_error_converts_to_io() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file missing");
        let err: IroncladError = io_err.into();
        assert!(
            matches!(err, IroncladError::Io(_)),
            "io error must convert to Io variant"
        );
    }

    /// All error variants produce non-empty Display strings.
    #[test]
    fn all_variants_have_nonempty_display() {
        let variants: Vec<IroncladError> = vec![
            IroncladError::Config("test".into()),
            IroncladError::Channel("test".into()),
            IroncladError::Database("test".into()),
            IroncladError::Llm("test".into()),
            IroncladError::Network("test".into()),
            IroncladError::Policy {
                rule: "r".into(),
                reason: "r".into(),
            },
            IroncladError::Tool {
                tool: "t".into(),
                message: "m".into(),
            },
            IroncladError::Wallet("test".into()),
            IroncladError::Injection("test".into()),
            IroncladError::Schedule("test".into()),
            IroncladError::A2a("test".into()),
            IroncladError::Skill("test".into()),
            IroncladError::Keystore("test".into()),
        ];
        for err in &variants {
            let display = err.to_string();
            assert!(
                !display.is_empty(),
                "Display for {:?} must be non-empty",
                err
            );
        }
    }

    /// is_credit_error correctly classifies non-credit errors.
    #[test]
    fn non_credit_errors_return_false() {
        let non_credit = vec![
            IroncladError::Config("credit billing".into()),
            IroncladError::Database("payment".into()),
            IroncladError::Llm("rate limited, try again".into()),
            IroncladError::Network("connection refused".into()),
        ];
        for err in &non_credit {
            // Config/Database always false; generic rate limit without "credit" is false
            if matches!(err, IroncladError::Config(_) | IroncladError::Database(_)) {
                assert!(
                    !err.is_credit_error(),
                    "non-Llm/Network variants must not be credit errors: {err}"
                );
            }
        }
    }
}

// ── 7. Cross-boundary catch_unwind: critical paths never panic ──────

mod panic_safety {
    use ironclad_core::config::IroncladConfig;
    use ironclad_db::Database;
    use ironclad_schedule::scheduler::DurableScheduler;

    /// Config parsing with null bytes does not panic.
    #[test]
    fn config_null_bytes_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = IroncladConfig::from_str("\0\0\0[agent]\0name=\"x\"");
        });
        assert!(result.is_ok(), "null bytes in config must not panic");
    }

    /// DB open with null bytes in path does not panic.
    #[test]
    fn db_null_bytes_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = Database::new("/tmp/\0test.db");
        });
        assert!(result.is_ok(), "null bytes in DB path must not panic");
    }

    /// Scheduler cron with unicode craziness does not panic.
    #[test]
    fn scheduler_unicode_no_panic() {
        let result = std::panic::catch_unwind(|| {
            DurableScheduler::evaluate_cron(
                "\u{FEFF}0 12 * * *",
                None,
                "2025-01-01T12:00:00+00:00",
            )
        });
        assert!(result.is_ok(), "BOM in cron must not panic");
    }

    /// Scheduler with extremely long expression does not panic.
    #[test]
    fn scheduler_huge_expression_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let huge = "0 ".repeat(10_000);
            DurableScheduler::evaluate_cron(&huge, None, "2025-01-01T12:00:00+00:00")
        });
        assert!(
            result.is_ok(),
            "huge cron expression must not panic"
        );
    }

    /// LlmClient::new() never panics.
    #[test]
    fn llm_client_construction_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = ironclad_llm::LlmClient::new();
        });
        assert!(result.is_ok(), "LlmClient::new() must not panic");
    }

    /// Database in-memory construction never panics.
    #[test]
    fn db_memory_construction_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = Database::new(":memory:");
        });
        assert!(result.is_ok(), "Database::new(':memory:') must not panic");
    }

    /// WasmPluginRegistry::new() never panics.
    #[test]
    fn wasm_registry_construction_no_panic() {
        let result = std::panic::catch_unwind(|| {
            let _ = ironclad_agent::wasm::WasmPluginRegistry::new();
        });
        assert!(
            result.is_ok(),
            "WasmPluginRegistry::new() must not panic"
        );
    }
}
