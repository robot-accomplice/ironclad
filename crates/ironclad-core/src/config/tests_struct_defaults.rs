    #[test]
    fn default_functions_return_expected_values() {
        assert_eq!(default_max_image_size(), 10 * 1024 * 1024);
        assert_eq!(default_max_chunks(), 10);
        assert!(default_digest_enabled());
        assert_eq!(default_digest_max_tokens(), 512);
        assert_eq!(default_decay_half_life_days(), 7);
        assert_eq!(default_service_name(), "_ironclad._tcp");
        assert_eq!(
            default_obsidian_ignored_folders(),
            vec![".obsidian", ".trash", ".git"]
        );
        assert_eq!(default_obsidian_template_folder(), "templates");
        assert_eq!(default_obsidian_default_folder(), "ironclad");
        assert!((default_obsidian_tag_boost() - 0.2).abs() < f64::EPSILON);
        assert_eq!(default_log_level(), "info");
        assert!((default_min_decomposition_complexity() - 0.35).abs() < f64::EPSILON);
        assert!((default_min_delegation_utility_margin() - 0.15).abs() < f64::EPSILON);
        assert_eq!(default_log_max_days(), 7);
        assert_eq!(default_rate_limit_requests(), 100);
        assert_eq!(default_rate_limit_window_secs(), 60);
        assert_eq!(default_per_ip_rate_limit_requests(), 300);
        assert_eq!(default_per_actor_rate_limit_requests(), 200);
        assert_eq!(default_port(), 18789);
        assert_eq!(default_bind(), "127.0.0.1");
        assert_eq!(default_estimated_output_tokens(), 500);
        assert_eq!(default_routing_mode(), "metascore");
        assert!((default_confidence_threshold() - 0.9).abs() < f64::EPSILON);
        assert!(default_true());
        assert_eq!(default_cb_threshold(), 3);
        assert_eq!(default_cb_window(), 60);
        assert_eq!(default_cb_cooldown(), 60);
        assert_eq!(default_cb_max_cooldown(), 900);
        assert!((default_working_pct() - 30.0).abs() < f64::EPSILON);
        assert!((default_episodic_pct() - 25.0).abs() < f64::EPSILON);
        assert!((default_semantic_pct() - 20.0).abs() < f64::EPSILON);
        assert!((default_procedural_pct() - 15.0).abs() < f64::EPSILON);
        assert!((default_relationship_pct() - 10.0).abs() < f64::EPSILON);
        assert!((default_hybrid_weight() - 0.5).abs() < f64::EPSILON);
        assert!((default_compression_ratio() - 0.5).abs() < f64::EPSILON);
        assert_eq!(default_cache_ttl(), 3600);
        assert!((default_semantic_threshold() - 0.95).abs() < f64::EPSILON);
        assert_eq!(default_max_entries(), 10000);
        assert!((default_per_payment_cap() - 100.0).abs() < f64::EPSILON);
        assert!((default_hourly_limit() - 500.0).abs() < f64::EPSILON);
        assert!((default_daily_limit() - 2000.0).abs() < f64::EPSILON);
        assert!((default_min_reserve() - 5.0).abs() < f64::EPSILON);
        assert!((default_inference_budget() - 50.0).abs() < f64::EPSILON);
        assert_eq!(default_yield_protocol(), "aave");
        assert_eq!(default_yield_chain(), "base");
        assert!((default_min_deposit() - 50.0).abs() < f64::EPSILON);
        assert!((default_withdrawal_threshold() - 30.0).abs() < f64::EPSILON);
        assert!(default_yield_pool_address().starts_with("0x"));
        assert!(default_yield_usdc_address().starts_with("0x"));
        assert_eq!(default_chain_id(), 8453);
        assert_eq!(default_rpc_url(), "https://mainnet.base.org");
        assert_eq!(default_a2a_max_msg_size(), 65536);
        assert_eq!(default_a2a_rate_limit(), 10);
        assert_eq!(default_a2a_session_timeout(), 3600);
        assert_eq!(default_script_timeout(), 30);
        assert_eq!(default_script_max_output(), 1_048_576);
        assert_eq!(default_thinking_threshold(), 30);
        assert_eq!(default_signal_daemon_url(), "http://127.0.0.1:8080");
        assert_eq!(default_imap_port(), 993);
        assert_eq!(default_smtp_port(), 587);
        assert_eq!(default_poll_interval(), 30);
        assert_eq!(default_poll_timeout(), 30);
        assert_eq!(default_max_context_tokens(), 128_000);
        assert!((default_soft_trim_ratio() - 0.8).abs() < f64::EPSILON);
        assert!((default_hard_clear_ratio() - 0.95).abs() < f64::EPSILON);
        assert_eq!(default_preserve_recent(), 10);
        assert_eq!(default_checkpoint_interval(), 10);
        assert_eq!(default_approval_timeout(), 300);
        assert_eq!(default_cdp_port(), 9222);
        assert_eq!(default_update_channel(), "stable");
        assert!(default_update_registry_url().starts_with("https://"));
        assert_eq!(default_os_file(), "OS.toml");
        assert_eq!(default_firmware_file(), "FIRMWARE.toml");
        assert_eq!(default_session_ttl(), 86400);
        assert_eq!(default_session_scope_mode(), "agent");
        assert_eq!(default_mcp_port(), 3001);
        assert!((default_confidence_floor() - 0.6).abs() < f64::EPSILON);
        assert_eq!(default_escalation_latency_ms(), 3000);
        assert_eq!(
            default_t2_preamble(),
            Some("Be concise and direct. Focus on accuracy.".into())
        );
    }
    #[test]
    fn default_path_functions_return_valid_paths() {
        let ws = default_workspace();
        assert!(ws.to_str().unwrap().contains("workspace"));
        let db = default_db_path();
        assert!(db.to_str().unwrap().contains("state.db"));
        let log = default_log_dir();
        assert!(log.to_str().unwrap().contains("logs"));
        let wallet = default_wallet_path();
        assert!(wallet.to_str().unwrap().contains("wallet.json"));
        let skills = default_skills_dir();
        assert!(skills.to_str().unwrap().contains("skills"));
        let plugins = default_plugins_dir();
        assert!(plugins.to_str().unwrap().contains("plugins"));
        let browser = default_browser_profile_dir();
        assert!(browser.to_str().unwrap().contains("browser-profiles"));
        let pid = default_pid_file();
        assert!(pid.to_str().unwrap().contains("ironclad.pid"));
    }

    #[test]
    fn default_interpreters_contains_bash() {
        let interp = default_interpreters();
        assert!(interp.contains(&"bash".to_string()));
    }

    // ── Default impl coverage for struct types ──────────────────────────

    #[test]
    fn server_config_default() {
        let cfg = ServerConfig::default();
        assert_eq!(cfg.port, 18789);
        assert_eq!(cfg.bind, "127.0.0.1");
        assert!(cfg.api_key.is_none());
        assert_eq!(cfg.log_max_days, 7);
        assert_eq!(cfg.rate_limit_requests, 100);
        assert_eq!(cfg.rate_limit_window_secs, 60);
        assert_eq!(cfg.per_ip_rate_limit_requests, 300);
        assert_eq!(cfg.per_actor_rate_limit_requests, 200);
        assert!(cfg.trusted_proxy_cidrs.is_empty());
    }

    #[test]
    fn database_config_default() {
        let cfg = DatabaseConfig::default();
        assert!(cfg.path.to_str().unwrap().contains("state.db"));
    }

    #[test]
    fn routing_config_default() {
        let cfg = RoutingConfig::default();
        assert_eq!(cfg.mode, "metascore");
        assert!((cfg.confidence_threshold - 0.9).abs() < f64::EPSILON);
        assert!(cfg.local_first);
        assert!(!cfg.cost_aware);
        assert_eq!(cfg.estimated_output_tokens, 500);
    }

    #[test]
    fn tiered_inference_config_default() {
        let cfg = TieredInferenceConfig::default();
        assert!(!cfg.enabled);
        assert!((cfg.confidence_floor - 0.6).abs() < f64::EPSILON);
        assert_eq!(cfg.escalation_latency_budget_ms, 3000);
    }

    #[test]
    fn circuit_breaker_config_default() {
        let cfg = CircuitBreakerConfig::default();
        assert_eq!(cfg.threshold, 3);
        assert_eq!(cfg.window_seconds, 60);
        assert_eq!(cfg.cooldown_seconds, 60);
        assert_eq!(cfg.max_cooldown_seconds, 900);
    }

    #[test]
    fn memory_config_default() {
        let cfg = MemoryConfig::default();
        assert!((cfg.working_budget_pct - 30.0).abs() < f64::EPSILON);
        assert!((cfg.hybrid_weight - 0.5).abs() < f64::EPSILON);
        assert!(!cfg.ann_index);
    }

    #[test]
    fn cache_config_default() {
        let cfg = CacheConfig::default();
        assert!(cfg.enabled);
        assert_eq!(cfg.exact_match_ttl_seconds, 3600);
        assert!((cfg.compression_target_ratio - 0.5).abs() < f64::EPSILON);
        assert!(!cfg.prompt_compression);
    }

    #[test]
    fn treasury_config_default() {
        let cfg = TreasuryConfig::default();
        assert!((cfg.per_payment_cap - 100.0).abs() < f64::EPSILON);
        assert!((cfg.hourly_transfer_limit - 500.0).abs() < f64::EPSILON);
        assert!((cfg.daily_transfer_limit - 2000.0).abs() < f64::EPSILON);
        assert!((cfg.minimum_reserve - 5.0).abs() < f64::EPSILON);
        assert!((cfg.daily_inference_budget - 50.0).abs() < f64::EPSILON);
    }

    #[test]
    fn yield_config_default() {
        let cfg = YieldConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.protocol, "aave");
        assert_eq!(cfg.chain, "base");
        assert!(cfg.chain_rpc_url.is_none());
        assert!(cfg.atoken_address.is_none());
    }

    #[test]
    fn wallet_config_default() {
        let cfg = WalletConfig::default();
        assert_eq!(cfg.chain_id, 8453);
        assert_eq!(cfg.rpc_url, "https://mainnet.base.org");
    }

    #[test]
    fn a2a_config_default() {
        let cfg = A2aConfig::default();
        assert!(cfg.enabled);
        assert!(cfg.require_on_chain_identity);
    }

    #[test]
    fn channels_config_default() {
        let cfg = ChannelsConfig::default();
        assert!(cfg.telegram.is_none());
        assert!(cfg.whatsapp.is_none());
        assert!(cfg.discord.is_none());
        assert!(cfg.signal.is_none());
        assert!(cfg.trusted_sender_ids.is_empty());
        assert_eq!(cfg.thinking_threshold_seconds, 30);
        assert!(cfg.startup_announcements.is_none());
    }

    #[test]
    fn email_config_default() {
        let cfg = EmailConfig::default();
        assert!(!cfg.enabled);
        assert_eq!(cfg.imap_port, 993);
        assert_eq!(cfg.smtp_port, 587);
        assert_eq!(cfg.poll_interval_seconds, 30);
        assert!(cfg.oauth2_token_env.is_empty());
        assert!(!cfg.use_oauth2);
        assert!(cfg.imap_idle_enabled);
    }

    #[test]
    fn approvals_config_default() {
        let cfg = ApprovalsConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.gated_tools.is_empty());
        assert!(cfg.blocked_tools.is_empty());
        assert_eq!(cfg.timeout_seconds, 300);
    }

    #[test]
    fn plugins_config_default() {
        let cfg = PluginsConfig::default();
        assert!(cfg.allow.is_empty());
        assert!(cfg.deny.is_empty());
    }

    #[test]
    fn browser_config_default() {
        let cfg = BrowserConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.executable_path.is_none());
        assert!(cfg.headless);
        assert_eq!(cfg.cdp_port, 9222);
    }

    #[test]
    fn daemon_config_default() {
        let cfg = DaemonConfig::default();
        assert!(!cfg.auto_restart);
    }

    #[test]
    fn update_config_default() {
        let cfg = UpdateConfig::default();
        assert!(cfg.check_on_start);
        assert_eq!(cfg.channel, "stable");
    }

    #[test]
    fn personality_config_default() {
        let cfg = PersonalityConfig::default();
        assert_eq!(cfg.os_file, "OS.toml");
        assert_eq!(cfg.firmware_file, "FIRMWARE.toml");
    }

    #[test]
    fn mcp_config_default() {
        let cfg = McpConfig::default();
        assert!(!cfg.server_enabled);
        assert_eq!(cfg.server_port, 3001);
        assert!(cfg.clients.is_empty());
    }

    #[test]
    fn device_config_default() {
        let cfg = DeviceConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.identity_path.is_none());
        assert!(!cfg.sync_enabled);
        assert_eq!(cfg.max_paired_devices, 5);
    }

    #[test]
    fn discovery_config_default() {
        let cfg = DiscoveryConfig::default();
        assert!(!cfg.enabled);
        assert!(!cfg.dns_sd);
        assert!(!cfg.mdns);
        assert!(!cfg.advertise);
        assert_eq!(cfg.service_name, "_ironclad._tcp");
    }

    #[test]
    fn tier_adapt_config_default() {
        let cfg = TierAdaptConfig::default();
        assert!(!cfg.t1_strip_system);
        assert!(!cfg.t1_condense_turns);
        assert!(cfg.t3_t4_passthrough);
    }

    // ── validate() edge cases ───────────────────────────────────────────
