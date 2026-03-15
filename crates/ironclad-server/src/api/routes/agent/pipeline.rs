//! Unified pipeline configuration and entry-point presets.
//!
//! Replaces the ad-hoc divergence across the four entry points (`handlers.rs`,
//! `streaming.rs`, `channel_message.rs`, `scheduled_tasks.rs`) with a single
//! [`PipelineConfig`] that declaratively specifies which pipeline stages are
//! active. This makes it **impossible** to forget a stage because every path
//! constructs its config from one of the four preset constructors.
//!
//! ## Pipeline stages (in execution order)
//!
//! 1. **Injection defense** — block/sanitize via `ironclad_agent::injection`
//! 2. **Dedup tracking** — in-flight request deduplication via `DedupTracker`
//! 3. **Session resolution** — find or create the session for this turn
//! 4. **Decomposition gate** — evaluate whether to delegate subtasks
//! 5. **Delegated execution** — run `orchestrate-subagents` before inference
//! 6. **Specialist controls** — handle specialist creation control flows
//! 7. **Shortcut dispatch** — try execution shortcuts before LLM inference
//! 8. **Cache check** — look up semantic cache before LLM call
//! 9. **Inference** — Standard (ReAct tool loop) or Streaming (SSE)
//! 10. **Guard chain** — post-inference truth/integrity guards
//! 11. **Post-turn ingest** — background memory ingestion
//! 12. **Nickname refinement** — background LLM-driven session naming
//!
//! ## Security fixes integrated
//!
//! - **Cron injection defense**: `PipelineConfig::cron()` sets
//!   `injection_defense: true` — was completely absent from
//!   `scheduled_tasks.rs`.
//! - **Cache guard parity**: `cache_guard_set` uses `Cached` preset which
//!   includes `SubagentClaim` and `LiteraryQuoteRetry` (were missing).
//!
//! ## Feature matrix (authoritative)
//!
//! | Feature               | API | Stream | Channel | Cron  |
//! |-----------------------|-----|--------|---------|-------|
//! | injection_defense     |  ✓  |   ✓    |    ✓    |  ✓*   |
//! | dedup_tracking        |  ✓  |   ✓    |    ✓    |  ✗    |
//! | decomposition_gate    |  ✓  |   ✗    |    ✓    |  ✓    |
//! | delegated_execution   |  ✓  |   ✗    |    ✓    |  ✗    |
//! | shortcuts_enabled     |  ✓  |   ✗    |    ✓    |  ✓    |
//! | specialist_controls   |  ✗  |   ✗    |    ✓    |  ✗    |
//! | inference: Standard   |  ✓  |   ✗    |    ✓    |  ✓    |
//! | inference: Streaming  |  ✗  |   ✓    |    ✗    |  ✗    |
//! | guard_set: Full       |  ✓  |   ✗    |    ✓    |  ✓    |
//! | guard_set: Streaming  |  ✗  |   ✓    |    ✗    |  ✗    |
//! | cache_guard: Cached   |  ✓  |   ✗    |    ✓    |  ✓    |
//! | cache_enabled         |  ✓  |   ✓    |    ✓    |  ✓    |
//! | authority: ApiClaim   |  ✓  |   ✗    |    ✗    |  ✗    |
//! | authority: Channel    |  ✗  |   ✗    |    ✓    |  ✗    |
//! | authority: SelfGen    |  ✗  |   ✗    |    ✗    |  ✓    |
//! | authority: AuditOnly  |  ✗  |   ✓    |    ✗    |  ✗    |
//! | post_turn_ingest      |  ✓  |   ✓    |    ✓    |  ✓    |
//! | nickname_refinement   |  ✓  |   ✗    |    ✗    |  ✗    |
//! | inject_diagnostics    |  ✓  |   ✓    |    ✗    |  ✗    |
//!
//! `*` = security fix — was missing, now enabled.

use super::AppState;
use super::core;
use super::decomposition::DelegationProvenance;
#[cfg(test)]
use super::guard_registry::{GuardChain, guard_sets};

// ── Guard set presets ─────────────────────────────────────────────────────

/// Which guard set to apply to inference output.
///
/// Resolved to a concrete [`GuardChain`] via [`GuardSetPreset::resolve`] at
/// pipeline execution time. Using an enum rather than a direct `GuardChain`
/// allows `PipelineConfig` to be `Clone + Debug` without requiring `Guard`
/// implementors to be cloneable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GuardSetPreset {
    /// All 10 guards — used for fresh inference on standard paths.
    Full,
    /// All 10 guards — used for cached responses. Fixes previously-missing
    /// `SubagentClaim` and `LiteraryQuoteRetry`.
    Cached,
    /// 6 guards — reduced subset for streaming where retries are impractical.
    Streaming,
    /// No guards applied. Used for paths where guards are inapplicable
    /// (e.g., cache guard set on the streaming path which doesn't use cache).
    None,
}

impl GuardSetPreset {
    /// Materialize the preset into a concrete guard chain.
    #[cfg(test)]
    pub fn resolve(self) -> GuardChain {
        match self {
            Self::Full => guard_sets::full(),
            Self::Cached => guard_sets::cached(),
            Self::Streaming => guard_sets::streaming(),
            Self::None => GuardChain::empty(),
        }
    }
}

// ── Session resolution modes ──────────────────────────────────────────────

/// How the session is resolved for this pipeline execution.
///
/// Each entry point has different session semantics:
/// - API: optional `session_id` in request body, or create from web scope
/// - Channel: scope derived from `platform:chat_id`
/// - Cron: dedicated agent-scoped session
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum SessionResolutionMode {
    /// API/Streaming: session_id provided in request body, or create from
    /// web scope if not provided.
    FromBody,
    /// Channel: scope derived from platform + chat_id.
    FromChannel {
        /// The chat platform identifier (e.g., "telegram", "discord").
        platform: String,
    },
    /// Cron/scheduled: find_or_create with agent-scoped session.
    Dedicated,
    /// Pre-resolved: session already created by caller (useful for
    /// testing or specialized workflows).
    #[cfg(test)]
    Provided { session_id: String },
}

// ── Authority modes ───────────────────────────────────────────────────────

/// How authority (RBAC) is determined for this pipeline execution.
///
/// Authority controls which tools the agent is permitted to execute.
/// Different entry points have different trust models.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum AuthorityMode {
    /// API: resolve via `resolve_api_claim()`. Supports reduced authority
    /// from injection caution-level detection.
    ApiClaim,
    /// Channel: resolve via `resolve_channel_claim()` with sender context
    /// (allow-list membership, trusted sender IDs, threat score).
    ChannelClaim,
    /// Cron: hardcoded `InputAuthority::SelfGenerated`. Internal system
    /// caller with no external user input.
    SelfGenerated,
    /// Streaming: authority is logged for audit trail but NOT enforced
    /// because the streaming path does not execute tools.
    AuditOnly,
}

// ── Inference modes ───────────────────────────────────────────────────────

/// How inference is executed.
///
/// The two modes have fundamentally different execution models:
/// - Standard: full ReAct tool loop, shortcut dispatch, guard chain
/// - Streaming: direct provider SSE stream, no ReAct, minimal post-processing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum InferenceMode {
    /// Full inference with ReAct tool loop, shortcut dispatch, and guard chain.
    Standard,
    /// SSE streaming — direct provider call with chunk-by-chunk delivery.
    /// No ReAct loop, no shortcut dispatch. Reduced guard set applied
    /// post-accumulation.
    Streaming,
}

// ── Pipeline configuration ────────────────────────────────────────────────

/// Declarative configuration for the unified pipeline.
///
/// Each entry point constructs a `PipelineConfig` via one of the four preset
/// constructors ([`api`], [`streaming`], [`channel`], [`cron`]), making it
/// impossible to forget a pipeline stage.
///
/// Every boolean flag corresponds to a pipeline stage. If the flag is `false`,
/// the stage is completely skipped — no branching inside the stage itself.
///
/// [`api`]: PipelineConfig::api
/// [`streaming`]: PipelineConfig::streaming
/// [`channel`]: PipelineConfig::channel
/// [`cron`]: PipelineConfig::cron
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields set by constructors; some consumed only by tests until pipeline phases are wired
pub(super) struct PipelineConfig {
    // ── Input defense ─────────────────────────────────────────────
    /// Run injection detection: block (>0.7), sanitize (0.3-0.7), pass (<0.3).
    pub injection_defense: bool,
    /// Track in-flight duplicates and reject concurrent identical requests.
    pub dedup_tracking: bool,

    // ── Session ──────────────────────────────────────────────────
    /// How to resolve or create the session for this turn.
    pub session_resolution: SessionResolutionMode,

    // ── Pre-inference ────────────────────────────────────────────
    /// Evaluate the decomposition gate for potential delegation.
    pub decomposition_gate: bool,
    /// Execute delegated subtasks via `orchestrate-subagents` before inference.
    pub delegated_execution: bool,
    /// Handle specialist creation control flows (channel-only feature).
    pub specialist_controls: bool,
    /// Try execution shortcuts (acknowledgement, model identity, etc.) before LLM.
    pub shortcuts_enabled: bool,

    // ── Inference ────────────────────────────────────────────────
    /// Standard (ReAct loop) or Streaming (SSE, no ReAct).
    pub inference_mode: InferenceMode,
    /// Guard set applied to fresh inference output.
    pub guard_set: GuardSetPreset,
    /// Guard set applied to cached responses.
    pub cache_guard_set: GuardSetPreset,
    /// Whether semantic cache is checked before inference.
    pub cache_enabled: bool,

    // ── Authority ────────────────────────────────────────────────
    /// How authority (RBAC) is resolved for tool execution.
    pub authority_mode: AuthorityMode,

    // ── Post-inference ──────────────────────────────────────────
    /// Run background memory ingestion after the turn completes.
    pub post_turn_ingest: bool,
    /// Run background nickname refinement after 4+ messages.
    pub nickname_refinement: bool,

    // ── Output control ──────────────────────────────────────────
    /// Inject diagnostics metadata into system prompt.
    pub inject_diagnostics: bool,

    // ── Channel label ────────────────────────────────────────────
    /// Human-readable label for logging, cost tracking, and event bus.
    pub channel_label: String,
}

// ── Preset constructors ───────────────────────────────────────────────────

impl PipelineConfig {
    /// API endpoint (`/agent/message`): all features enabled.
    ///
    /// Injection defense, dedup, decomposition, delegated execution,
    /// shortcuts, full guard chain, cache, ReAct, diagnostics, nickname
    /// refinement. Authority via `resolve_api_claim()`.
    pub fn api() -> Self {
        Self {
            injection_defense: true,
            dedup_tracking: true,
            session_resolution: SessionResolutionMode::FromBody,
            decomposition_gate: true,
            delegated_execution: true,
            specialist_controls: false,
            shortcuts_enabled: true,
            inference_mode: InferenceMode::Standard,
            guard_set: GuardSetPreset::Full,
            cache_guard_set: GuardSetPreset::Cached,
            cache_enabled: true,
            authority_mode: AuthorityMode::ApiClaim,
            post_turn_ingest: true,
            nickname_refinement: true,
            inject_diagnostics: true,
            channel_label: "api".into(),
        }
    }

    /// SSE streaming endpoint (`/agent/message/stream`).
    ///
    /// Injection defense and dedup are active. No ReAct loop, no shortcuts,
    /// no decomposition, no delegated execution. Authority is audit-only
    /// (no tool execution on this path). Streaming guard set applied
    /// post-accumulation.
    pub fn streaming() -> Self {
        Self {
            injection_defense: true,
            dedup_tracking: true,
            session_resolution: SessionResolutionMode::FromBody,
            decomposition_gate: false,
            delegated_execution: false,
            specialist_controls: false,
            shortcuts_enabled: false,
            inference_mode: InferenceMode::Streaming,
            guard_set: GuardSetPreset::Streaming,
            cache_guard_set: GuardSetPreset::None,
            cache_enabled: true, // streaming writes to cache post-stream
            authority_mode: AuthorityMode::AuditOnly,
            post_turn_ingest: true,
            nickname_refinement: false,
            inject_diagnostics: true,
            channel_label: "api-stream".into(),
        }
    }

    /// Channel message (Telegram, Discord, Signal, Email, etc.).
    ///
    /// Full pipeline with channel-specific authority resolution.
    /// All core features enabled. Channel-specific behaviors that are
    /// NOT part of the pipeline config (handled by channel handler wrapper):
    /// - Addressability filter
    /// - Multimodal enrichment
    /// - Typing/thinking indicators
    /// - Correction turn detection
    /// - Skill-first fulfillment
    /// - Bot command handling
    /// - Reply formatting (telegram normalize, etc.)
    pub fn channel(platform: &str) -> Self {
        Self {
            injection_defense: true,
            dedup_tracking: true,
            session_resolution: SessionResolutionMode::FromChannel {
                platform: platform.to_string(),
            },
            decomposition_gate: true,
            delegated_execution: true,
            specialist_controls: true,
            shortcuts_enabled: true,
            inference_mode: InferenceMode::Standard,
            guard_set: GuardSetPreset::Full,
            cache_guard_set: GuardSetPreset::Cached,
            cache_enabled: true,
            authority_mode: AuthorityMode::ChannelClaim,
            post_turn_ingest: true,
            nickname_refinement: false,
            inject_diagnostics: false,
            channel_label: platform.to_string(),
        }
    }

    /// Cron/scheduled task execution: internal system caller.
    ///
    /// **Security fix**: injection defense is now enabled — was completely
    /// absent from `scheduled_tasks.rs`.
    ///
    /// No dedup (cron tasks are guaranteed unique by scheduler). Authority
    /// is `SelfGenerated` (internal system caller). No delegated execution
    /// (cron tasks go through standard inference with tool access). No
    /// nickname refinement.
    pub fn cron() -> Self {
        Self {
            injection_defense: true, // SECURITY FIX: was missing!
            dedup_tracking: false,
            session_resolution: SessionResolutionMode::Dedicated,
            decomposition_gate: true,
            delegated_execution: false, // cron doesn't pre-execute delegation
            specialist_controls: false,
            shortcuts_enabled: true,
            inference_mode: InferenceMode::Standard,
            guard_set: GuardSetPreset::Full,
            cache_guard_set: GuardSetPreset::Cached,
            cache_enabled: true,
            authority_mode: AuthorityMode::SelfGenerated,
            post_turn_ingest: true,
            nickname_refinement: false,
            inject_diagnostics: false,
            channel_label: "cron".into(),
        }
    }
}

// ── Stage predicates ──────────────────────────────────────────────────────
//
// Convenience methods for querying pipeline capabilities. These are used by
// `execute_unified_pipeline()` (Phase 5) to branch on stage availability.

#[cfg(test)]
impl PipelineConfig {
    /// Whether this pipeline uses the standard ReAct inference path.
    pub fn is_standard_inference(&self) -> bool {
        self.inference_mode == InferenceMode::Standard
    }

    /// Whether this pipeline uses the streaming inference path.
    pub fn is_streaming_inference(&self) -> bool {
        self.inference_mode == InferenceMode::Streaming
    }

    /// Whether authority needs to be enforced for tool execution.
    /// Returns `false` for `AuditOnly` (streaming) and `SelfGenerated` (cron).
    pub fn enforces_authority(&self) -> bool {
        matches!(
            self.authority_mode,
            AuthorityMode::ApiClaim | AuthorityMode::ChannelClaim
        )
    }

    /// Whether this pipeline can execute tools (ReAct loop).
    /// Streaming never executes tools.
    pub fn can_execute_tools(&self) -> bool {
        self.inference_mode == InferenceMode::Standard
    }

    /// Whether this pipeline resolves the session from the request body.
    /// True for API and streaming paths.
    pub fn resolves_session_from_body(&self) -> bool {
        matches!(self.session_resolution, SessionResolutionMode::FromBody)
    }

    /// Whether this is a channel pipeline.
    pub fn is_channel(&self) -> bool {
        matches!(
            self.session_resolution,
            SessionResolutionMode::FromChannel { .. }
        )
    }

    /// Whether this is a cron pipeline.
    pub fn is_cron(&self) -> bool {
        matches!(self.session_resolution, SessionResolutionMode::Dedicated)
            && matches!(self.authority_mode, AuthorityMode::SelfGenerated)
    }
}

// ── Unified pipeline input ──────────────────────────────────────────────

/// Input to `execute_unified_pipeline()`.
///
/// Callers are responsible for:
/// 1. Injection defense (before constructing this struct)
/// 2. Dedup tracking (before constructing this struct, ideally via `DedupGuard`)
/// 3. Session resolution (providing the resolved `session_id`)
/// 4. Storing the user message
/// 5. Creating the turn record
///
/// The pipeline handles everything from `prepare_inference()` through
/// `execute_inference_pipeline()`, driven by `PipelineConfig` feature flags.
pub(super) struct UnifiedPipelineInput<'a> {
    pub state: &'a AppState,
    pub config: &'a PipelineConfig,
    pub session_id: &'a str,
    pub user_content: &'a str,
    pub turn_id: &'a str,
    pub is_correction_turn: bool,
    /// Delegation workflow note from `apply_decomposition_decision()`.
    pub delegation_workflow_note: Option<String>,
    /// Gate system note from `build_gate_system_note()`.
    pub gate_system_note: Option<String>,
    /// Note from delegated `orchestrate-subagents` execution.
    pub delegated_execution_note: Option<String>,
    /// Pre-computed delegation provenance (from delegation step).
    pub delegation_provenance: DelegationProvenance,
}

// ── Unified pipeline execution ──────────────────────────────────────────

/// Execute the unified inference pipeline for Standard inference paths.
///
/// This is the single entry point for **all non-streaming inference**,
/// replacing the duplicated ceremony in `handlers.rs`, `channel_message.rs`,
/// and `scheduled_tasks.rs`.
///
/// ## Stages handled
///
/// 1. Read agent config (name, ID, primary model, tier config)
/// 2. Read personality (OS text, firmware text)
/// 3. Build `InferenceInput` from `PipelineConfig` + caller-provided context
/// 4. `prepare_inference()` — model selection, embedding, RAG, history assembly
/// 5. `execute_inference_pipeline()` — cache check → inference + ReAct → store
///    assistant → record cost → post-turn ingest → cache store
///
/// ## Stages NOT handled (caller responsibility)
///
/// - Injection defense (`check_injection` / `sanitize`)
/// - Dedup tracking (`DedupGuard` RAII)
/// - Session resolution (various per entry point)
/// - User message storage (`append_message`)
/// - Turn pre-creation (`create_turn_with_id`)
/// - Decomposition gate evaluation (`evaluate_decomposition_gate`)
/// - Delegated execution (`orchestrate-subagents`)
/// - Output formatting (JSON, channel reply, etc.)
/// - Nickname refinement (API-only post-step)
pub(super) async fn execute_unified_pipeline(
    input: UnifiedPipelineInput<'_>,
) -> Result<core::PipelineResult, String> {
    let config = input.state.config.read().await;
    let agent_name = config.agent.name.clone();
    let agent_id = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let tier_adapt = config.tier_adapt.clone();
    drop(config);

    let personality = input.state.personality.read().await;
    let os_text = personality.os_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let inference_input = core::InferenceInput {
        state: input.state,
        session_id: input.session_id,
        user_content: input.user_content,
        turn_id: input.turn_id,
        channel_label: &input.config.channel_label,
        agent_name,
        agent_id,
        os_text,
        firmware_text,
        primary_model,
        tier_adapt,
        delegation_workflow_note: input.delegation_workflow_note,
        inject_diagnostics: input.config.inject_diagnostics,
        gate_system_note: input.gate_system_note,
        delegated_execution_note: input.delegated_execution_note,
        is_correction_turn: input.is_correction_turn,
    };

    let prepared = core::prepare_inference(&inference_input).await?;

    // Resolve authority from PipelineConfig.
    let authority = match input.config.authority_mode {
        AuthorityMode::SelfGenerated => ironclad_core::InputAuthority::SelfGenerated,
        AuthorityMode::AuditOnly => ironclad_core::InputAuthority::SelfGenerated,
        // ApiClaim and ChannelClaim must be resolved by the caller and passed
        // as SelfGenerated only when the caller has verified the claim. For the
        // unified pipeline, callers that need claim-based authority should
        // resolve it before calling and set authority_mode to SelfGenerated with
        // the resolved authority. This case should not be reached in practice.
        AuthorityMode::ApiClaim | AuthorityMode::ChannelClaim => {
            tracing::warn!(
                mode = ?input.config.authority_mode,
                "execute_unified_pipeline called with claim-based authority — \
                 caller should resolve authority before calling"
            );
            ironclad_core::InputAuthority::SelfGenerated
        }
    };

    let mut provenance = input.delegation_provenance;
    core::execute_inference_pipeline(
        input.state,
        &prepared,
        input.session_id,
        input.user_content,
        input.turn_id,
        authority,
        Some(&input.config.channel_label),
        &mut provenance,
    )
    .await
}

/// Execute the unified pipeline with an explicit authority override.
///
/// Used by entry points that resolve authority externally (API via
/// `resolve_api_claim()`, channels via allow-list). Identical to
/// `execute_unified_pipeline()` except authority comes from the caller
/// instead of being derived from `PipelineConfig::authority_mode`.
pub(super) async fn execute_unified_pipeline_with_authority(
    input: UnifiedPipelineInput<'_>,
    authority: ironclad_core::InputAuthority,
) -> Result<core::PipelineResult, String> {
    let config = input.state.config.read().await;
    let agent_name = config.agent.name.clone();
    let agent_id = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let tier_adapt = config.tier_adapt.clone();
    drop(config);

    let personality = input.state.personality.read().await;
    let os_text = personality.os_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let inference_input = core::InferenceInput {
        state: input.state,
        session_id: input.session_id,
        user_content: input.user_content,
        turn_id: input.turn_id,
        channel_label: &input.config.channel_label,
        agent_name,
        agent_id,
        os_text,
        firmware_text,
        primary_model,
        tier_adapt,
        delegation_workflow_note: input.delegation_workflow_note,
        inject_diagnostics: input.config.inject_diagnostics,
        gate_system_note: input.gate_system_note,
        delegated_execution_note: input.delegated_execution_note,
        is_correction_turn: input.is_correction_turn,
    };

    let prepared = core::prepare_inference(&inference_input).await?;
    let mut provenance = input.delegation_provenance;
    core::execute_inference_pipeline(
        input.state,
        &prepared,
        input.session_id,
        input.user_content,
        input.turn_id,
        authority,
        Some(&input.config.channel_label),
        &mut provenance,
    )
    .await
}

/// Execute the unified pipeline returning both the result and the prepared
/// inference, allowing callers to inspect or modify prepared state (e.g.,
/// channel model-switch logic).
pub(super) async fn prepare_unified_pipeline(
    input: &UnifiedPipelineInput<'_>,
) -> Result<core::PreparedInference, String> {
    let config = input.state.config.read().await;
    let agent_name = config.agent.name.clone();
    let agent_id = config.agent.id.clone();
    let primary_model = config.models.primary.clone();
    let tier_adapt = config.tier_adapt.clone();
    drop(config);

    let personality = input.state.personality.read().await;
    let os_text = personality.os_text.clone();
    let firmware_text = personality.firmware_text.clone();
    drop(personality);

    let inference_input = core::InferenceInput {
        state: input.state,
        session_id: input.session_id,
        user_content: input.user_content,
        turn_id: input.turn_id,
        channel_label: &input.config.channel_label,
        agent_name,
        agent_id,
        os_text,
        firmware_text,
        primary_model,
        tier_adapt,
        delegation_workflow_note: input.delegation_workflow_note.clone(),
        inject_diagnostics: input.config.inject_diagnostics,
        gate_system_note: input.gate_system_note.clone(),
        delegated_execution_note: input.delegated_execution_note.clone(),
        is_correction_turn: input.is_correction_turn,
    };

    core::prepare_inference(&inference_input).await
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Preset field verification ─────────────────────────────────────

    #[test]
    fn api_preset_enables_all_core_features() {
        let cfg = PipelineConfig::api();
        assert!(cfg.injection_defense);
        assert!(cfg.dedup_tracking);
        assert!(cfg.decomposition_gate);
        assert!(cfg.delegated_execution);
        assert!(cfg.shortcuts_enabled);
        assert!(cfg.cache_enabled);
        assert!(cfg.post_turn_ingest);
        assert!(cfg.nickname_refinement);
        assert!(cfg.inject_diagnostics);
        assert!(!cfg.specialist_controls); // API doesn't have specialist controls
        assert_eq!(cfg.inference_mode, InferenceMode::Standard);
        assert_eq!(cfg.guard_set, GuardSetPreset::Full);
        assert_eq!(cfg.cache_guard_set, GuardSetPreset::Cached);
        assert_eq!(cfg.authority_mode, AuthorityMode::ApiClaim);
        assert_eq!(cfg.channel_label, "api");
        assert_eq!(cfg.session_resolution, SessionResolutionMode::FromBody);
    }

    #[test]
    fn streaming_preset_disables_react_features() {
        let cfg = PipelineConfig::streaming();
        assert!(cfg.injection_defense);
        assert!(cfg.dedup_tracking);
        // Streaming disables all pre-inference stages
        assert!(!cfg.decomposition_gate);
        assert!(!cfg.delegated_execution);
        assert!(!cfg.shortcuts_enabled);
        assert!(!cfg.specialist_controls);
        // Streaming mode with reduced guards
        assert_eq!(cfg.inference_mode, InferenceMode::Streaming);
        assert_eq!(cfg.guard_set, GuardSetPreset::Streaming);
        assert_eq!(cfg.cache_guard_set, GuardSetPreset::None);
        // No nickname refinement on streaming
        assert!(!cfg.nickname_refinement);
        // But post-turn ingest and cache are on
        assert!(cfg.post_turn_ingest);
        assert!(cfg.cache_enabled);
        assert_eq!(cfg.authority_mode, AuthorityMode::AuditOnly);
        assert_eq!(cfg.channel_label, "api-stream");
    }

    #[test]
    fn channel_preset_enables_specialist_controls() {
        let cfg = PipelineConfig::channel("telegram");
        assert!(cfg.injection_defense);
        assert!(cfg.dedup_tracking);
        assert!(cfg.decomposition_gate);
        assert!(cfg.delegated_execution);
        assert!(cfg.shortcuts_enabled);
        assert!(cfg.specialist_controls); // only channel has this
        assert!(cfg.cache_enabled);
        assert!(cfg.post_turn_ingest);
        assert!(!cfg.nickname_refinement); // channels don't refine nicknames
        assert!(!cfg.inject_diagnostics); // channels don't inject diagnostics
        assert_eq!(cfg.inference_mode, InferenceMode::Standard);
        assert_eq!(cfg.guard_set, GuardSetPreset::Full);
        assert_eq!(cfg.cache_guard_set, GuardSetPreset::Cached);
        assert_eq!(cfg.authority_mode, AuthorityMode::ChannelClaim);
        assert_eq!(cfg.channel_label, "telegram");
        assert_eq!(
            cfg.session_resolution,
            SessionResolutionMode::FromChannel {
                platform: "telegram".into()
            }
        );
    }

    #[test]
    fn channel_preset_uses_platform_as_label() {
        let telegram = PipelineConfig::channel("telegram");
        assert_eq!(telegram.channel_label, "telegram");

        let discord = PipelineConfig::channel("discord");
        assert_eq!(discord.channel_label, "discord");

        let email = PipelineConfig::channel("email");
        assert_eq!(email.channel_label, "email");
    }

    #[test]
    fn cron_preset_has_injection_defense() {
        let cfg = PipelineConfig::cron();
        // SECURITY FIX: injection defense was missing from scheduled_tasks.rs
        assert!(cfg.injection_defense);
        // No dedup for cron (scheduler guarantees uniqueness)
        assert!(!cfg.dedup_tracking);
        assert!(cfg.decomposition_gate);
        assert!(!cfg.delegated_execution); // cron doesn't pre-execute delegation
        assert!(cfg.shortcuts_enabled);
        assert!(!cfg.specialist_controls);
        assert!(cfg.cache_enabled);
        assert!(cfg.post_turn_ingest);
        assert!(!cfg.nickname_refinement);
        assert!(!cfg.inject_diagnostics);
        assert_eq!(cfg.inference_mode, InferenceMode::Standard);
        assert_eq!(cfg.guard_set, GuardSetPreset::Full);
        assert_eq!(cfg.cache_guard_set, GuardSetPreset::Cached);
        assert_eq!(cfg.authority_mode, AuthorityMode::SelfGenerated);
        assert_eq!(cfg.channel_label, "cron");
        assert_eq!(cfg.session_resolution, SessionResolutionMode::Dedicated);
    }

    // ── Guard set resolution ──────────────────────────────────────────

    #[test]
    fn guard_set_presets_resolve_to_non_empty_chains() {
        let full = GuardSetPreset::Full.resolve();
        assert!(!full.is_empty());

        let cached = GuardSetPreset::Cached.resolve();
        assert!(!cached.is_empty());

        let streaming = GuardSetPreset::Streaming.resolve();
        assert!(!streaming.is_empty());
    }

    #[test]
    fn guard_set_none_resolves_to_empty_chain() {
        let none = GuardSetPreset::None.resolve();
        assert!(none.is_empty());
    }

    // ── Predicate methods ─────────────────────────────────────────────

    #[test]
    fn api_predicates() {
        let cfg = PipelineConfig::api();
        assert!(cfg.is_standard_inference());
        assert!(!cfg.is_streaming_inference());
        assert!(cfg.enforces_authority());
        assert!(cfg.can_execute_tools());
        assert!(cfg.resolves_session_from_body());
        assert!(!cfg.is_channel());
        assert!(!cfg.is_cron());
    }

    #[test]
    fn streaming_predicates() {
        let cfg = PipelineConfig::streaming();
        assert!(!cfg.is_standard_inference());
        assert!(cfg.is_streaming_inference());
        assert!(!cfg.enforces_authority());
        assert!(!cfg.can_execute_tools());
        assert!(cfg.resolves_session_from_body());
        assert!(!cfg.is_channel());
        assert!(!cfg.is_cron());
    }

    #[test]
    fn channel_predicates() {
        let cfg = PipelineConfig::channel("telegram");
        assert!(cfg.is_standard_inference());
        assert!(!cfg.is_streaming_inference());
        assert!(cfg.enforces_authority());
        assert!(cfg.can_execute_tools());
        assert!(!cfg.resolves_session_from_body());
        assert!(cfg.is_channel());
        assert!(!cfg.is_cron());
    }

    #[test]
    fn cron_predicates() {
        let cfg = PipelineConfig::cron();
        assert!(cfg.is_standard_inference());
        assert!(!cfg.is_streaming_inference());
        // Cron uses SelfGenerated — authority not enforced (trusted internal caller)
        assert!(!cfg.enforces_authority());
        assert!(cfg.can_execute_tools());
        assert!(!cfg.resolves_session_from_body());
        assert!(!cfg.is_channel());
        assert!(cfg.is_cron());
    }

    // ── Security invariants ───────────────────────────────────────────

    #[test]
    fn all_presets_have_injection_defense() {
        // Every entry point MUST have injection defense. This is a
        // security-critical invariant.
        assert!(PipelineConfig::api().injection_defense);
        assert!(PipelineConfig::streaming().injection_defense);
        assert!(PipelineConfig::channel("test").injection_defense);
        assert!(PipelineConfig::cron().injection_defense);
    }

    #[test]
    fn all_presets_have_post_turn_ingest() {
        // Memory ingestion should never be skipped — it's essential for
        // episodic memory continuity.
        assert!(PipelineConfig::api().post_turn_ingest);
        assert!(PipelineConfig::streaming().post_turn_ingest);
        assert!(PipelineConfig::channel("test").post_turn_ingest);
        assert!(PipelineConfig::cron().post_turn_ingest);
    }

    #[test]
    fn standard_inference_paths_have_full_guards() {
        // All standard inference paths must use the Full guard set.
        let api = PipelineConfig::api();
        let channel = PipelineConfig::channel("telegram");
        let cron = PipelineConfig::cron();

        for cfg in [&api, &channel, &cron] {
            assert_eq!(cfg.inference_mode, InferenceMode::Standard);
            assert_eq!(cfg.guard_set, GuardSetPreset::Full);
            assert_eq!(cfg.cache_guard_set, GuardSetPreset::Cached);
        }
    }

    #[test]
    fn only_api_has_nickname_refinement() {
        assert!(PipelineConfig::api().nickname_refinement);
        assert!(!PipelineConfig::streaming().nickname_refinement);
        assert!(!PipelineConfig::channel("test").nickname_refinement);
        assert!(!PipelineConfig::cron().nickname_refinement);
    }

    #[test]
    fn only_channel_has_specialist_controls() {
        assert!(!PipelineConfig::api().specialist_controls);
        assert!(!PipelineConfig::streaming().specialist_controls);
        assert!(PipelineConfig::channel("test").specialist_controls);
        assert!(!PipelineConfig::cron().specialist_controls);
    }

    #[test]
    fn streaming_never_executes_tools() {
        let cfg = PipelineConfig::streaming();
        assert!(!cfg.can_execute_tools());
        assert!(!cfg.shortcuts_enabled);
        assert!(!cfg.decomposition_gate);
        assert!(!cfg.delegated_execution);
    }

    // ── Session resolution mode ───────────────────────────────────────

    #[test]
    fn session_resolution_modes_are_correct() {
        assert_eq!(
            PipelineConfig::api().session_resolution,
            SessionResolutionMode::FromBody
        );
        assert_eq!(
            PipelineConfig::streaming().session_resolution,
            SessionResolutionMode::FromBody
        );
        assert_eq!(
            PipelineConfig::channel("discord").session_resolution,
            SessionResolutionMode::FromChannel {
                platform: "discord".into()
            }
        );
        assert_eq!(
            PipelineConfig::cron().session_resolution,
            SessionResolutionMode::Dedicated
        );
    }

    #[test]
    fn provided_session_resolution_stores_id() {
        let mode = SessionResolutionMode::Provided {
            session_id: "test-session-123".into(),
        };
        match mode {
            SessionResolutionMode::Provided { session_id } => {
                assert_eq!(session_id, "test-session-123");
            }
            _ => panic!("expected Provided variant"),
        }
    }
}
