//! Runtime diagnostics, status endpoint, and diagnostic helpers.

use std::collections::HashSet;

use axum::extract::State;
use axum::response::IntoResponse;
use serde::Serialize;
use serde_json::json;

use super::AppState;

#[derive(Debug, Clone, Serialize)]
pub(super) struct RuntimeDiagnostics {
    pub uptime_seconds: u64,
    pub primary_model: String,
    pub active_model: String,
    pub primary_provider: String,
    pub primary_provider_state: String,
    pub breaker_open_count: usize,
    pub breaker_half_open_count: usize,
    pub cache_entries: usize,
    pub cache_hit_rate_pct: f64,
    pub pending_approvals: usize,
    pub taskable_subagents_total: usize,
    pub taskable_subagents_enabled: usize,
    pub taskable_subagents_booting: usize,
    pub taskable_subagents_running: usize,
    pub taskable_subagents_error: usize,
    pub delegation_tools_available: bool,
    pub channels_total: usize,
    pub channels_with_errors: usize,
}

pub(super) fn sanitize_diag_token(raw: &str, max_len: usize) -> String {
    let cleaned: String = raw
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '/' | '.' | ':'))
        .collect();
    let trimmed = cleaned.trim_matches(|c| c == '-' || c == '_' || c == ':' || c == '/');
    trimmed.chars().take(max_len).collect()
}

pub(super) fn is_model_proxy_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("model-proxy")
}

pub(super) async fn collect_runtime_diagnostics(state: &AppState) -> RuntimeDiagnostics {
    let (
        primary_model,
        active_model,
        primary_provider,
        primary_provider_state,
        cache_entries,
        cache_hit_rate_pct,
        breaker_open_count,
        breaker_half_open_count,
    ) = {
        let config = state.config.read().await;
        let llm = state.llm.read().await;
        let primary_model = sanitize_diag_token(&config.models.primary, 120);
        let active_model = sanitize_diag_token(llm.router.select_model(), 120);
        let primary_provider = sanitize_diag_token(
            config.models.primary.split('/').next().unwrap_or("unknown"),
            40,
        );
        let primary_provider_state =
            format!("{:?}", llm.breakers.get_state(&primary_provider)).to_lowercase();
        let providers = llm.breakers.list_providers();
        let breaker_open_count = providers
            .iter()
            .filter(|(_, s)| *s == ironclad_llm::CircuitState::Open)
            .count();
        let breaker_half_open_count = providers
            .iter()
            .filter(|(_, s)| *s == ironclad_llm::CircuitState::HalfOpen)
            .count();
        let cache_entries = llm.cache.size();
        let hits = llm.cache.hit_count();
        let misses = llm.cache.miss_count();
        let cache_hit_rate_pct = if hits + misses > 0 {
            (hits as f64 / (hits + misses) as f64) * 100.0
        } else {
            0.0
        };
        (
            primary_model,
            active_model,
            primary_provider,
            primary_provider_state,
            cache_entries,
            cache_hit_rate_pct,
            breaker_open_count,
            breaker_half_open_count,
        )
    };

    let channels = state.channel_router.channel_status().await;
    let channels_with_errors = channels.iter().filter(|c| c.last_error.is_some()).count();
    let runtime_agents = state.registry.list_agents().await;
    let configured_subagents = ironclad_db::agents::list_sub_agents(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to list sub-agents for status"))
        .unwrap_or_default();
    let model_proxy_names: HashSet<String> = configured_subagents
        .iter()
        .filter(|a| is_model_proxy_role(&a.role))
        .map(|a| a.name.to_ascii_lowercase())
        .collect();
    let taskable_subagents_running = runtime_agents
        .iter()
        .filter(|a| !model_proxy_names.contains(&a.id.to_ascii_lowercase()))
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Running)
        .count();
    let taskable_subagents_booting = runtime_agents
        .iter()
        .filter(|a| !model_proxy_names.contains(&a.id.to_ascii_lowercase()))
        .filter(|a| {
            matches!(
                a.state,
                ironclad_agent::subagents::AgentRunState::Starting
                    | ironclad_agent::subagents::AgentRunState::Idle
            )
        })
        .count();
    let taskable_subagents_error = runtime_agents
        .iter()
        .filter(|a| !model_proxy_names.contains(&a.id.to_ascii_lowercase()))
        .filter(|a| a.state == ironclad_agent::subagents::AgentRunState::Error)
        .count();
    let taskable_subagents_total = configured_subagents
        .iter()
        .filter(|a| !is_model_proxy_role(&a.role))
        .count();
    let taskable_subagents_enabled = configured_subagents
        .iter()
        .filter(|a| !is_model_proxy_role(&a.role) && a.enabled)
        .count();
    let pending_approvals = state.approvals.list_pending().len();
    let delegation_tools_available = {
        let cfg = state.config.read().await;
        cfg.agent.delegation_enabled
            && (state.tools.list().iter().any(|t| {
                let name = t.name().to_ascii_lowercase();
                name.contains("subagent") || name.contains("delegate")
            }) || super::is_virtual_delegation_tool("orchestrate-subagents")
                || super::is_virtual_orchestration_tool("compose-subagent"))
    };

    RuntimeDiagnostics {
        uptime_seconds: state.started_at.elapsed().as_secs(),
        primary_model,
        active_model,
        primary_provider,
        primary_provider_state,
        breaker_open_count,
        breaker_half_open_count,
        cache_entries,
        cache_hit_rate_pct,
        pending_approvals,
        taskable_subagents_total,
        taskable_subagents_enabled,
        taskable_subagents_booting,
        taskable_subagents_running,
        taskable_subagents_error,
        delegation_tools_available,
        channels_total: channels.len(),
        channels_with_errors,
    }
}

pub(super) fn diagnostics_system_note(diag: &RuntimeDiagnostics) -> String {
    let delegation_policy = if !diag.delegation_tools_available {
        "Delegation policy: delegated subagent tools are unavailable in this runtime. Do NOT claim delegation, stand-by status, or subagent-produced output."
    } else if diag.taskable_subagents_booting > 0 && diag.taskable_subagents_running == 0 {
        "Delegation policy: subagents are booting and are not taskable yet. Report booting status and wait for running>0 before claiming delegated execution."
    } else if diag.taskable_subagents_running == 0 && diag.taskable_subagents_enabled > 0 {
        "Delegation policy: subagent execution is currently unavailable (enabled>0, running=0). If the user asks for a subagent-produced result, explicitly say it is unavailable and do NOT simulate or fabricate subagent output."
    } else {
        "Delegation policy: never claim a subagent produced content unless a real delegated subagent turn occurred."
    };
    // Guardrails: aggregate-only metrics; no secrets, no raw error strings, no IDs.
    [
        "Runtime diagnostics (internal, bounded):",
        &format!(
            "- models: active={} primary={}",
            diag.active_model, diag.primary_model
        ),
        &format!(
            "- provider: {} ({}) | breaker_open={} half_open={}",
            diag.primary_provider,
            diag.primary_provider_state,
            diag.breaker_open_count,
            diag.breaker_half_open_count
        ),
        &format!(
            "- cache: entries={} hit_rate={:.0}%",
            diag.cache_entries, diag.cache_hit_rate_pct
        ),
        &format!(
            "- taskable_subagents: total={} enabled={} booting={} running={} error={}",
            diag.taskable_subagents_total,
            diag.taskable_subagents_enabled,
            diag.taskable_subagents_booting,
            diag.taskable_subagents_running,
            diag.taskable_subagents_error
        ),
        &format!(
            "- delegation_tools_available={}",
            diag.delegation_tools_available
        ),
        &format!(
            "- approvals_pending={} channels={} channels_with_errors={}",
            diag.pending_approvals, diag.channels_total, diag.channels_with_errors
        ),
        &format!("- uptime_seconds={}", diag.uptime_seconds),
        "Security policy: do not proactively disclose internal diagnostics. Share high-level status only when asked; never fabricate details.",
        delegation_policy,
    ]
    .join("\n")
}

pub async fn agent_status(State(state): State<AppState>) -> impl IntoResponse {
    let config = state.config.read().await;
    let diag = collect_runtime_diagnostics(&state).await;

    axum::Json(json!({
        "state": "running",
        "name": config.agent.name,
        "agent_name": config.agent.name,
        "agent_id": config.agent.id,
        "primary_model": diag.primary_model,
        "active_model": diag.active_model,
        "primary_provider_state": diag.primary_provider_state,
        "cache_entries": diag.cache_entries,
        "cache_hit_rate_pct": diag.cache_hit_rate_pct,
        "diagnostics": diag,
    }))
}
