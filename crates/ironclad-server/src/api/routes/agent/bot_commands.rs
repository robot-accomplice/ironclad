//! Slash-command handlers for channel bots (/status, /model, /help, etc.).

use std::collections::HashMap;

use ironclad_core::InputAuthority;

use super::AppState;
use super::diagnostics::{collect_runtime_diagnostics, is_model_proxy_role, sanitize_diag_token};

pub(crate) async fn handle_bot_command(
    state: &AppState,
    command: &str,
    inbound: Option<&ironclad_channels::InboundMessage>,
) -> Option<String> {
    let (cmd, args) = command
        .split_once(|c: char| c.is_whitespace())
        .unwrap_or((command, ""));
    let cmd = cmd.split('@').next().unwrap_or(cmd);
    let args = args.trim();
    let authority = resolve_command_authority(state, inbound).await;

    match cmd {
        "/status" => {
            if authority < InputAuthority::Peer {
                Some("⛔ /status requires Peer authority or higher.".into())
            } else {
                Some(build_status_reply(state).await)
            }
        }
        "/model" => Some(handle_model_command(state, args, authority).await),
        "/models" => Some(handle_models_list(state).await),
        "/breaker" => Some(handle_breaker_command(state, args, authority).await),
        "/retry" => Some(handle_retry_command(state, inbound).await),
        "/help" => Some(HELP_TEXT.into()),
        _ => None,
    }
}

const HELP_TEXT: &str = "\
/status  — agent + subagent runtime health\n\
/model   — show current model & override\n\
/model <provider/name> — force a model override\n\
/model reset — clear override, resume normal routing\n\
/models  — list primary + fallback models\n\
/breaker — show circuit breaker status\n\
/breaker reset [provider] — reset tripped breakers\n\
/retry   — show last assistant response in this chat\n\
/help    — show this message\n\n\
Anything else is sent to the LLM.";

async fn resolve_command_authority(
    state: &AppState,
    inbound: Option<&ironclad_channels::InboundMessage>,
) -> InputAuthority {
    let Some(inbound) = inbound else {
        // Test/internal invocations without channel context keep full authority.
        return InputAuthority::Creator;
    };

    let config = state.config.read().await;
    let trusted = config.channels.trusted_sender_ids.clone();
    let security_config = config.security.clone();
    let chat_id = super::resolve_channel_chat_id(inbound);
    let platform = inbound.platform.to_lowercase();

    let (sender_in_allowlist, allowlist_configured) = match platform.as_str() {
        "telegram" => {
            if let Some(ref tg) = config.channels.telegram {
                // Telegram command authority is scoped to the chat context to align
                // with adapter allow-list semantics (chat IDs, not sender IDs).
                let in_list = tg
                    .allowed_chat_ids
                    .iter()
                    .any(|id| id.to_string() == chat_id);
                (
                    !tg.allowed_chat_ids.is_empty() && in_list,
                    !tg.allowed_chat_ids.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "whatsapp" => {
            if let Some(ref wa) = config.channels.whatsapp {
                let in_list = wa.allowed_numbers.iter().any(|n| n == &inbound.sender_id);
                (
                    !wa.allowed_numbers.is_empty() && in_list,
                    !wa.allowed_numbers.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "discord" => {
            if let Some(ref dc) = config.channels.discord {
                let in_list = dc.allowed_guild_ids.iter().any(|g| g == &chat_id);
                (
                    !dc.allowed_guild_ids.is_empty() && in_list,
                    !dc.allowed_guild_ids.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "signal" => {
            if let Some(ref sig) = config.channels.signal {
                let in_list = sig.allowed_numbers.iter().any(|n| n == &inbound.sender_id);
                (
                    !sig.allowed_numbers.is_empty() && in_list,
                    !sig.allowed_numbers.is_empty(),
                )
            } else {
                (false, false)
            }
        }
        "email" => {
            let sender_lc = inbound.sender_id.to_lowercase();
            let in_list = config
                .channels
                .email
                .allowed_senders
                .iter()
                .map(|s| s.to_lowercase())
                .any(|s| s == sender_lc);
            (
                !config.channels.email.allowed_senders.is_empty() && in_list,
                !config.channels.email.allowed_senders.is_empty(),
            )
        }
        _ => (false, false),
    };
    drop(config);

    ironclad_core::security::resolve_channel_claim(
        &ironclad_core::security::ChannelContext {
            sender_id: &inbound.sender_id,
            chat_id: &chat_id,
            channel: &platform,
            sender_in_allowlist,
            allowlist_configured,
            threat_is_caution: false,
            trusted_sender_ids: &trusted,
        },
        &security_config,
    )
    .authority
}

async fn handle_retry_command(
    state: &AppState,
    inbound: Option<&ironclad_channels::InboundMessage>,
) -> String {
    let Some(inbound) = inbound else {
        return "Retry requires a channel context. Send /retry in the target chat.".into();
    };

    let chat_id = super::resolve_channel_chat_id(inbound);
    let cfg = state.config.read().await;
    let scope = super::resolve_channel_scope(&cfg, inbound, &chat_id);
    let agent_id = cfg.agent.id.clone();
    drop(cfg);

    let session_id = match ironclad_db::sessions::find_or_create(&state.db, &agent_id, Some(&scope))
    {
        Ok(id) => id,
        Err(e) => return format!("Retry failed: {e}"),
    };
    let messages = match ironclad_db::sessions::list_messages(&state.db, &session_id, Some(100)) {
        Ok(m) => m,
        Err(e) => return format!("Retry failed: {e}"),
    };
    let target = messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant" && !m.content.trim().is_empty())
        .map(|m| m.content.clone());
    let Some(content) = target else {
        return "No prior assistant response found to retry in this chat.".into();
    };
    content
}

async fn handle_model_command(state: &AppState, args: &str, authority: InputAuthority) -> String {
    if args.is_empty() {
        let llm = state.llm.read().await;
        let current = llm.router.select_model().to_string();
        let primary = llm.router.primary().to_string();
        return match llm.router.get_override() {
            Some(ovr) => {
                format!("🔧 Model override active\n  override: {ovr}\n  primary: {primary}")
            }
            None => {
                format!("🤖 Current model: {current}\n  primary: {primary}\n  (no override set)")
            }
        };
    }

    if args == "reset" || args == "clear" {
        if authority != InputAuthority::Creator {
            return "⛔ /model reset requires Creator authority.".into();
        }
        let mut llm = state.llm.write().await;
        llm.router.clear_override();
        let current = llm.router.select_model().to_string();
        return format!("✅ Model override cleared. Routing normally → {current}");
    }

    let model_name = args.to_string();
    let has_provider = {
        let llm = state.llm.read().await;
        llm.providers.get_by_model(&model_name).is_some()
    };

    if !has_provider {
        return format!(
            "⚠️ Unknown model: {model_name}\n\
             Use /models to see available models, or specify as provider/model."
        );
    }

    if authority != InputAuthority::Creator {
        return "⛔ /model override requires Creator authority.".into();
    }

    let mut llm = state.llm.write().await;
    llm.router.set_override(model_name.clone());
    format!("✅ Model override set → {model_name}\nUse /model reset to return to normal routing.")
}

async fn handle_models_list(state: &AppState) -> String {
    let config = state.config.read().await;
    let llm = state.llm.read().await;

    let primary = &config.models.primary;
    let current = llm.router.select_model();
    let mut lines = vec!["📋 Configured models".to_string()];
    lines.push(format!("  primary: {primary}"));

    if !config.models.fallbacks.is_empty() {
        lines.push("  fallbacks:".into());
        for fb in &config.models.fallbacks {
            lines.push(format!("    • {fb}"));
        }
    } else {
        lines.push("  fallbacks: (none)".into());
    }

    if current != primary {
        lines.push(format!("  active: {current}"));
    }

    if let Some(ovr) = llm.router.get_override() {
        lines.push(format!("  override: {ovr}"));
    }

    lines.push(format!("  routing: {}", config.models.routing.mode));
    lines.join("\n")
}

async fn handle_breaker_command(state: &AppState, args: &str, authority: InputAuthority) -> String {
    if args.starts_with("reset") {
        if authority != InputAuthority::Creator {
            return "⛔ /breaker reset requires Creator authority.".into();
        }
        let provider = args.strip_prefix("reset").unwrap_or("").trim();
        let mut llm = state.llm.write().await;

        if provider.is_empty() {
            let providers: Vec<String> = llm
                .breakers
                .list_providers()
                .into_iter()
                .filter(|(_, s)| *s != ironclad_llm::CircuitState::Closed)
                .map(|(name, _)| name)
                .collect();

            if providers.is_empty() {
                return "✅ All circuit breakers are already closed.".into();
            }
            for p in &providers {
                llm.breakers.reset(p);
            }
            return format!(
                "✅ Reset {} circuit breaker(s): {}",
                providers.len(),
                providers.join(", ")
            );
        }

        llm.breakers.reset(provider);
        return format!("✅ Circuit breaker for '{provider}' reset to closed.");
    }

    let llm = state.llm.read().await;
    let providers = llm.breakers.list_providers();

    if providers.is_empty() {
        return "🔌 No circuit breaker state recorded yet.".into();
    }

    let mut lines = vec!["🔌 Circuit breaker status".to_string()];
    for (name, state) in &providers {
        let icon = match state {
            ironclad_llm::CircuitState::Closed => "🟢",
            ironclad_llm::CircuitState::Open => "🔴",
            ironclad_llm::CircuitState::HalfOpen => "🟡",
        };
        let credit_note = if llm.breakers.is_credit_tripped(name) {
            " (credit — requires /breaker reset)"
        } else {
            ""
        };
        lines.push(format!("  {icon} {name}: {state:?}{credit_note}"));
    }
    lines.push("\nUse /breaker reset [provider] to reset.".into());
    lines.join("\n")
}

pub(super) async fn build_status_reply(state: &AppState) -> String {
    let config = state.config.read().await;
    let diag = collect_runtime_diagnostics(state).await;
    let balance = state.wallet.wallet.get_usdc_balance().await.unwrap_or(0.0);
    let channels = state.channel_router.channel_status().await;
    let runtime_agents = state.registry.list_agents().await;
    let runtime_by_name: HashMap<String, ironclad_agent::subagents::AgentRunState> = runtime_agents
        .into_iter()
        .map(|a| (a.id.to_ascii_lowercase(), a.state))
        .collect();
    let configured_subagents = ironclad_db::agents::list_sub_agents(&state.db)
        .inspect_err(|e| tracing::error!(error = %e, "failed to list sub-agents for diagnostics"))
        .unwrap_or_default();
    let channel_summary: Vec<String> = channels
        .iter()
        .map(|c| {
            let err = if c.last_error.is_some() { " (err)" } else { "" };
            format!(
                "  {} — rx:{} tx:{}{}",
                sanitize_diag_token(&c.name, 32),
                c.messages_received,
                c.messages_sent,
                err
            )
        })
        .collect();
    let mut subagent_breakdown: Vec<String> = configured_subagents
        .iter()
        .filter(|a| !is_model_proxy_role(&a.role))
        .map(|a| {
            let state_label = if let Some(state) = runtime_by_name.get(&a.name.to_ascii_lowercase())
            {
                match state {
                    ironclad_agent::subagents::AgentRunState::Starting => "booting",
                    ironclad_agent::subagents::AgentRunState::Running => "running",
                    ironclad_agent::subagents::AgentRunState::Error => "error",
                    ironclad_agent::subagents::AgentRunState::Stopped => "stopped",
                    ironclad_agent::subagents::AgentRunState::Idle => {
                        if a.enabled {
                            "booting"
                        } else {
                            "stopped"
                        }
                    }
                }
            } else if a.enabled {
                "booting"
            } else {
                "stopped"
            };
            format!("{}={}", a.name, state_label)
        })
        .collect();
    subagent_breakdown.sort();

    let mut lines = vec![
        format!("🤖 {} ({})", config.agent.name, config.agent.id),
        "  state: running".to_string(),
        format!("  primary: {}", diag.primary_model),
    ];
    if diag.active_model != diag.primary_model {
        lines.push(format!("  current: {}", diag.active_model));
    }
    lines.extend([
        format!(
            "  provider: {} ({})",
            diag.primary_provider, diag.primary_provider_state
        ),
        format!(
            "  cache: {} entries, {:.0}% hit rate",
            diag.cache_entries, diag.cache_hit_rate_pct
        ),
        format!(
            "  taskable subagents: total={} enabled={} booting={} running={} error={}",
            diag.taskable_subagents_total,
            diag.taskable_subagents_enabled,
            diag.taskable_subagents_booting,
            diag.taskable_subagents_running,
            diag.taskable_subagents_error
        ),
        format!(
            "  subagent taskability: {} taskable now{}",
            diag.taskable_subagents_running,
            if diag.delegation_tools_available {
                String::new()
            } else {
                ", delegation tools unavailable".to_string()
            }
        ),
        format!(
            "  breakers: {} open, {} half-open",
            diag.breaker_open_count, diag.breaker_half_open_count
        ),
        format!("  approvals: {} pending", diag.pending_approvals),
        format!(
            "  channels: {} total, {} with errors",
            diag.channels_total, diag.channels_with_errors
        ),
        format!("  uptime: {}s", diag.uptime_seconds),
        format!("  wallet: {balance:.2} USDC"),
    ]);

    if !channel_summary.is_empty() {
        lines.push("  channels:".into());
        lines.extend(channel_summary);
    }
    if !subagent_breakdown.is_empty() {
        lines.push(format!("  subagents: {}", subagent_breakdown.join(", ")));
    }

    lines.join("\n")
}
