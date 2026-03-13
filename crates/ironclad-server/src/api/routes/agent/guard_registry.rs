//! Unified post-inference guard registry.
//!
//! Replaces the 12+ inline guard calls in `core.rs` and duplicated subsets
//! in the cached response path with a single [`GuardChain`] that applies a
//! declared set of [`Guard`] implementations uniformly.
//!
//! **Key fixes over the original inline guards:**
//! - [`ExecutionTruthGuard`] removes the L314 tool-results bypass bug where
//!   ANY non-empty tool results caused the entire guard to short-circuit.
//! - [`guard_sets::cached()`] includes `SubagentClaim` and `LiteraryQuoteRetry`
//!   (were missing from the cached response path).
//! - All guards receive classified intents via [`GuardContext`] — no
//!   re-evaluation of keyword detectors.

// Phase 3: guards are now wired into the production pipeline via
// `apply_guards_with_retry()` in core.rs.

use std::collections::HashSet;

use super::decomposition::DelegationProvenance;
use super::intent_registry::Intent;

// ── Guard ID ─────────────────────────────────────────────────────────────

/// Identifies each guard for logging, retry coordination, and diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum GuardId {
    SubagentClaim,
    ExecutionTruth,
    ModelIdentityTruth,
    CurrentEventsTruth,
    LiteraryQuoteRetry,
    PersonalityIntegrity,
    InternalJargon,
    NonRepetition,
    LowValueParroting,
    InternalProtocol,
}

// ── Guard verdict ────────────────────────────────────────────────────────

/// Outcome of a single guard evaluation.
pub(super) enum GuardVerdict {
    /// Content passes this guard unchanged.
    Pass,
    /// Content was rewritten by the guard.
    Rewritten(String),
    /// Guard detected a condition requiring an inference retry.
    /// The pipeline should re-run inference and resume the chain from
    /// `RetrySignal::resume_index`.
    RetryRequested { reason: String },
}

// ── Guard context ────────────────────────────────────────────────────────

/// Shared context threaded through every guard in the chain.
/// Contains classified intents (from `IntentRegistry`) so guards never
/// re-evaluate keyword matchers.
pub(super) struct GuardContext<'a> {
    pub user_prompt: &'a str,
    pub intents: &'a [Intent],
    pub tool_results: &'a [(String, String)],
    pub agent_name: &'a str,
    pub resolved_model: &'a str,
    pub delegation_provenance: &'a DelegationProvenance,
    pub previous_assistant: Option<&'a str>,
}

impl GuardContext<'_> {
    fn has_intent(&self, intent: Intent) -> bool {
        self.intents.contains(&intent)
    }
}

// ── Guard trait ──────────────────────────────────────────────────────────

/// A single post-inference guard that can inspect and optionally rewrite
/// model output.
pub(super) trait Guard: Send + Sync {
    fn id(&self) -> GuardId;

    /// Quick relevance check. If `false`, `evaluate()` is never called.
    fn is_relevant(&self, ctx: &GuardContext) -> bool;

    /// Evaluate the guard on the current content. Returns a verdict.
    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict;
}

// ── Guard chain ──────────────────────────────────────────────────────────

/// Coordinates a retry after a [`GuardVerdict::RetryRequested`].
pub(super) struct RetrySignal {
    pub guard_id: GuardId,
    pub reason: String,
    /// Index in the chain to resume from after the retry completes.
    pub resume_index: usize,
}

/// Result of running the guard chain.
pub(super) struct GuardChainResult {
    /// The (possibly rewritten) content after guard application.
    pub content: String,
    /// If set, the pipeline should perform an inference retry and then
    /// call `apply_from(resume_index)` with the retried content.
    pub retry: Option<RetrySignal>,
}

/// An ordered sequence of guards applied to model output.
pub(super) struct GuardChain {
    guards: Vec<Box<dyn Guard>>,
}

impl GuardChain {
    /// Create an empty guard chain that passes all content through unchanged.
    #[allow(dead_code)]
    pub fn empty() -> Self {
        Self { guards: vec![] }
    }

    /// Apply all guards from the beginning.
    pub fn apply(&self, content: String, ctx: &GuardContext) -> GuardChainResult {
        self.apply_from(content, ctx, 0)
    }

    /// Returns `true` if this chain contains no guards.
    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.guards.is_empty()
    }

    /// Number of guards in the chain.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.guards.len()
    }

    /// Apply guards starting from index `from`. Used to resume after a retry.
    pub fn apply_from(
        &self,
        mut content: String,
        ctx: &GuardContext,
        from: usize,
    ) -> GuardChainResult {
        for (i, guard) in self.guards.iter().enumerate().skip(from) {
            if !guard.is_relevant(ctx) {
                continue;
            }
            match guard.evaluate(&content, ctx) {
                GuardVerdict::Pass => {}
                GuardVerdict::Rewritten(new) => content = new,
                GuardVerdict::RetryRequested { reason } => {
                    return GuardChainResult {
                        content,
                        retry: Some(RetrySignal {
                            guard_id: guard.id(),
                            reason,
                            resume_index: i + 1,
                        }),
                    };
                }
            }
        }
        GuardChainResult {
            content,
            retry: None,
        }
    }
}

// ── Guard implementations ────────────────────────────────────────────────

// 1. SubagentClaimGuard

pub(super) struct SubagentClaimGuard;

impl Guard for SubagentClaimGuard {
    fn id(&self) -> GuardId {
        GuardId::SubagentClaim
    }

    fn is_relevant(&self, _ctx: &GuardContext) -> bool {
        true
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        let prov = ctx.delegation_provenance;
        let allow_claim = prov.subagent_task_started
            && prov.subagent_task_completed
            && prov.subagent_result_attached;
        if allow_claim || !claims_unverified_subagent_output(content) {
            return GuardVerdict::Pass;
        }
        tracing::warn!("guard[SubagentClaim]: blocking unverified subagent output claim");
        GuardVerdict::Rewritten(format!(
            "{}: by your command, I will not fake delegation. I can't claim live subagent-produced \
             output unless I actually run a delegated subagent/tool turn in this reply. Ask me to \
             run a concrete delegated task and I'll return that output directly.",
            ctx.agent_name
        ))
    }
}

// 2. ExecutionTruthGuard (with L314 bug fix)

pub(super) struct ExecutionTruthGuard;

impl Guard for ExecutionTruthGuard {
    fn id(&self) -> GuardId {
        GuardId::ExecutionTruth
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        // Relevant for any intent that implies tool execution or delegation.
        ctx.has_intent(Intent::Execution)
            || ctx.has_intent(Intent::Delegation)
            || ctx.has_intent(Intent::Cron)
            || ctx.has_intent(Intent::FileDistribution)
            || ctx.has_intent(Intent::FolderScan)
            || ctx.has_intent(Intent::WalletAddressScan)
            || ctx.has_intent(Intent::ImageCountScan)
            || ctx.has_intent(Intent::MarkdownCountScan)
            || ctx.has_intent(Intent::ObsidianInsights)
            || ctx.has_intent(Intent::EmailTriage)
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        // Delegation claim verification.
        if ctx.has_intent(Intent::Delegation)
            && !ctx.tool_results.iter().any(|(name, output)| {
                let n = name.to_ascii_lowercase();
                let is_delegate_tool = n.contains("subagent")
                    || n.contains("delegate")
                    || n.contains("assign")
                    || n.contains("orchestrate");
                let succeeded = !output.to_ascii_lowercase().starts_with("error:");
                is_delegate_tool && succeeded
            })
        {
            tracing::warn!("guard[ExecutionTruth]: blocked unverified delegation claim");
            return GuardVerdict::Rewritten(format!(
                "{}: by your command, execution truth is strict. I did not execute a delegated \
                 subagent task for that request. I can only claim delegated results when a \
                 subagent tool call actually runs.",
                ctx.agent_name
            ));
        }

        // Cron claim verification.
        if ctx.has_intent(Intent::Cron)
            && !ctx.tool_results.iter().any(|(name, output)| {
                name.to_ascii_lowercase().contains("cron")
                    && !output.to_ascii_lowercase().starts_with("error:")
            })
        {
            tracing::warn!("guard[ExecutionTruth]: blocked unverified cron claim");
            return GuardVerdict::Rewritten(format!(
                "{}: by your command, execution truth is strict. I did not execute a cron \
                 scheduling tool for that request. I can only confirm schedules that were \
                 actually created or validated by a tool run.",
                ctx.agent_name
            ));
        }

        // Runtime execution prompt verification.
        let runtime_execution = ctx.has_intent(Intent::Execution)
            || ctx.has_intent(Intent::FileDistribution)
            || ctx.has_intent(Intent::FolderScan)
            || ctx.has_intent(Intent::WalletAddressScan)
            || ctx.has_intent(Intent::ImageCountScan)
            || ctx.has_intent(Intent::ObsidianInsights)
            || ctx.has_intent(Intent::EmailTriage);

        if !runtime_execution {
            return GuardVerdict::Pass;
        }

        // FIX (was L314 bug): When tool_results is non-empty, we still check
        // for false denial of local capability. The old code short-circuited
        // entirely, allowing "can't access your files" responses even when
        // tools actually ran.
        if !ctx.tool_results.is_empty() {
            if denies_local_runtime_capability(content) {
                tracing::warn!(
                    "guard[ExecutionTruth]: rewrote capability denial despite tool execution"
                );
                return GuardVerdict::Rewritten(format!(
                    "{}: execution truth is strict. I do have tool/runtime access for local \
                     operations, but I did not execute a tool in that turn. Give me the exact \
                     path/action and I will run it.",
                    ctx.agent_name
                ));
            }
            return GuardVerdict::Pass;
        }

        // No tools ran — check for false execution claims.
        let lower = content.to_ascii_lowercase();
        if lower.contains("encountered an error reaching all llm providers") {
            return GuardVerdict::Pass;
        }

        if looks_like_unexecuted_claim(content)
            || lower.contains("tool successfully executed")
            || lower.contains("the `")
            || lower.starts_with('{')
        {
            tracing::warn!("guard[ExecutionTruth]: rewrote unverified execution claim");
            return GuardVerdict::Rewritten(format!(
                "{}: by your command, execution truth is strict. I did not execute a tool for \
                 that request. I can only claim execution when I actually run a tool and return \
                 its output.",
                ctx.agent_name
            ));
        }

        if denies_local_runtime_capability(content) {
            tracing::warn!("guard[ExecutionTruth]: rewrote false capability denial");
            return GuardVerdict::Rewritten(format!(
                "{}: execution truth is strict. I do have tool/runtime access for local \
                 operations, but I did not execute a tool in that turn. Give me the exact \
                 path/action and I will run it.",
                ctx.agent_name
            ));
        }

        GuardVerdict::Pass
    }
}

// 3. ModelIdentityTruthGuard

pub(super) struct ModelIdentityTruthGuard;

impl Guard for ModelIdentityTruthGuard {
    fn id(&self) -> GuardId {
        GuardId::ModelIdentityTruth
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        ctx.has_intent(Intent::ModelIdentity)
    }

    fn evaluate(&self, _content: &str, ctx: &GuardContext) -> GuardVerdict {
        tracing::warn!(
            model = ctx.resolved_model,
            "guard[ModelIdentityTruth]: emitting canonical model identity"
        );
        GuardVerdict::Rewritten(format!(
            "{} reporting in. I am currently running on {}.",
            ctx.agent_name, ctx.resolved_model
        ))
    }
}

// 4. CurrentEventsTruthGuard

pub(super) struct CurrentEventsTruthGuard;

impl Guard for CurrentEventsTruthGuard {
    fn id(&self) -> GuardId {
        GuardId::CurrentEventsTruth
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        ctx.has_intent(Intent::CurrentEvents)
    }

    fn evaluate(&self, content: &str, _ctx: &GuardContext) -> GuardVerdict {
        if !looks_like_stale_knowledge_disclaimer(content) {
            return GuardVerdict::Pass;
        }
        tracing::warn!("guard[CurrentEventsTruth]: blocked stale-knowledge disclaimer");
        GuardVerdict::Rewritten(
            "Acknowledged. I cannot provide a current-events sitrep from stale memory. I will \
             run live retrieval/delegation and return an up-to-date report with the current date."
                .into(),
        )
    }
}

// 5. LiteraryQuoteRetryGuard

pub(super) struct LiteraryQuoteRetryGuard;

impl Guard for LiteraryQuoteRetryGuard {
    fn id(&self) -> GuardId {
        GuardId::LiteraryQuoteRetry
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        ctx.has_intent(Intent::LiteraryQuoteContext)
    }

    fn evaluate(&self, content: &str, _ctx: &GuardContext) -> GuardVerdict {
        if is_overbroad_sensitive_conflict_refusal(content) {
            tracing::warn!(
                "guard[LiteraryQuoteRetry]: overbroad refusal detected; requesting retry"
            );
            GuardVerdict::RetryRequested {
                reason: "overbroad sensitive-topic refusal for literary quote request".into(),
            }
        } else {
            GuardVerdict::Pass
        }
    }
}

// 6. PersonalityIntegrityGuard

pub(super) struct PersonalityIntegrityGuard;

impl Guard for PersonalityIntegrityGuard {
    fn id(&self) -> GuardId {
        GuardId::PersonalityIntegrity
    }

    fn is_relevant(&self, _ctx: &GuardContext) -> bool {
        true
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        if !contains_foreign_identity_boilerplate(content) {
            return GuardVerdict::Pass;
        }
        tracing::warn!("guard[PersonalityIntegrity]: stripped foreign identity boilerplate");
        let cleaned = filter_foreign_identity_sentences(content);
        if !cleaned.is_empty() {
            return GuardVerdict::Rewritten(cleaned);
        }

        // Empty after filtering — provide intent-appropriate fallback.
        let lower_prompt = ctx.user_prompt.to_ascii_lowercase();
        let asks_release_summary = lower_prompt.contains("release")
            || lower_prompt.contains("changelog")
            || lower_prompt.contains("linkedin")
            || lower_prompt.contains("x.com")
            || lower_prompt.contains("twitter")
            || lower_prompt.contains("v0.9.5")
            || lower_prompt.contains("0.9.5");
        if asks_release_summary {
            return GuardVerdict::Rewritten(
                "I need concrete Ironclad 0.9.5 context to summarize accurately. I can pull \
                 from changelog/roadmap memory if available, or you can provide release notes \
                 and I'll format them for operator, LinkedIn, and X."
                    .into(),
            );
        }
        if ctx.has_intent(Intent::ModelIdentity) {
            return GuardVerdict::Rewritten(format!(
                "I am {} and I am currently running on {}.",
                ctx.agent_name, ctx.resolved_model
            ));
        }
        GuardVerdict::Rewritten(format!(
            "I'm {}. I'll continue in my configured voice and avoid foreign boilerplate.",
            ctx.agent_name
        ))
    }
}

// 7. InternalJargonGuard

pub(super) struct InternalJargonGuard;

impl Guard for InternalJargonGuard {
    fn id(&self) -> GuardId {
        GuardId::InternalJargon
    }

    fn is_relevant(&self, _ctx: &GuardContext) -> bool {
        true
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        let mut kept = Vec::new();
        let mut removed = false;
        for line in content.lines() {
            let lower = line.trim().to_ascii_lowercase();
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
            return GuardVerdict::Pass;
        }
        let cleaned = kept.join("\n").trim().to_string();
        if cleaned.is_empty() {
            return GuardVerdict::Rewritten(format!(
                "{} here. I'll keep internals out of the reply and focus on actionable results.",
                ctx.agent_name
            ));
        }
        GuardVerdict::Rewritten(cleaned)
    }
}

// 8. NonRepetitionGuard

pub(super) struct NonRepetitionGuard;

impl Guard for NonRepetitionGuard {
    fn id(&self) -> GuardId {
        GuardId::NonRepetition
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        ctx.previous_assistant.is_some()
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        let Some(prev) = ctx.previous_assistant else {
            return GuardVerdict::Pass;
        };
        if !looks_repetitive(content, prev) {
            return GuardVerdict::Pass;
        }
        if user_requests_fresh_delta(ctx.user_prompt) {
            return GuardVerdict::Rewritten(
                "No verified delta since my last report. Name the exact check you want and I \
                 will run it now."
                    .into(),
            );
        }
        // Not a fresh-delta request — allow the repetition through.
        GuardVerdict::Pass
    }
}

// 9. LowValueParrotingGuard

pub(super) struct LowValueParrotingGuard;

impl Guard for LowValueParrotingGuard {
    fn id(&self) -> GuardId {
        GuardId::LowValueParroting
    }

    fn is_relevant(&self, ctx: &GuardContext) -> bool {
        // Only flag when no tools ran and the prompt doesn't request execution.
        ctx.tool_results.is_empty() && !ctx.has_intent(Intent::Execution)
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        if is_low_value_response(content, ctx.intents)
            || is_parroting_user_prompt(ctx.user_prompt, content)
        {
            tracing::warn!("guard[LowValueParroting]: low-value or parroting response detected");
            GuardVerdict::RetryRequested {
                reason: "low-value placeholder or parroting response".into(),
            }
        } else {
            GuardVerdict::Pass
        }
    }
}

// 10. InternalProtocolGuard

pub(super) struct InternalProtocolGuard;

impl Guard for InternalProtocolGuard {
    fn id(&self) -> GuardId {
        GuardId::InternalProtocol
    }

    fn is_relevant(&self, _ctx: &GuardContext) -> bool {
        true
    }

    fn evaluate(&self, content: &str, ctx: &GuardContext) -> GuardVerdict {
        let lower = content.to_ascii_lowercase();
        if !lower.contains("\"tool_call\"")
            && !lower.contains("unexecuted_streaming_tool_call")
            && !lower.contains("delegated_subagent=")
            && !lower.contains("selected_subagent=")
            && !lower.contains("subtask ")
        {
            return GuardVerdict::Pass;
        }

        let stripped = strip_internal_protocol_metadata(content);
        if stripped.is_empty() {
            return GuardVerdict::Rewritten(format!(
                "{} here. I filtered internal execution protocol and will continue with \
                 user-facing output only.",
                ctx.agent_name
            ));
        }
        GuardVerdict::Rewritten(stripped)
    }
}

// ── Guard presets ────────────────────────────────────────────────────────

pub(super) mod guard_sets {
    use super::*;

    /// Full guard chain applied after live inference with ReAct loop.
    /// Order matches the original core.rs L2024-2147 chain.
    pub fn full() -> GuardChain {
        GuardChain {
            guards: vec![
                Box::new(SubagentClaimGuard),
                Box::new(ExecutionTruthGuard),
                Box::new(ModelIdentityTruthGuard),
                Box::new(CurrentEventsTruthGuard),
                Box::new(LiteraryQuoteRetryGuard),
                Box::new(PersonalityIntegrityGuard),
                Box::new(InternalJargonGuard),
                Box::new(NonRepetitionGuard),
                Box::new(LowValueParrotingGuard),
                Box::new(InternalProtocolGuard),
            ],
        }
    }

    /// Guard chain for cached responses.
    /// **Fixes**: includes SubagentClaim and LiteraryQuoteRetry which were
    /// missing from the original cached path at core.rs L2337-2370.
    pub fn cached() -> GuardChain {
        GuardChain {
            guards: vec![
                Box::new(SubagentClaimGuard), // was missing
                Box::new(ExecutionTruthGuard),
                Box::new(ModelIdentityTruthGuard),
                Box::new(CurrentEventsTruthGuard),
                Box::new(LiteraryQuoteRetryGuard), // was missing
                Box::new(PersonalityIntegrityGuard),
                Box::new(InternalJargonGuard),
                Box::new(InternalProtocolGuard),
                Box::new(NonRepetitionGuard),
                Box::new(LowValueParrotingGuard),
            ],
        }
    }

    /// Reduced guard chain for SSE streaming responses where retries are
    /// impractical (content is already partially delivered).
    #[allow(dead_code)]
    pub fn streaming() -> GuardChain {
        GuardChain {
            guards: vec![
                Box::new(SubagentClaimGuard),
                Box::new(CurrentEventsTruthGuard),
                Box::new(PersonalityIntegrityGuard),
                Box::new(InternalJargonGuard),
                Box::new(NonRepetitionGuard),
                Box::new(InternalProtocolGuard),
            ],
        }
    }
}

// ── Helper functions (migrated from guards.rs) ───────────────────────────

fn claims_unverified_subagent_output(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
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
    MARKERS.iter().any(|m| lower.contains(m))
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

fn is_overbroad_sensitive_conflict_refusal(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "i cannot provide quotes related to ongoing conflicts",
        "i can't provide quotes related to ongoing conflicts",
        "i cannot provide quotes",
        "sensitive geopolitical situations",
        "helpful and harmless",
        "avoiding engagement with potentially harmful or biased content",
        "if you have other requests that do not involve sensitive topics",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

const FOREIGN_IDENTITY_MARKERS: &[&str] = &[
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

fn contains_foreign_identity_boilerplate(response: &str) -> bool {
    let lower = response.to_ascii_lowercase();
    FOREIGN_IDENTITY_MARKERS.iter().any(|m| lower.contains(m))
}

fn filter_foreign_identity_sentences(response: &str) -> String {
    let mut out = String::new();
    for chunk in response.split_inclusive(['\n', '.', '!', '?']) {
        let lower = chunk.to_ascii_lowercase();
        if FOREIGN_IDENTITY_MARKERS.iter().any(|m| lower.contains(m)) {
            continue;
        }
        out.push_str(chunk);
    }
    out.trim().to_string()
}

fn user_requests_fresh_delta(user_prompt: &str) -> bool {
    let lower = user_prompt.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
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
    MARKERS.iter().any(|m| lower.contains(m))
}

fn repeat_tokens(text: &str) -> HashSet<String> {
    text.to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|tok| tok.len() >= 3)
        .map(|tok| tok.to_string())
        .collect()
}

fn common_prefix_ratio(a: &str, b: &str) -> f64 {
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

fn looks_repetitive(current: &str, previous: &str) -> bool {
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

/// Intent-aware version of `is_low_value_response()`.
/// Uses classified intents instead of re-evaluating `requests_acknowledgement()`.
fn is_low_value_response(response: &str, intents: &[Intent]) -> bool {
    if intents.contains(&Intent::Acknowledgement) {
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

    const NOISE_MARKERS: &[&str] = &[
        "ready",
        "i await your insights",
        "acknowledged. working on that now.",
        "acknowledged. working on that now",
    ];
    let lines = trimmed
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>();
    if !lines.is_empty()
        && lines.iter().all(|line| {
            let low = line.to_ascii_lowercase();
            NOISE_MARKERS.iter().any(|m| low == *m)
        })
    {
        return true;
    }

    false
}

fn prompt_allows_echo(user_prompt: &str) -> bool {
    let lower = user_prompt.to_ascii_lowercase();
    const MARKERS: &[&str] = &[
        "repeat",
        "echo",
        "quote",
        "verbatim",
        "paraphrase",
        "summarize what i said",
        "summarize my message",
    ];
    MARKERS.iter().any(|m| lower.contains(m))
}

fn is_parroting_user_prompt(user_prompt: &str, response: &str) -> bool {
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

fn strip_internal_protocol_metadata(content: &str) -> String {
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

// ── Tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn default_provenance() -> DelegationProvenance {
        DelegationProvenance::default()
    }

    fn ctx<'a>(
        prompt: &'a str,
        intents: &'a [Intent],
        tool_results: &'a [(String, String)],
        provenance: &'a DelegationProvenance,
    ) -> GuardContext<'a> {
        GuardContext {
            user_prompt: prompt,
            intents,
            tool_results,
            agent_name: "TestAgent",
            resolved_model: "test-model",
            delegation_provenance: provenance,
            previous_assistant: None,
        }
    }

    // -- SubagentClaimGuard --

    #[test]
    fn subagent_claim_passes_when_provenance_valid() {
        let prov = DelegationProvenance {
            subagent_task_started: true,
            subagent_task_completed: true,
            subagent_result_attached: true,
        };
        let guard = SubagentClaimGuard;
        let ctx = ctx("test", &[], &[], &prov);
        let verdict = guard.evaluate("subagent-generated sitrep: all clear", &ctx);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    #[test]
    fn subagent_claim_blocks_without_provenance() {
        let prov = default_provenance();
        let guard = SubagentClaimGuard;
        let ctx = ctx("test", &[], &[], &prov);
        let verdict = guard.evaluate("subagent-generated sitrep: all clear", &ctx);
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    #[test]
    fn subagent_claim_passes_when_no_claim() {
        let prov = default_provenance();
        let guard = SubagentClaimGuard;
        let ctx = ctx("test", &[], &[], &prov);
        let verdict = guard.evaluate("Here is a normal response.", &ctx);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    // -- ExecutionTruthGuard --

    #[test]
    fn execution_truth_blocks_false_delegation_claim() {
        let prov = default_provenance();
        let intents = [Intent::Delegation];
        let guard = ExecutionTruthGuard;
        let ctx = ctx("delegate to a subagent", &intents, &[], &prov);
        let verdict = guard.evaluate("I delegated the task successfully.", &ctx);
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    #[test]
    fn execution_truth_passes_with_real_delegation_tool() {
        let prov = default_provenance();
        let intents = [Intent::Delegation];
        let tools = vec![("delegate-subagent".into(), "success".into())];
        let guard = ExecutionTruthGuard;
        let ctx = ctx("delegate to a subagent", &intents, &tools, &prov);
        let verdict = guard.evaluate("I delegated the task successfully.", &ctx);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    #[test]
    fn execution_truth_blocks_unexecuted_claim_no_tools() {
        let prov = default_provenance();
        let intents = [Intent::Execution];
        let guard = ExecutionTruthGuard;
        let ctx = ctx("run the scanner", &intents, &[], &prov);
        let verdict = guard.evaluate("tool successfully executed and returned results", &ctx);
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    #[test]
    fn execution_truth_fix_blocks_capability_denial_despite_tools() {
        // This test verifies the L314 bug fix: even with non-empty tool_results,
        // a response that denies local runtime capability should be caught.
        let prov = default_provenance();
        let intents = [Intent::FolderScan];
        let tools = vec![("list-files".into(), "file1.txt\nfile2.txt".into())];
        let guard = ExecutionTruthGuard;
        let ctx = ctx("scan my ~/Downloads folder", &intents, &tools, &prov);
        let verdict = guard.evaluate(
            "I cannot access your local files or folders on your device.",
            &ctx,
        );
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    #[test]
    fn execution_truth_passes_normal_response_with_tools() {
        let prov = default_provenance();
        let intents = [Intent::Execution];
        let tools = vec![("shell".into(), "hello world".into())];
        let guard = ExecutionTruthGuard;
        let ctx = ctx("run echo hello", &intents, &tools, &prov);
        let verdict = guard.evaluate("The command returned: hello world", &ctx);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    // -- ModelIdentityTruthGuard --

    #[test]
    fn model_identity_emits_canonical_response() {
        let prov = default_provenance();
        let intents = [Intent::ModelIdentity];
        let guard = ModelIdentityTruthGuard;
        let ctx = ctx("/status", &intents, &[], &prov);
        let verdict = guard.evaluate("I am a helpful assistant.", &ctx);
        match verdict {
            GuardVerdict::Rewritten(msg) => {
                assert!(msg.contains("TestAgent"));
                assert!(msg.contains("test-model"));
            }
            _ => panic!("expected Rewritten"),
        }
    }

    // -- CurrentEventsTruthGuard --

    #[test]
    fn current_events_blocks_stale_disclaimer() {
        let prov = default_provenance();
        let intents = [Intent::CurrentEvents];
        let guard = CurrentEventsTruthGuard;
        let ctx = ctx("geopolitical sitrep", &intents, &[], &prov);
        let verdict = guard.evaluate(
            "As of my last update in 2023, I cannot provide real-time updates.",
            &ctx,
        );
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    #[test]
    fn current_events_passes_live_content() {
        let prov = default_provenance();
        let intents = [Intent::CurrentEvents];
        let guard = CurrentEventsTruthGuard;
        let ctx = ctx("geopolitical sitrep", &intents, &[], &prov);
        let verdict = guard.evaluate("Here is the latest sitrep from live data.", &ctx);
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    // -- LiteraryQuoteRetryGuard --

    #[test]
    fn literary_quote_requests_retry_on_overbroad_refusal() {
        let prov = default_provenance();
        let intents = [Intent::LiteraryQuoteContext];
        let guard = LiteraryQuoteRetryGuard;
        let ctx = ctx("dune quote for iran conflict", &intents, &[], &prov);
        let verdict = guard.evaluate(
            "I cannot provide quotes related to ongoing conflicts due to sensitive geopolitical situations.",
            &ctx,
        );
        assert!(matches!(verdict, GuardVerdict::RetryRequested { .. }));
    }

    // -- PersonalityIntegrityGuard --

    #[test]
    fn personality_strips_foreign_identity() {
        let prov = default_provenance();
        let guard = PersonalityIntegrityGuard;
        let ctx = ctx("hello", &[], &[], &prov);
        let verdict = guard.evaluate(
            "As an AI developed by Microsoft, I can help you. Here is your answer.",
            &ctx,
        );
        match verdict {
            GuardVerdict::Rewritten(msg) => {
                assert!(!msg.to_ascii_lowercase().contains("microsoft"));
                assert!(msg.contains("answer"));
            }
            _ => panic!("expected Rewritten"),
        }
    }

    // -- InternalJargonGuard --

    #[test]
    fn jargon_strips_decomposition_lines() {
        let prov = default_provenance();
        let guard = InternalJargonGuard;
        let ctx = ctx("hello", &[], &[], &prov);
        let verdict = guard.evaluate(
            "Decomposition gate decision: centralized\nHere is your answer.",
            &ctx,
        );
        match verdict {
            GuardVerdict::Rewritten(msg) => {
                assert!(!msg.contains("Decomposition"));
                assert!(msg.contains("answer"));
            }
            _ => panic!("expected Rewritten"),
        }
    }

    // -- NonRepetitionGuard --

    #[test]
    fn non_repetition_detects_exact_repeat() {
        let prov = default_provenance();
        let mut gctx = ctx("give me a status update", &[], &[], &prov);
        let long = "a]".repeat(50); // >80 chars
        gctx.previous_assistant = Some(&long);
        let guard = NonRepetitionGuard;
        let verdict = guard.evaluate(&long, &gctx);
        assert!(matches!(verdict, GuardVerdict::Rewritten(_)));
    }

    // -- LowValueParrotingGuard --

    #[test]
    fn low_value_detects_placeholder() {
        let prov = default_provenance();
        let guard = LowValueParrotingGuard;
        let ctx = ctx("tell me about X", &[], &[], &prov);
        let verdict = guard.evaluate("Ready", &ctx);
        assert!(matches!(verdict, GuardVerdict::RetryRequested { .. }));
    }

    #[test]
    fn low_value_passes_acknowledgement_intent() {
        let prov = default_provenance();
        let intents = [Intent::Acknowledgement];
        let guard = LowValueParrotingGuard;
        // Note: LowValueParrotingGuard is not relevant when Execution is present,
        // but Acknowledgement doesn't affect relevance — it affects the helper.
        let ctx = ctx(
            "acknowledge in one sentence then wait",
            &intents,
            &[],
            &prov,
        );
        let verdict = guard.evaluate("Ready", &ctx);
        // With Acknowledgement intent, is_low_value_response returns false.
        assert!(matches!(verdict, GuardVerdict::Pass));
    }

    // -- InternalProtocolGuard --

    #[test]
    fn protocol_strips_tool_call_json() {
        let prov = default_provenance();
        let guard = InternalProtocolGuard;
        let ctx = ctx("do something", &[], &[], &prov);
        let verdict = guard.evaluate(
            "{\"tool_call\": {\"name\": \"shell\"}}\nActual response here.",
            &ctx,
        );
        match verdict {
            GuardVerdict::Rewritten(msg) => {
                assert!(!msg.contains("tool_call"));
                assert!(msg.contains("Actual response"));
            }
            _ => panic!("expected Rewritten"),
        }
    }

    // -- GuardChain integration --

    #[test]
    fn full_chain_applies_all_relevant_guards() {
        let chain = guard_sets::full();
        assert_eq!(chain.len(), 10);
    }

    #[test]
    fn cached_chain_includes_previously_missing_guards() {
        let chain = guard_sets::cached();
        let ids: Vec<GuardId> = chain.guards.iter().map(|g| g.id()).collect();
        assert!(
            ids.contains(&GuardId::SubagentClaim),
            "cached must include SubagentClaim"
        );
        assert!(
            ids.contains(&GuardId::LiteraryQuoteRetry),
            "cached must include LiteraryQuoteRetry"
        );
    }

    #[test]
    fn chain_stops_on_retry_requested() {
        let chain = guard_sets::full();
        let prov = default_provenance();
        let intents = [Intent::LiteraryQuoteContext];
        let ctx = ctx("dune quote for iran conflict", &intents, &[], &prov);
        let result = chain.apply(
            "I cannot provide quotes related to ongoing conflicts.".into(),
            &ctx,
        );
        assert!(result.retry.is_some());
        let retry = result.retry.unwrap();
        assert_eq!(retry.guard_id, GuardId::LiteraryQuoteRetry);
        assert!(retry.resume_index > 0);
    }

    #[test]
    fn chain_resume_from_continues_remaining_guards() {
        let chain = guard_sets::full();
        let prov = default_provenance();
        let intents = [Intent::LiteraryQuoteContext];
        let ctx = ctx("dune quote for iran conflict", &intents, &[], &prov);
        // Simulate a retry that produced clean content.
        let result = chain.apply_from(
            "Fear is the mind-killer. In this context, resist panic.".into(),
            &ctx,
            5, // resume after LiteraryQuoteRetry (index 4)
        );
        assert!(result.retry.is_none());
        // Content should pass remaining guards unchanged.
        assert!(result.content.contains("Fear is the mind-killer"));
    }
}
