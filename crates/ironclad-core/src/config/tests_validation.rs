    #[test]
    fn validate_empty_agent_name_fails() {
        let toml = r#"
[agent]
name = ""
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("agent.name"));
    }
    #[test]
    fn validate_empty_agent_id_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = ""

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("agent.id"));
    }
    #[test]
    fn validate_empty_model_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = ""
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("models.primary"));
    }

    #[test]
    fn validate_invalid_bind_address_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999
bind = "not-an-ip"

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("not a valid IP"));
    }

    #[test]
    fn validate_localhost_bind_ok() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999
bind = "localhost"

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_invalid_session_scope_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "invalid"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("scope_mode"));
    }

    #[test]
    fn validate_group_scope_ok() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[session]
scope_mode = "group"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_negative_minimum_reserve_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[treasury]
minimum_reserve = -1.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("minimum_reserve"));
    }

    #[test]
    fn validate_zero_payment_cap_fails() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[treasury]
per_payment_cap = 0.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("per_payment_cap"));
    }

    // ── startup_announcement_channels coverage ──────────────────────────

    #[test]
    fn startup_announcements_none_returns_empty() {
        let cfg = ChannelsConfig::default();
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_flag_returns_empty() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Flag(true)),
            ..ChannelsConfig::default()
        };
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_text_returns_normalized() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Text("Telegram".into())),
            ..ChannelsConfig::default()
        };
        assert_eq!(cfg.startup_announcement_channels(), vec!["telegram"]);
    }

    #[test]
    fn startup_announcements_text_none_variant() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Text("none".into())),
            ..ChannelsConfig::default()
        };
        assert!(cfg.startup_announcement_channels().is_empty());
    }

    #[test]
    fn startup_announcements_channels_dedup_and_sort() {
        let cfg = ChannelsConfig {
            startup_announcements: Some(StartupAnnouncementsConfig::Channels(vec![
                "whatsapp".into(),
                "telegram".into(),
                "TELEGRAM".into(),
                "none".into(),
            ])),
            ..ChannelsConfig::default()
        };
        let ch = cfg.startup_announcement_channels();
        assert_eq!(ch, vec!["telegram", "whatsapp"]);
    }

    // ── expand_tilde coverage ───────────────────────────────────────────

    #[test]
    fn expand_tilde_no_tilde() {
        let p = PathBuf::from("/absolute/path");
        assert_eq!(expand_tilde(&p), p);
    }

    #[test]
    fn expand_tilde_with_tilde() {
        let p = PathBuf::from("~/Documents/vault");
        let expanded = expand_tilde(&p);
        assert!(!expanded.to_str().unwrap().starts_with("~"));
        assert!(expanded.to_str().unwrap().contains("Documents/vault"));
    }

    // ── ProviderConfig::new ─────────────────────────────────────────────

    #[test]
    fn provider_config_new() {
        let pc = ProviderConfig::new("http://localhost:11434", "T1");
        assert_eq!(pc.url, "http://localhost:11434");
        assert_eq!(pc.tier, "T1");
        assert!(pc.format.is_none());
        assert!(pc.api_key_env.is_none());
        assert!(pc.is_local.is_none());
        assert!(pc.tpm_limit.is_none());
        assert!(pc.rpm_limit.is_none());
    }

    // ── MCP transport default ───────────────────────────────────────────

    #[test]
    fn mcp_transport_default_is_sse() {
        let t = McpTransport::default();
        assert!(matches!(t, McpTransport::Sse));
    }

    // ── home_dir and dirs_next helpers ───────────────────────────────────

    #[test]
    fn home_dir_returns_valid_path() {
        let h = home_dir();
        assert!(h.is_absolute() || h == std::path::Path::new("/tmp"));
    }

    #[test]
    fn dirs_next_appends_ironclad() {
        let d = dirs_next();
        assert!(d.to_str().unwrap().contains(".ironclad"));
    }
