    use super::*;
    use proptest::prelude::*;

    proptest! {
        #[test]
        fn config_toml_roundtrip_preserves_values(port in 1024u16..=65535u16) {
            let toml_str = format!(r#"
[agent]
name = "TestBot"
id = "test"
workspace = "/tmp/test"
log_level = "debug"

[server]
bind = "127.0.0.1"
port = {port}

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#);
            let config = IroncladConfig::from_str(&toml_str).unwrap();
            assert_eq!(config.server.port, port);
            assert_eq!(config.server.bind, "127.0.0.1");
        }
    }

    fn minimal_toml() -> &'static str {
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
"#
    }

    #[test]
    fn parse_minimal_config() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.agent.name, "TestBot");
        assert_eq!(cfg.agent.id, "test");
        assert_eq!(cfg.server.port, 9999);
        assert_eq!(cfg.models.primary, "ollama/qwen3:8b");
    }

    #[test]
    fn defaults_applied() {
        let cfg = IroncladConfig::from_str(minimal_toml()).unwrap();
        assert_eq!(cfg.memory.working_budget_pct, 30.0);
        assert_eq!(cfg.memory.episodic_budget_pct, 25.0);
        assert_eq!(cfg.memory.semantic_budget_pct, 20.0);
        assert_eq!(cfg.memory.procedural_budget_pct, 15.0);
        assert_eq!(cfg.memory.relationship_budget_pct, 10.0);
        assert_eq!(cfg.cache.semantic_threshold, 0.95);
        assert_eq!(cfg.cache.max_entries, 10000);
        assert_eq!(cfg.treasury.per_payment_cap, 100.0);
        assert!(cfg.skills.sandbox_env);
        assert_eq!(cfg.skills.script_timeout_seconds, 30);
        assert_eq!(
            cfg.skills.allowed_interpreters,
            vec!["bash", "python3", "node"]
        );
        assert_eq!(cfg.a2a.max_message_size, 65536);
        assert_eq!(cfg.a2a.rate_limit_per_peer, 10);
        assert!(cfg.a2a.enabled);
        assert_eq!(cfg.agent.autonomy_max_react_turns, 10);
        assert_eq!(cfg.agent.autonomy_max_turn_duration_seconds, 90);
    }

    #[test]
    fn autonomy_budget_validation_fail() {
        let toml = r#"
[agent]
name = "TestBot"
id = "test"
autonomy_max_react_turns = 0

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("autonomy_max_react_turns"));

        let toml2 = r#"
[agent]
name = "TestBot"
id = "test"
autonomy_max_turn_duration_seconds = 0

[server]
port = 9999

[database]
path = "/tmp/test.db"

[models]
primary = "ollama/qwen3:8b"
"#;
        let err2 = IroncladConfig::from_str(toml2).unwrap_err();
        assert!(
            err2.to_string()
                .contains("autonomy_max_turn_duration_seconds")
        );
    }

    #[test]
    fn memory_budget_validation_fail() {
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

[memory]
working_budget_pct = 50.0
episodic_budget_pct = 25.0
semantic_budget_pct = 20.0
procedural_budget_pct = 15.0
relationship_budget_pct = 10.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("sum to 100"));
    }

    #[test]
    fn treasury_validation_fail() {
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
per_payment_cap = -1.0
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("per_payment_cap"));
    }

    #[test]
    fn revenue_swap_validation_requires_default_chain_to_exist() {
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

[treasury.revenue_swap]
enabled = true
target_symbol = "PALM_USD"
default_chain = "ARBITRUM"

[[treasury.revenue_swap.chains]]
chain = "ETH"
target_contract_address = "0xfaf0cee6b20e2aaa4b80748a6af4cd89609a3d78"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("default_chain"));
    }

    #[test]
    fn revenue_swap_validation_rejects_duplicate_chain_entries() {
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

[treasury.revenue_swap]
enabled = true
target_symbol = "PALM_USD"
default_chain = "ETH"

[[treasury.revenue_swap.chains]]
chain = "ETH"
target_contract_address = "0xfaf0cee6b20e2aaa4b80748a6af4cd89609a3d78"

[[treasury.revenue_swap.chains]]
chain = "eth"
target_contract_address = "0x1111111111111111111111111111111111111111"
"#;
        let err = IroncladConfig::from_str(toml).unwrap_err();
        assert!(err.to_string().contains("duplicate chain"));
    }

    #[test]
    fn full_config_roundtrip() {
        let toml = r#"
[agent]
name = "Duncan Idaho"
id = "duncan"
workspace = "/tmp/workspace"
log_level = "debug"

[server]
port = 18789
bind = "0.0.0.0"

[database]
path = "/tmp/state.db"

[models]
primary = "openai/gpt-5.3-codex"
fallbacks = ["google/gemini-3-flash", "ollama/qwen3:14b"]

[models.routing]
mode = "metascore"
confidence_threshold = 0.85
local_first = true

[providers.anthropic]
url = "https://api.anthropic.com"
tier = "T3"

[providers.ollama]
url = "http://localhost:11434"
tier = "T1"

[circuit_breaker]
threshold = 5
window_seconds = 120

[memory]
working_budget_pct = 30.0
episodic_budget_pct = 25.0
semantic_budget_pct = 20.0
procedural_budget_pct = 15.0
relationship_budget_pct = 10.0

[cache]
enabled = true
exact_match_ttl_seconds = 7200
semantic_threshold = 0.92
max_entries = 5000

[treasury]
per_payment_cap = 50.0
hourly_transfer_limit = 200.0
daily_transfer_limit = 1000.0
minimum_reserve = 10.0
daily_inference_budget = 25.0

[yield]
enabled = false
protocol = "aave"
chain = "base"
min_deposit = 100.0
withdrawal_threshold = 50.0

[wallet]
path = "/tmp/wallet.json"
chain_id = 8453
rpc_url = "https://mainnet.base.org"

[a2a]
enabled = true
max_message_size = 32768
rate_limit_per_peer = 5
session_timeout_seconds = 1800
require_on_chain_identity = true

[skills]
skills_dir = "/tmp/skills"
script_timeout_seconds = 15
script_max_output_bytes = 524288
allowed_interpreters = ["bash", "python3"]
sandbox_env = true
hot_reload = true
"#;
        let cfg = IroncladConfig::from_str(toml).unwrap();
        assert_eq!(cfg.agent.name, "Duncan Idaho");
        assert_eq!(cfg.models.routing.confidence_threshold, 0.85);
        assert!(
            cfg.providers.len() >= 2,
            "user providers plus bundled defaults"
        );
        assert!(cfg.providers.contains_key("anthropic"));
        assert!(cfg.providers.contains_key("ollama"));
        assert_eq!(cfg.providers["anthropic"].url, "https://api.anthropic.com");
        assert_eq!(cfg.providers["anthropic"].tier, "T3");
        assert_eq!(cfg.circuit_breaker.threshold, 5);
        assert_eq!(cfg.cache.semantic_threshold, 0.92);
        assert_eq!(cfg.treasury.per_payment_cap, 50.0);
        assert!(!cfg.r#yield.enabled);
        assert_eq!(cfg.a2a.max_message_size, 32768);
        assert_eq!(cfg.skills.script_timeout_seconds, 15);
        assert_eq!(cfg.skills.allowed_interpreters, vec!["bash", "python3"]);
    }

    #[test]
    fn config_from_missing_file() {
        let result = IroncladConfig::from_file(Path::new("/nonexistent/config.toml"));
        assert!(result.is_err());
    }
