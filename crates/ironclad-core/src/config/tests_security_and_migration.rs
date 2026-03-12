    #[test]
    fn resolve_config_path_explicit_overrides_all() {
        let p = resolve_config_path(Some("/tmp/custom.toml"));
        assert_eq!(p.unwrap(), std::path::PathBuf::from("/tmp/custom.toml"));
    }

    #[test]
    fn resolve_config_path_explicit_even_if_nonexistent() {
        // Explicit path is returned even if file doesn't exist — caller handles errors
        let p = resolve_config_path(Some("/nonexistent/path/ironclad.toml"));
        assert_eq!(
            p.unwrap(),
            std::path::PathBuf::from("/nonexistent/path/ironclad.toml")
        );
    }

    #[test]
    fn resolve_config_path_explicit_tilde_expands() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let p = resolve_config_path(Some("~/ironclad.toml")).unwrap();
        assert_eq!(p, std::path::PathBuf::from(home).join("ironclad.toml"));
    }

    #[test]
    fn tilde_expansion_for_multimodal_knowledge_and_device_paths() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let cfg = IroncladConfig::from_str(
            r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"

[multimodal]
media_dir = "~/media"

[[knowledge.sources]]
name = "local"
source_type = "filesystem"
path = "~/docs"

[devices]
identity_path = "~/.ironclad/device.json"
"#,
        )
        .unwrap();

        assert_eq!(
            cfg.multimodal.media_dir.unwrap(),
            std::path::PathBuf::from(&home).join("media")
        );
        assert_eq!(
            cfg.knowledge.sources[0].path.clone().unwrap(),
            std::path::PathBuf::from(&home).join("docs")
        );
        assert_eq!(
            cfg.devices.identity_path.unwrap(),
            std::path::PathBuf::from(&home)
                .join(".ironclad")
                .join("device.json")
        );
    }

    // ── KnowledgeConfig / WorkspaceConfig defaults ──────────────────────

    #[test]
    fn knowledge_config_default() {
        let cfg = KnowledgeConfig::default();
        assert!(cfg.sources.is_empty());
    }

    #[test]
    fn workspace_config_default() {
        let cfg = WorkspaceConfig::default();
        assert!(!cfg.soul_versioning);
        assert!(!cfg.index_on_start);
        assert!(!cfg.watch_for_changes);
    }

    #[test]
    fn voice_channel_config_default() {
        let cfg = VoiceChannelConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.stt_model.is_none());
        assert!(cfg.tts_model.is_none());
        assert!(cfg.tts_voice.is_none());
    }

    // ── Security validation ────────────────────────────────────────────

    #[test]
    fn validate_default_security_config_ok() {
        // Default SecurityConfig should pass validation.
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
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_allowlist_authority_exceeds_trusted_fails() {
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

[security]
allowlist_authority = "Creator"
trusted_authority = "Peer"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("allowlist_authority"));
    }

    #[test]
    fn validate_threat_ceiling_creator_fails() {
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

[security]
threat_caution_ceiling = "Creator"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("threat_caution_ceiling"));
    }

    #[test]
    fn validate_security_peer_ceiling_ok() {
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

[security]
threat_caution_ceiling = "Peer"
"#;
        IroncladConfig::from_str(toml).unwrap();
    }

    #[test]
    fn validate_routing_accuracy_floor_out_of_range_fails() {
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

[models.routing]
accuracy_floor = 1.5
"#;
        assert!(IroncladConfig::from_str(toml).is_err());
    }

    #[test]
    fn validate_routing_canary_fraction_requires_canary_model() {
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

[models.routing]
canary_fraction = 0.1
"#;
        assert!(IroncladConfig::from_str(toml).is_err());
    }

    #[test]
    fn validate_routing_canary_model_must_not_be_blocked() {
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

[models.routing]
canary_model = "ollama/qwen3:8b"
canary_fraction = 0.2
blocked_models = ["ollama/qwen3:8b"]
"#;
        assert!(IroncladConfig::from_str(toml).is_err());
    }

    #[test]
    fn validate_routing_mode_invalid_fails() {
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

[models.routing]
mode = "random"
"#;
        assert!(IroncladConfig::from_str(toml).is_err());
    }

    #[test]
    fn validate_routing_mode_heuristic_is_rejected() {
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

[models.routing]
mode = "heuristic"
"#;
        let err = IroncladConfig::from_str(toml).expect_err("heuristic must be rejected");
        assert!(format!("{err}").contains("models.routing.mode"));
    }

    #[test]
    fn validate_deny_on_empty_allowlist_false_is_rejected() {
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

[security]
deny_on_empty_allowlist = false
"#;
        let err =
            IroncladConfig::from_str(toml).expect_err("legacy deny_on_empty flag must be rejected");
        assert!(format!("{err}").contains("deny_on_empty_allowlist"));
    }

    #[test]
    fn migrate_removed_legacy_config_rewrites_removed_fields() {
        let raw = r#"
[server]
host = "127.0.0.1"

[models]
primary = "ollama/qwen3:8b"

[models.routing]
mode = "heuristic"

[security]
deny_on_empty_allowlist = false

[circuit_breaker]
credit_cooldown_seconds = 300
"#;
        let (rewritten, report) = migrate_removed_legacy_config(raw)
            .expect("migration helper should succeed")
            .expect("legacy config should be rewritten");
        assert!(report.renamed_server_host_to_bind);
        assert!(report.routing_mode_heuristic_rewritten);
        assert!(report.deny_on_empty_allowlist_hardened);
        assert!(report.removed_credit_cooldown_seconds);
        assert!(rewritten.contains("bind = \"127.0.0.1\""));
        assert!(rewritten.contains("mode = \"metascore\""));
        assert!(rewritten.contains("deny_on_empty_allowlist = true"));
        assert!(!rewritten.contains("credit_cooldown_seconds"));
    }
