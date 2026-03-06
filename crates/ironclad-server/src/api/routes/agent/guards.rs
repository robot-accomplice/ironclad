//! Dedup guard, subagent claim enforcement, and repetition detection.

use std::collections::HashSet;
use std::sync::Arc;

use super::intents::{
    requests_acknowledgement, requests_cron, requests_current_events, requests_delegation,
    requests_email_triage, requests_execution, requests_file_distribution, requests_folder_scan,
    requests_image_count_scan, requests_model_identity, requests_obsidian_insights,
    requests_wallet_address_scan,
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
    agent_name: &str,
) -> String {
    let allow_claim = provenance.subagent_task_started
        && provenance.subagent_task_completed
        && provenance.subagent_result_attached;
    if allow_claim || !claims_unverified_subagent_output(&response) {
        return response;
    }
    tracing::warn!("Blocking unverified channel response that claims subagent-produced output");
    format!(
        "{agent_name}: by your command, I will not fake delegation. I can't claim live subagent-produced output unless I actually run a delegated subagent/tool turn in this reply. Ask me to run a concrete delegated task and I'll return that output directly."
    )
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

fn user_requests_fresh_delta(user_prompt: &str) -> bool {
    let lower = user_prompt.to_ascii_lowercase();
    let markers = [
        "status",
        "update",
        "what changed",
        "anything changed",
        "fresh check",
        "check again",
        "still",
        "latest",
        "current",
        "sitrep",
    ];
    markers.iter().any(|m| lower.contains(m))
}

pub(super) fn enforce_non_repetition(
    user_prompt: &str,
    response: String,
    previous_assistant: Option<&str>,
) -> String {
    if previous_assistant.is_some_and(|prev| looks_repetitive(&response, prev)) {
        if user_requests_fresh_delta(user_prompt) {
            return "No verified delta since my last report. Name the exact check you want and I will run it now."
                .to_string();
        }
        return response;
    }
    response
}

pub(super) fn is_low_value_response(user_prompt: &str, response: &str) -> bool {
    if requests_acknowledgement(user_prompt) {
        return false;
    }
    let trimmed = response.trim();
    if trimmed.is_empty() {
        return true;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower == "ready"
        || lower == "on it"
        || lower == "working on that now"
        || lower == "working on that now."
        || lower == "i await your insights"
        || lower == "i await your insights."
    {
        return true;
    }

    // Reject status-only loops that contain no substantive content.
    let noise_markers = [
        "ready",
        "i await your insights",
        "acknowledged. working on that now.",
        "acknowledged. working on that now",
        "⚔️ duncan is on it…",
        "⚔️ duncan is on it...",
        "🤖🧠…",
        "🤖🧠...",
    ];
    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    if !lines.is_empty()
        && lines.iter().all(|line| {
            let low = line.to_ascii_lowercase();
            noise_markers.iter().any(|m| low == *m)
        })
    {
        return true;
    }

    false
}

fn prompt_allows_echo(user_prompt: &str) -> bool {
    let lower = user_prompt.to_ascii_lowercase();
    let markers = [
        "repeat",
        "echo",
        "quote",
        "verbatim",
        "paraphrase",
        "summarize what i said",
        "summarize my message",
    ];
    markers.iter().any(|m| lower.contains(m))
}

pub(super) fn is_parroting_user_prompt(user_prompt: &str, response: &str) -> bool {
    if prompt_allows_echo(user_prompt) {
        return false;
    }
    let u = user_prompt.trim();
    let r = response.trim();
    if u.is_empty() || r.is_empty() {
        return false;
    }
    let u_lower = u.to_ascii_lowercase();
    let r_lower = r.to_ascii_lowercase();
    if r_lower == u_lower {
        return true;
    }

    // If response mostly mirrors prompt tokens and adds little, treat as parroting.
    let ut = repeat_tokens(&u_lower);
    let rt = repeat_tokens(&r_lower);
    if ut.is_empty() || rt.is_empty() {
        return false;
    }
    let overlap = ut.intersection(&rt).count() as f64;
    let overlap_vs_prompt = overlap / ut.len() as f64;
    let prefix_ratio = common_prefix_ratio(&u_lower, &r_lower);
    let length_ratio = (r.len() as f64 / u.len().max(1) as f64).clamp(0.0, 10.0);

    overlap_vs_prompt >= 0.88 && prefix_ratio >= 0.55 && length_ratio <= 1.35
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

fn denies_local_runtime_capability(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    (lower.contains("can't access your files")
        || lower.contains("cannot access your files")
        || lower.contains("can't access your local files")
        || lower.contains("cannot access your local files")
        || lower.contains("can't access your folders")
        || lower.contains("cannot access your folders")
        || lower.contains("can't browse your files")
        || lower.contains("cannot browse your files")
        || lower.contains("can't write directly to your local filesystem")
        || lower.contains("cannot write directly to your local filesystem")
        || lower.contains("i'm not able to directly access")
        || lower.contains("i am not able to directly access"))
        && (lower.contains("folder")
            || lower.contains("filesystem")
            || lower.contains("device")
            || lower.contains("local"))
}

pub(super) fn enforce_execution_truth_guard(
    user_prompt: &str,
    response: String,
    tool_results: &[(String, String)],
    agent_name: &str,
) -> String {
    let runtime_execution_prompt = requests_execution(user_prompt)
        || requests_file_distribution(user_prompt)
        || requests_folder_scan(user_prompt)
        || requests_wallet_address_scan(user_prompt)
        || requests_image_count_scan(user_prompt)
        || requests_obsidian_insights(user_prompt)
        || requests_email_triage(user_prompt);

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
        return format!(
            "{agent_name}: by your command, execution truth is strict. I did not execute a delegated subagent task for that request. I can only claim delegated results when a subagent tool call actually runs."
        );
    }
    if requests_cron(user_prompt)
        && !tool_results.iter().any(|(name, output)| {
            name.to_ascii_lowercase().contains("cron")
                && !output.to_ascii_lowercase().starts_with("error:")
        })
    {
        tracing::warn!("execution truth guard blocked unverified cron claim");
        return format!(
            "{agent_name}: by your command, execution truth is strict. I did not execute a cron scheduling tool for that request. I can only confirm schedules that were actually created or validated by a tool run."
        );
    }

    if !runtime_execution_prompt {
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
        return format!(
            "{agent_name}: by your command, execution truth is strict. I did not execute a tool for that request. I can only claim execution when I actually run a tool and return its output."
        );
    }
    if denies_local_runtime_capability(&response) {
        tracing::warn!("execution truth guard rewrote false local-runtime capability denial");
        return format!(
            "{agent_name}: execution truth is strict. I do have tool/runtime access for local operations, but I did not execute a tool in that turn. Give me the exact path/action and I will run it."
        );
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
        "as an ai text-based interface",
        "as an ai, i can't",
        "as an ai, i cannot",
        "as an ai i can't",
        "as an ai i cannot",
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
        "as an ai text-based interface",
        "as an ai, i can't",
        "as an ai, i cannot",
        "as an ai i can't",
        "as an ai i cannot",
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
        || lower.contains("i cannot provide real-time geopolitical analysis")
        || lower.contains("i can't provide real-time geopolitical analysis")
        || lower.contains("do not include live news feeds")
        || lower.contains("does not include live news feeds")
        || lower.contains("no live news feeds")
        || lower.contains("specialized geopolitical subagents")
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

pub(super) fn is_overbroad_sensitive_conflict_refusal(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    let markers = [
        "i cannot provide quotes related to ongoing conflicts",
        "i can't provide quotes related to ongoing conflicts",
        "i cannot provide quotes",
        "sensitive geopolitical situations",
        "helpful and harmless",
        "avoiding engagement with potentially harmful or biased content",
        "if you have other requests that do not involve sensitive topics",
    ];
    markers.iter().any(|m| lower.contains(m))
}

pub(super) fn enforce_internal_jargon_guard(response: String, agent_name: &str) -> String {
    let mut kept = Vec::new();
    let mut removed = false;
    for line in response.lines() {
        let t = line.trim();
        let lower = t.to_ascii_lowercase();
        let internal = lower.contains("decomposition gate decision")
            || lower.contains("expected_utility_margin")
            || lower.starts_with("centralized delegation is sensible")
            || lower.starts_with("delegation gate decision");
        if internal {
            removed = true;
            continue;
        }
        kept.push(line);
    }
    if !removed {
        return response;
    }
    let cleaned = kept.join("\n").trim().to_string();
    if cleaned.is_empty() {
        return format!(
            "{} here. I’ll keep internals out of the reply and focus on actionable results.",
            agent_name
        );
    }
    cleaned
}

fn is_internal_delegation_metadata_line(line: &str) -> bool {
    let t = line.trim();
    if t.starts_with("delegated_subagent=")
        || t.starts_with("selected_subagent=")
        || t.starts_with("fallback_models=")
        || t.starts_with("notes=")
    {
        return true;
    }
    if let Some(rest) = t.strip_prefix("subtask ") {
        let mut parts = rest.splitn(2, " -> ");
        if let (Some(left), Some(_)) = (parts.next(), parts.next())
            && left.chars().all(|c| c.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

fn is_internal_orchestration_narrative_line(line: &str) -> bool {
    let t = line.trim().to_ascii_lowercase();
    t.starts_with("centralized delegation is sensible")
        || t.starts_with("decomposition gate decision")
        || t.starts_with("expected_utility_margin=")
        || t.starts_with("expected utility margin")
        || t.starts_with("delegation decision:")
        || t.starts_with("rationale:")
        || t.starts_with("subtasks:")
}

fn is_internal_tool_protocol_line(line: &str) -> bool {
    let t = line.trim().to_ascii_lowercase();
    t.contains(r#""tool_call""#)
        || t.starts_with("unexecuted_streaming_tool_call:")
        || t.starts_with("tool_call:")
        || t.starts_with("{\"tool_call\"")
}

pub(super) fn strip_internal_delegation_metadata(content: &str) -> String {
    let filtered = content
        .lines()
        .filter(|line| {
            !is_internal_delegation_metadata_line(line)
                && !is_internal_orchestration_narrative_line(line)
                && !is_internal_tool_protocol_line(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();
    if filtered.is_empty() {
        "I suppressed internal execution metadata. Ask for the user-facing result and I will return it plainly."
            .to_string()
    } else {
        filtered
    }
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
