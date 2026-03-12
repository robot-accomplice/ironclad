    #[test]
    fn skills_config_defaults() {
        let cfg = SkillsConfig::default();
        assert_eq!(cfg.script_timeout_seconds, 30);
        assert_eq!(cfg.script_max_output_bytes, 1_048_576);
        assert!(cfg.sandbox_env);
        assert!(cfg.hot_reload);
        #[cfg(windows)]
        assert_eq!(
            cfg.allowed_interpreters,
            vec!["bash", "python", "python3", "node"]
        );
    #[cfg(not(windows))]
    assert_eq!(cfg.allowed_interpreters, vec!["bash", "python3", "node"]);
    }
    #[test]
    fn new_config_defaults() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.context.max_tokens, 128_000);
        assert_eq!(cfg.context.soft_trim_ratio, 0.8);
        assert_eq!(cfg.context.preserve_recent, 10);
        assert!(!cfg.approvals.enabled);
        assert!(cfg.approvals.gated_tools.is_empty());
        assert!(!cfg.browser.enabled);
        assert!(cfg.browser.headless);
        assert!(!cfg.daemon.auto_restart);
        assert_eq!(cfg.memory.hybrid_weight, 0.5);
        assert!(cfg.memory.embedding_provider.is_none());
    }

    #[test]
    fn bundled_providers_merged_on_minimal_config() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(cfg.providers.contains_key("ollama"));
        assert!(cfg.providers.contains_key("openai"));
        assert!(cfg.providers.contains_key("anthropic"));
        assert!(cfg.providers.contains_key("google"));
        assert!(cfg.providers.contains_key("openrouter"));
        assert!(cfg.providers.contains_key("moonshot"));
        assert_eq!(cfg.providers["ollama"].tier, "T1");
        assert_eq!(cfg.providers["moonshot"].tier, "T2");
        assert_eq!(
            cfg.providers["anthropic"].format.as_deref(),
            Some("anthropic")
        );
        assert_eq!(cfg.providers["ollama"].is_local, Some(true));
    }

    #[test]
    fn user_provider_overrides_bundled() {
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

[providers.ollama]
url = "http://custom-host:9999"
tier = "T2"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.providers["ollama"].url, "http://custom-host:9999");
        assert_eq!(cfg.providers["ollama"].tier, "T2");
        assert!(
            cfg.providers.contains_key("openai"),
            "bundled providers still present"
        );
    }

    #[test]
    fn tier_adapt_defaults() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!cfg.tier_adapt.t1_strip_system);
        assert!(!cfg.tier_adapt.t1_condense_turns);
        assert_eq!(
            cfg.tier_adapt.t2_default_preamble.as_deref(),
            Some("Be concise and direct. Focus on accuracy.")
        );
        assert!(cfg.tier_adapt.t3_t4_passthrough);
    }

    #[test]
    fn model_overrides_in_config() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "openai/gpt-4o"

[models.model_overrides."openai/gpt-4o"]
tier = "T4"
cost_per_input_token = 0.00005
cost_per_output_token = 0.00015
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        let ov = &cfg.models.model_overrides["openai/gpt-4o"];
        assert_eq!(ov.tier.as_deref(), Some("T4"));
        assert!((ov.cost_per_input_token.unwrap() - 0.00005).abs() < f64::EPSILON);
    }

    #[test]
    fn bundled_providers_toml_is_valid() {
        let toml_str = IroncladConfig::bundled_providers_toml();
        let parsed: BundledProviders = toml::from_str(toml_str).expect("bundled TOML must parse");
        assert!(!parsed.providers.is_empty());
    }

    #[test]
    fn context_checkpoint_config_defaults() {
        let cfg = ContextConfig::default();
        assert!(!cfg.checkpoint_enabled);
        assert_eq!(cfg.checkpoint_interval_turns, 10);
    }

    #[test]
    fn session_config_defaults() {
        let cfg = SessionConfig::default();
        assert_eq!(cfg.ttl_seconds, 86400);
        assert_eq!(cfg.scope_mode, "agent");
        assert!(cfg.reset_schedule.is_none());
    }

    #[test]
    fn digest_config_defaults() {
        let cfg = DigestConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.max_tokens, 512);
        assert_eq!(cfg.decay_half_life_days, 7);

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(full.digest.enabled);
        assert_eq!(full.digest.max_tokens, 512);
        assert_eq!(full.digest.decay_half_life_days, 7);
    }

    #[test]
    fn learning_config_defaults() {
        let cfg = LearningConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.min_tool_sequence, 3);
        assert!((cfg.min_success_ratio - 0.7).abs() < f64::EPSILON);
        assert_eq!(cfg.priority_boost_on_success, 5);
        assert_eq!(cfg.priority_decay_on_failure, 10);
        assert_eq!(cfg.max_learned_skills, 100);

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(full.learning.enabled);
        assert_eq!(full.learning.min_tool_sequence, 3);
    }

    #[test]
    fn session_config_from_toml() {
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
ttl_seconds = 3600
scope_mode = "peer"
reset_schedule = "0 0 * * *"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.session.ttl_seconds, 3600);
        assert_eq!(cfg.session.scope_mode, "peer");
        assert_eq!(cfg.session.reset_schedule.as_deref(), Some("0 0 * * *"));
    }

    #[test]
    fn session_reset_schedule_accepts_timezone_prefix() {
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
reset_schedule = "CRON_TZ=UTC+02:00 0 9 * * *"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(
            cfg.session.reset_schedule.as_deref(),
            Some("CRON_TZ=UTC+02:00 0 9 * * *")
        );
    }

    #[test]
    fn tilde_expansion_in_database_path() {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let expected = std::path::PathBuf::from(&home)
            .join(".ironclad")
            .join("state.db");
        let toml = r#"
[agent]
name = "TestBot"
id = "test"

[server]
port = 9999

[database]
path = "~/.ironclad/state.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(
            cfg.database.path, expected,
            "~/.ironclad/state.db should expand to $HOME/.ironclad/state.db"
        );
    }

    #[test]
    fn obsidian_config_defaults() {
        let cfg = ObsidianConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.vault_path.is_none());
        assert!(!cfg.auto_detect);
        assert!(cfg.auto_detect_paths.is_empty());
        assert!(cfg.index_on_start);
        assert!(!cfg.watch_for_changes);
        assert_eq!(cfg.ignored_folders, vec![".obsidian", ".trash", ".git"]);
        assert_eq!(cfg.template_folder, "templates");
        assert_eq!(cfg.default_folder, "ironclad");
        assert!(cfg.preferred_destination);
        assert!((cfg.tag_boost - 0.2).abs() < f64::EPSILON);

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!full.obsidian.enabled);
        assert!(full.obsidian.vault_path.is_none());
    }

    #[test]
    fn obsidian_config_from_toml() {
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

[obsidian]
enabled = true
vault_path = "~/Documents/MyVault"
default_folder = "agent-notes"
tag_boost = 0.3
ignored_folders = [".obsidian", ".git"]
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert!(cfg.obsidian.enabled);
        assert!(cfg.obsidian.vault_path.is_some());
        let vp = cfg.obsidian.vault_path.unwrap();
        assert!(
            !vp.to_str().unwrap().starts_with("~"),
            "tilde should be expanded"
        );
        assert!(vp.to_str().unwrap().contains("Documents/MyVault"));
        assert_eq!(cfg.obsidian.default_folder, "agent-notes");
        assert!((cfg.obsidian.tag_boost - 0.3).abs() < f64::EPSILON);
        assert_eq!(cfg.obsidian.ignored_folders.len(), 2);
    }

    #[test]
    fn multimodal_config_defaults() {
        let cfg = MultimodalConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.media_dir.is_none());
        assert_eq!(cfg.max_image_size_bytes, 10 * 1024 * 1024);
        assert!(cfg.vision_model.is_none());
        assert!(cfg.transcription_model.is_none());

        let full = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert!(!full.multimodal.enabled);
        assert!(full.multimodal.vision_model.is_none());
    }

    // ── direct default_*() function coverage ────────────────────────────
