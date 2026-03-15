//! Dedup guard, response quality filters, scope validation, and current-events truth guard.

use std::collections::HashSet;
use std::sync::Arc;

use super::intents::{requests_acknowledgement, requests_current_events};

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
        let fp = std::mem::take(&mut self.fingerprint);
        // Try synchronous release first (avoids race with sequential callers).
        // Falls back to tokio::spawn if the lock is contended.
        if let Ok(mut llm) = self.llm.try_write() {
            llm.dedup.release(&fp);
        } else {
            let llm = Arc::clone(&self.llm);
            tokio::spawn(async move {
                let mut llm = llm.write().await;
                llm.dedup.release(&fp);
            });
        }
    }
}

// ── Token analysis utilities ─────────────────────────────────

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

// ── Response quality filters ─────────────────────────────────

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
        "\u{2694}\u{FE0F} duncan is on it\u{2026}",
        "\u{2694}\u{FE0F} duncan is on it...",
        "\u{1F916}\u{1F9E0}\u{2026}",
        "\u{1F916}\u{1F9E0}...",
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

// ── Current-events truth guard ───────────────────────────────

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

// ── Internal metadata stripping ──────────────────────────────

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
    content
        .lines()
        .filter(|line| {
            !is_internal_delegation_metadata_line(line)
                && !is_internal_orchestration_narrative_line(line)
                && !is_internal_tool_protocol_line(line)
        })
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

// ── Scope validation ─────────────────────────────────────────

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
