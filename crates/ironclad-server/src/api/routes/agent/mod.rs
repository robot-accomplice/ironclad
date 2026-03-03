//! Agent message, channel processing, and Telegram poll.

mod bot_commands;
mod channel_helpers;
mod channel_message;
mod core;
mod decomposition;
mod delegation;
mod diagnostics;
mod guards;
mod handlers;
mod orchestration;
mod poll_loops;
mod routing;
mod streaming;
#[cfg(test)]
mod tests;
mod tools;

use serde::Deserialize;

pub(crate) use self::bot_commands::handle_bot_command;
pub(crate) use self::channel_helpers::channel_chat_id_for_inbound;
#[cfg(test)]
use self::channel_helpers::{metadata_str, resolve_channel_is_group};
use self::channel_helpers::{parse_skills_json, resolve_channel_chat_id, resolve_channel_scope};
pub use self::channel_message::process_channel_message;
use self::decomposition::{DelegationProvenance, capability_tokens};
#[cfg(test)]
use self::decomposition::{
    SpecialistProposal, proposal_to_json, split_subtasks, utility_margin_for_delegation,
};
use self::delegation::{execute_virtual_subagent_tool_call, is_virtual_delegation_tool};
pub use self::diagnostics::agent_status;
use self::diagnostics::is_model_proxy_role;
#[cfg(test)]
use self::diagnostics::{RuntimeDiagnostics, diagnostics_system_note, sanitize_diag_token};
use self::guards::resolve_web_scope;
#[cfg(test)]
use self::guards::{
    MAX_SCOPE_ID, claims_unverified_subagent_output, common_prefix_ratio, enforce_non_repetition,
    enforce_subagent_claim_guard, looks_repetitive, repeat_tokens,
};
pub use self::handlers::agent_message;
use self::orchestration::{execute_virtual_orchestration_tool, is_virtual_orchestration_tool};
pub(crate) use self::poll_loops::CHANNEL_PROCESSING_ERROR_REPLY;
pub use self::poll_loops::{
    discord_poll_loop, email_poll_loop, signal_poll_loop, telegram_poll_loop,
};
use self::routing::{DELEGATED_INFERENCE_BUDGET, infer_with_fallback_with_budget_and_preferred};
#[cfg(test)]
use self::routing::{estimate_cost_from_provider, fallback_candidates, summarize_user_excerpt};
pub(crate) use self::routing::{infer_content_with_fallback, select_routed_model};
pub use self::streaming::agent_message_stream;
pub(crate) use self::tools::{
    check_tool_policy, classify_provider_error, execute_tool_call, execute_tool_call_after_approval,
};
#[cfg(test)]
use self::tools::{parse_tool_call, provider_failure_user_message};
use super::AppState;
#[cfg(test)]
use super::JsonError;
#[cfg(test)]
use axum::http::StatusCode;

#[derive(Deserialize)]
pub struct AgentMessageRequest {
    content: String,
    #[serde(default)]
    session_id: Option<String>,
    #[serde(default)]
    channel: Option<String>,
    #[serde(default)]
    sender_id: Option<String>,
    #[serde(default)]
    peer_id: Option<String>,
    #[serde(default)]
    group_id: Option<String>,
    #[serde(default)]
    is_group: Option<bool>,
}
