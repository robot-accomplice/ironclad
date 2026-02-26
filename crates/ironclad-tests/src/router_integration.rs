use std::collections::HashMap;

use ironclad_core::config::{CircuitBreakerConfig, RoutingConfig};
use ironclad_core::{ApiFormat, ModelTier};
use ironclad_llm::{CircuitBreakerRegistry, ModelRouter, Provider, ProviderRegistry};

fn mk_provider(name: &str, is_local: bool, in_cost: f64, out_cost: f64) -> Provider {
    Provider {
        name: name.to_string(),
        url: format!("http://{name}.example.com"),
        tier: if is_local {
            ModelTier::T1
        } else {
            ModelTier::T2
        },
        api_key_env: format!("{}_API_KEY", name.to_uppercase()),
        format: ApiFormat::OpenAiCompletions,
        chat_path: "/v1/chat/completions".to_string(),
        embedding_path: None,
        embedding_model: None,
        embedding_dimensions: None,
        is_local,
        cost_per_input_token: in_cost,
        cost_per_output_token: out_cost,
        auth_header: "Authorization".to_string(),
        extra_headers: HashMap::new(),
        tpm_limit: None,
        rpm_limit: None,
        auth_mode: "api_key".to_string(),
        oauth_client_id: None,
        api_key_ref: None,
    }
}

fn mk_registry() -> ProviderRegistry {
    let mut reg = ProviderRegistry::new();
    reg.register(mk_provider("ollama", true, 0.0, 0.0));
    reg.register(mk_provider("moonshot", false, 0.000001, 0.000002));
    reg.register(mk_provider("anthropic", false, 0.000003, 0.000015));
    reg
}

fn mk_breakers() -> CircuitBreakerRegistry {
    let cfg = CircuitBreakerConfig {
        threshold: 1,
        window_seconds: 60,
        cooldown_seconds: 300,
        credit_cooldown_seconds: 300,
        max_cooldown_seconds: 900,
    };
    CircuitBreakerRegistry::new(&cfg)
}

#[test]
fn router_local_first_prefers_primary_for_low_complexity() {
    let cfg = RoutingConfig {
        mode: "ml".to_string(),
        confidence_threshold: 0.8,
        local_first: true,
        ..Default::default()
    };
    let router = ModelRouter::new(
        "ollama/qwen3:14b".to_string(),
        vec!["moonshot/kimi-k2-turbo-preview".to_string()],
        cfg,
        Box::new(ironclad_llm::router::HeuristicBackend),
    );
    let reg = mk_registry();
    let breakers = mk_breakers();

    let selected = router.select_for_complexity(0.2, Some(&reg), None, Some(&breakers));
    assert_eq!(selected, "ollama/qwen3:14b");
}

#[test]
fn router_skips_blocked_first_choice() {
    let cfg = RoutingConfig {
        mode: "ml".to_string(),
        confidence_threshold: 0.5,
        local_first: false,
        ..Default::default()
    };
    let router = ModelRouter::new(
        "moonshot/kimi-k2-turbo-preview".to_string(),
        vec![
            "anthropic/claude-sonnet-4-6".to_string(),
            "ollama/qwen3:14b".to_string(),
        ],
        cfg,
        Box::new(ironclad_llm::router::HeuristicBackend),
    );
    let reg = mk_registry();
    let mut breakers = mk_breakers();
    breakers.record_credit_error("moonshot");

    let selected = router.select_for_complexity(0.95, Some(&reg), None, Some(&breakers));
    assert_eq!(selected, "anthropic/claude-sonnet-4-6");
}

#[test]
fn router_cost_aware_chooses_cheapest_eligible() {
    let cfg = RoutingConfig {
        mode: "ml".to_string(),
        confidence_threshold: 0.9,
        local_first: false,
        ..Default::default()
    };
    let router = ModelRouter::new(
        "anthropic/claude-sonnet-4-6".to_string(),
        vec!["moonshot/kimi-k2-turbo-preview".to_string()],
        cfg,
        Box::new(ironclad_llm::router::HeuristicBackend),
    );
    let reg = mk_registry();
    let breakers = mk_breakers();

    let selected = router.select_cheapest_qualified(0.3, &reg, None, Some(&breakers), 1000, 500);
    assert_eq!(selected, "moonshot/kimi-k2-turbo-preview");
}

#[test]
fn router_override_short_circuits_then_clear_restores() {
    let cfg = RoutingConfig {
        mode: "ml".to_string(),
        confidence_threshold: 0.7,
        local_first: true,
        ..Default::default()
    };
    let mut router = ModelRouter::new(
        "ollama/qwen3:14b".to_string(),
        vec!["moonshot/kimi-k2-turbo-preview".to_string()],
        cfg,
        Box::new(ironclad_llm::router::HeuristicBackend),
    );
    let reg = mk_registry();
    let breakers = mk_breakers();

    router.set_override("anthropic/claude-sonnet-4-6".to_string());
    let selected = router.select_for_complexity(0.1, Some(&reg), None, Some(&breakers));
    assert_eq!(selected, "anthropic/claude-sonnet-4-6");

    router.clear_override();
    let selected = router.select_for_complexity(0.1, Some(&reg), None, Some(&breakers));
    assert_eq!(selected, "ollama/qwen3:14b");
}

#[test]
fn router_falls_through_multiple_blocked_candidates() {
    let cfg = RoutingConfig {
        mode: "ml".to_string(),
        confidence_threshold: 0.5,
        local_first: false,
        ..Default::default()
    };
    let router = ModelRouter::new(
        "moonshot/kimi-k2-turbo-preview".to_string(),
        vec![
            "anthropic/claude-sonnet-4-6".to_string(),
            "ollama/qwen3:14b".to_string(),
        ],
        cfg,
        Box::new(ironclad_llm::router::HeuristicBackend),
    );
    let reg = mk_registry();
    let mut breakers = mk_breakers();
    breakers.record_credit_error("moonshot");
    breakers.record_credit_error("anthropic");

    let selected = router.select_for_complexity(0.95, Some(&reg), None, Some(&breakers));
    assert_eq!(
        selected, "ollama/qwen3:14b",
        "router should continue fallback traversal until it finds a closed provider"
    );
}
