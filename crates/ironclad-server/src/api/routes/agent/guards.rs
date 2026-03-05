//! Dedup guard, subagent claim enforcement, and repetition detection.

use std::collections::HashSet;
use std::sync::Arc;

use super::intents::{
    requests_cron, requests_current_events, requests_delegation, requests_execution,
    requests_model_identity,
};

/// RAII guard that releases a dedup fingerprint when dropped.
/// Ensures cleanup on all exit paths, including async stream disconnects.
pub(super) struct DedupGuard {
    pub llm: Arc<tokio::sync::RwLock<ironclad_llm::LlmService>>,
    pub fingerprint: String,
}

impl Drop for DedupGuard {
    fn drop(&mut self) {
        if self.fingerprint.is_empty() {
            return;
        }
        let llm = Arc::clone(&self.llm);
        let fp = std::mem::take(&mut self.fingerprint);
        tokio::spawn(async move {
            let mut llm = llm.write().await;
            llm.dedup.release(&fp);
        });
    }
}

pub(super) fn claims_unverified_subagent_output(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    let markers = [
        "[delegating to subagent",
        "delegating to geopolitical specialist now",
        "came directly from the running subagent",
        "came directly from a running subagent",
        "subagent status - live",
        "geopolitical flash update",
        "standing by for tasking",
        "taskable subagents operational",
        "subagent-generated sitrep",
        "subagent-generated",
        "geopolitical specialist is live",
    ];
    markers.iter().any(|m| lower.contains(m))
}

pub(super) fn enforce_subagent_claim_guard(
    response: String,
    provenance: &super::DelegationProvenance,
) -> String {
    let allow_claim = provenance.subagent_task_started
        && provenance.subagent_task_completed
        && provenance.subagent_result_attached;
    if allow_claim || !claims_unverified_subagent_output(&response) {
        return response;
    }
    tracing::warn!("Blocking unverified channel response that claims subagent-produced output");
    "By your command, I will not fake delegation. I can't claim live subagent-produced output unless I actually run a delegated subagent/tool turn in this reply. Ask me to run a concrete delegated task and I'll return that output directly."
        .to_string()
}

pub(super) fn repeat_tokens(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|tok| tok.len() >= 3)
        .map(|tok| tok.to_string())
        .collect()
}

pub(super) fn common_prefix_ratio(a: &str, b: &str) -> f64 {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    let max_len = a_chars.len().max(b_chars.len());
    if max_len == 0 {
        return 0.0;
    }
    let shared = a_chars
        .iter()
        .zip(b_chars.iter())
        .take_while(|(ac, bc)| ac == bc)
        .count();
    shared as f64 / max_len as f64
}

pub(super) fn looks_repetitive(current: &str, previous: &str) -> bool {
    let cur = current.trim();
    let prev = previous.trim();
    if cur.is_empty() || prev.is_empty() {
        return false;
    }
    if cur.eq_ignore_ascii_case(prev) {
        return true;
    }
    if cur.len() < 80 || prev.len() < 80 {
        return false;
    }

    let a = repeat_tokens(cur);
    let b = repeat_tokens(prev);
    if a.is_empty() || b.is_empty() {
        return false;
    }
    let overlap = a.intersection(&b).count() as f64;
    let denom = a.len().max(b.len()) as f64;
    let overlap_ratio = overlap / denom;
    let prefix_ratio = common_prefix_ratio(&cur.to_ascii_lowercase(), &prev.to_ascii_lowercase());
    overlap_ratio >= 0.86 || (overlap_ratio >= 0.72 && prefix_ratio >= 0.55)
}

pub(super) fn enforce_non_repetition(response: String, previous_assistant: Option<&str>) -> String {
    if previous_assistant.is_some_and(|prev| looks_repetitive(&response, prev)) {
        return "I don't have a new verified update beyond my previous reply. I can run a fresh check now and report only what changed."
            .to_string();
    }
    response
}

fn looks_like_unexecuted_claim(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    lower.contains("\"tool_call\"")
        || lower.contains("you can use the following")
        || lower.contains("you can run")
        || lower.contains("would use the following")
        || lower.contains("crontab entry")
        || lower.contains("unable to directly execute")
}

pub(super) fn enforce_execution_truth_guard(
    user_prompt: &str,
    response: String,
    tool_results: &[(String, String)],
) -> String {
    if requests_delegation(user_prompt)
        && !tool_results.iter().any(|(name, output)| {
            let n = name.to_ascii_lowercase();
            let is_delegate_tool = n.contains("subagent")
                || n.contains("delegate")
                || n.contains("assign")
                || n.contains("orchestrate");
            let succeeded = !output.to_ascii_lowercase().starts_with("error:");
            is_delegate_tool && succeeded
        })
    {
        tracing::warn!("execution truth guard blocked unverified delegation claim");
        return "By your command, execution truth is strict: I did not execute a delegated subagent task for that request. I can only claim delegated results when a subagent tool call actually runs."
            .to_string();
    }
    if requests_cron(user_prompt)
        && !tool_results.iter().any(|(name, output)| {
            name.to_ascii_lowercase().contains("cron")
                && !output.to_ascii_lowercase().starts_with("error:")
        })
    {
        tracing::warn!("execution truth guard blocked unverified cron claim");
        return "By your command, execution truth is strict: I did not execute a cron scheduling tool for that request. I can only confirm schedules that were actually created or validated by a tool run."
            .to_string();
    }

    if !requests_execution(user_prompt) {
        return response;
    }
    if !tool_results.is_empty() {
        return response;
    }
    let lower = response.to_ascii_lowercase();
    if lower.contains("encountered an error reaching all llm providers") {
        return response;
    }
    if looks_like_unexecuted_claim(&response)
        || lower.contains("tool successfully executed")
        || lower.contains("the `")
        || lower.starts_with('{')
    {
        tracing::warn!("execution truth guard rewrote unverified execution-style response");
        return "By your command, execution truth is strict: I did not execute a tool for that request. I can only claim execution when I actually run a tool and return its output."
            .to_string();
    }
    // If there is no explicit execution claim, keep the response.
    response
}

pub(super) fn enforce_model_identity_truth_guard(
    user_prompt: &str,
    response: String,
    executed_model: &str,
    agent_name: &str,
) -> String {
    if !requests_model_identity(user_prompt) {
        return response;
    }
    tracing::warn!(
        executed_model,
        "model identity guard emitted canonical model identity"
    );
    format!("{agent_name} reporting in. I am currently running on {executed_model}.")
}

fn contains_foreign_identity_boilerplate(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    let markers = [
        "as an ai developed by microsoft",
        "as an ai developed by",
        "as an ai language model",
        "as a language model",
        "i am claude",
        "i'm claude",
        "i am chatgpt",
        "i'm chatgpt",
    ];
    markers.iter().any(|m| lower.contains(m))
}

fn filter_foreign_identity_sentences(response: &str) -> String {
    let markers = [
        "as an ai developed by microsoft",
        "as an ai developed by",
        "as an ai language model",
        "as a language model",
        "i am claude",
        "i'm claude",
        "i am chatgpt",
        "i'm chatgpt",
    ];

    let mut out = String::new();
    for chunk in response.split_inclusive(['\n', '.', '!', '?']) {
        let lower = chunk.to_ascii_lowercase();
        if markers.iter().any(|m| lower.contains(m)) {
            continue;
        }
        out.push_str(chunk);
    }
    out.trim().to_string()
}

pub(super) fn enforce_personality_integrity_guard(
    user_prompt: &str,
    response: String,
    agent_name: &str,
    executed_model: &str,
) -> String {
    if !contains_foreign_identity_boilerplate(&response) {
        return response;
    }
    tracing::warn!("personality integrity guard stripped foreign identity boilerplate");
    let cleaned = filter_foreign_identity_sentences(&response);
    if !cleaned.is_empty() {
        return cleaned;
    }
    let lower_prompt = user_prompt.to_ascii_lowercase();
    let asks_release_summary = lower_prompt.contains("release")
        || lower_prompt.contains("changelog")
        || lower_prompt.contains("linkedin")
        || lower_prompt.contains("x.com")
        || lower_prompt.contains("twitter")
        || lower_prompt.contains("v0.9.5")
        || lower_prompt.contains("0.9.5");
    if asks_release_summary {
        return "I need concrete Ironclad 0.9.5 context to summarize accurately. I can pull from changelog/roadmap memory if available, or you can provide release notes and I’ll format them for operator, LinkedIn, and X."
            .to_string();
    }
    if requests_model_identity(user_prompt) {
        return format!(
            "I am {} and I am currently running on {}.",
            agent_name, executed_model
        );
    }
    format!(
        "I’m {}. I’ll continue in my configured voice and avoid foreign boilerplate.",
        agent_name
    )
}

fn looks_like_stale_knowledge_disclaimer(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    lower.contains("as of my last update")
        || lower.contains("as of my last training")
        || lower.contains("i cannot provide real-time updates")
        || lower.contains("i can't provide real-time updates")
        || lower.contains("as of early 2023")
        || lower.contains("as of 2023")
}

pub(super) fn enforce_current_events_truth_guard(user_prompt: &str, response: String) -> String {
    if !requests_current_events(user_prompt) {
        return response;
    }
    if !looks_like_stale_knowledge_disclaimer(&response) {
        return response;
    }
    tracing::warn!("current-events guard blocked stale-knowledge disclaimer response");
    "Acknowledged. I cannot provide a current-events sitrep from stale memory. I will run live retrieval/delegation and return an up-to-date report with the current date."
        .to_string()
}

// ── Scope validation ──────────────────────────────────────────

/// Max length for scope identifiers (peer_id, group_id, channel).
pub(super) const MAX_SCOPE_ID: usize = 256;

/// Validate a scope identifier: reject control chars and enforce length cap.
pub(super) fn validate_scope_id(value: &str, field: &'static str) -> Result<(), &'static str> {
    if value.len() > MAX_SCOPE_ID {
        return Err(field);
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(field);
    }
    Ok(())
}

pub(super) fn resolve_web_scope(
    cfg: &ironclad_core::IroncladConfig,
    body: &super::AgentMessageRequest,
) -> Result<ironclad_db::sessions::SessionScope, &'static str> {
    let channel = body
        .channel
        .as_deref()
        .unwrap_or("web")
        .trim()
        .to_lowercase();
    validate_scope_id(
        &channel,
        "channel exceeds max length or contains control characters",
    )?;
    let peer_id = body
        .peer_id
        .clone()
        .or_else(|| body.sender_id.clone())
        .and_then(|id| {
            let trimmed = id.trim();
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        });
    if let Some(ref pid) = peer_id {
        validate_scope_id(
            pid,
            "peer_id exceeds max length or contains control characters",
        )?;
    }
    if let Some(ref gid) = body.group_id {
        validate_scope_id(
            gid.trim(),
            "group_id exceeds max length or contains control characters",
        )?;
    }
    let mode = cfg.session.scope_mode.as_str();
    if (mode == "group"
        || (mode != "agent" && body.is_group == Some(true) && body.group_id.is_some()))
        && let Some(group_id) = body.group_id.clone().filter(|s| !s.trim().is_empty())
    {
        return Ok(ironclad_db::sessions::SessionScope::Group { group_id, channel });
    }
    if mode == "peer" || mode == "group" {
        let Some(peer_id) = peer_id else {
            return Err("peer_id or sender_id is required when session.scope_mode is peer/group");
        };
        return Ok(ironclad_db::sessions::SessionScope::Peer { peer_id, channel });
    }
    Ok(ironclad_db::sessions::SessionScope::Agent)
}
