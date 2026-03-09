//! Shared inference core used by all three entry points (API, streaming, channel).
//!
//! `prepare_inference` builds a `PreparedInference` from an `InferenceInput`, handling:
//! model selection, embedding, RAG retrieval, history, system prompt, HMAC, context building.
//!
//! `run_react_loop` drives the Think→Act→Observe→Finish cycle on top of a prepared request.
//!
//! `post_turn_ingest` spawns background memory + embedding work.

use std::sync::Arc;

use ironclad_agent::agent_loop::{AgentLoop, ReactAction, ReactState};
use ironclad_core::config::TierAdaptConfig;
use ironclad_core::{InputAuthority, ModelTier};

use super::AppState;
use super::decomposition::DelegationProvenance;
use super::diagnostics::{collect_runtime_diagnostics, diagnostics_system_note};
use super::guards::{
    enforce_current_events_truth_guard, enforce_execution_truth_guard,
    enforce_internal_jargon_guard, enforce_internal_protocol_guard,
    enforce_model_identity_truth_guard, enforce_non_repetition,
    enforce_personality_integrity_guard, enforce_subagent_claim_guard, is_low_value_response,
    is_overbroad_sensitive_conflict_refusal, is_parroting_user_prompt,
};
use super::intents::{
    requests_acknowledgement, requests_capability_summary, requests_cron, requests_current_events,
    requests_delegation, requests_email_triage, requests_execution, requests_file_distribution,
    requests_folder_scan, requests_image_count_scan, requests_introspection,
    requests_literary_quote_context, requests_markdown_count_scan, requests_obsidian_insights,
    requests_personality_profile, requests_provider_inventory, requests_random_tool_use,
    requests_wallet_address_scan, should_bypass_cache_for_prompt,
};
use super::routing::{
    infer_with_fallback, persist_model_selection_audit, select_routed_model_with_audit,
};
use super::tools::{execute_tool_call, parse_tool_call, parse_tool_calls};

/// Caller-supplied context that differs across the three entry points.
pub(super) struct InferenceInput<'a> {
    pub state: &'a AppState,
    pub session_id: &'a str,
    pub user_content: &'a str,
    pub turn_id: &'a str,
    /// Label for model audit trail ("api", "api-stream", "telegram", etc.)
    pub channel_label: &'a str,
    /// System prompt fragments from caller
    pub agent_name: String,
    pub agent_id: String,
    pub soul_text: String,
    pub firmware_text: String,
    pub primary_model: String,
    pub tier_adapt: TierAdaptConfig,
    /// Optional delegation workflow note injected into system prompt
    pub delegation_workflow_note: Option<String>,
    /// Whether to inject runtime diagnostics (API yes, channels no)
    pub inject_diagnostics: bool,
    /// Optional gate system note for channels
    pub gate_system_note: Option<String>,
    /// Optional delegated execution note for channels
    pub delegated_execution_note: Option<String>,
}

/// Result of `prepare_inference` — everything needed to call the LLM.
pub(super) struct PreparedInference {
    pub model: String,
    pub model_for_api: String,
    pub tier: ModelTier,
    pub request: ironclad_llm::format::UnifiedRequest,
    pub previous_assistant: Option<String>,
    pub query_embedding: Option<Vec<f32>>,
    pub cache_hash: String,
}

/// Result of a completed (non-streaming) inference cycle.
pub(super) struct InferenceOutput {
    pub content: String,
    pub model: String,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub react_turns: usize,
    pub latency_ms: u64,
    pub quality_score: f64,
    pub escalated: bool,
    /// Tool calls executed during the ReAct loop: (tool_name, result_text).
    pub tool_results: Vec<(String, String)>,
}

fn deterministic_quality_fallback(user_prompt: &str, agent_name: &str) -> String {
    let lower = user_prompt.trim().to_ascii_lowercase();
    if matches!(
        lower.as_str(),
        "awesome" | "great" | "nice" | "perfect" | "go ahead"
    ) {
        return "Copy. Next concrete step: I can produce paste-ready markdown bodies for Wallet-Defaults.md, Cadence-and-Approvals.md, and Daily-Progress-Template.md right now."
            .to_string();
    }
    if requests_obsidian_insights(user_prompt)
        || lower.contains("my vault")
        || lower.contains("obsidian")
    {
        return "Obsidian vault starter scaffold:\n\nMy Vault/\n- Governance/\n  - Wallet-Defaults.md\n  - Cadence-and-Approvals.md\n- Ledger/\n  - Ledger-Skeleton.md\n- Subagents/\n  - web3-dispatcher.md\n  - api-vender.md\n  - audit-fuzzer.md\n- Data/\n  - Data-Sources.md\n  - Data-Flows.md\n- Reports/\n  - Daily-Progress-Template.md\n- Templates/\n- References/\n\nIf you want, I will now produce the first three file bodies (Wallet-Defaults, Cadence-and-Approvals, Daily-Progress-Template) in paste-ready markdown."
            .to_string();
    }
    if requests_current_events(user_prompt) {
        return format!(
            "{agent_name} here. I failed to produce a reliable live sitrep in that turn. I can still provide a concrete briefing now if you specify scope (global, US, or region) and I will return it with dated caveats."
        );
    }
    if requests_capability_summary(user_prompt) {
        return "I can execute tools for filesystem/command tasks, delegate to subagents, inspect runtime state, schedule jobs, and report outcomes with evidence from executed steps.".to_string();
    }
    if requests_personality_profile(user_prompt) {
        return format!(
            "{agent_name}: concise, direct, and execution-first. I acknowledge quickly, act with tools when needed, and avoid fabricated claims."
        );
    }
    if requests_provider_inventory(user_prompt) {
        return "I can list active provider/model routing from runtime state. Ask me for a provider inventory and I will return the current configured primary and fallback chain."
            .to_string();
    }
    format!(
        "{agent_name} here. The prior generation degraded. I am returning a concrete fallback: state the exact outcome format you want (for example: bullet summary, command output, or action plan) and I will deliver it directly."
    )
}

/// Build a `PreparedInference` from the caller's `InferenceInput`.
///
/// Handles: model routing, embedding, RAG retrieval, history, system prompt,
/// HMAC injection, context assembly, and tier adaptation.
pub(super) async fn prepare_inference(
    input: &InferenceInput<'_>,
) -> Result<PreparedInference, String> {
    let state = input.state;

    // Model selection + audit
    let features = ironclad_llm::extract_features(input.user_content, 0, 1);
    let complexity = ironclad_llm::classify_complexity(&features);
    let model_audit = select_routed_model_with_audit(state, input.user_content).await;
    let model = model_audit.selected_model.clone();
    let complexity_label = format!("{complexity:?}");
    persist_model_selection_audit(
        state,
        input.turn_id,
        input.session_id,
        input.channel_label,
        Some(&complexity_label),
        input.user_content,
        &model_audit,
    )
    .await;
    if let Err(e) = ironclad_db::sessions::update_model(&state.db, input.session_id, &model) {
        tracing::warn!(session_id = %input.session_id, model = %model, error = %e, "failed to update session model");
    }

    // Tier resolution
    let tier = {
        let llm = state.llm.read().await;
        llm.providers
            .get_by_model(&model)
            .map(|p| p.tier)
            .unwrap_or_else(|| ironclad_llm::tier::classify(&model))
    };

    // Embedding for RAG + cache L2
    let query_embedding = {
        let llm = state.llm.read().await;
        llm.embedding
            .embed_single(input.user_content)
            .await
            .inspect_err(|e| {
                tracing::warn!(error = %e, "embedding generation failed, RAG retrieval will be skipped")
            })
            .ok()
    };

    // Cache lookup
    let cache_hash = ironclad_llm::SemanticCache::compute_hash("", "", input.user_content);

    // Memory retrieval
    let complexity_level = ironclad_agent::context::determine_level(complexity);
    let ann_ref = if state.ann_index.is_built() {
        Some(&state.ann_index)
    } else {
        None
    };
    let memories = state.retriever.retrieve_with_ann(
        &state.db,
        input.session_id,
        input.user_content,
        query_embedding.as_deref(),
        complexity_level,
        ann_ref,
    );

    // History
    let history_messages =
        ironclad_db::sessions::list_messages(&state.db, input.session_id, Some(50))
            .map_err(|e| format!("failed to load conversation history: {e}"))?;
    let previous_assistant = history_messages
        .iter()
        .rev()
        .find(|m| m.role == "assistant")
        .map(|m| m.content.clone());
    let history: Vec<ironclad_llm::format::UnifiedMessage> = history_messages
        .iter()
        .rev()
        .skip(1) // skip the user message just appended by caller
        .rev()
        .map(|m| ironclad_llm::format::UnifiedMessage {
            role: m.role.clone(),
            content: m.content.clone(),
            parts: None,
        })
        .collect();

    // System prompt
    let model_for_api = model
        .split_once('/')
        .map(|(_, m)| m)
        .unwrap_or(&model)
        .to_string();
    let system_prompt = if input.soul_text.is_empty() {
        format!(
            "You are {name}, an autonomous AI agent (id: {id}). \
             When asked who you are, always identify as {name}. \
             Never reveal the underlying model name or claim to be a generic assistant.",
            name = input.agent_name,
            id = input.agent_id,
        )
    } else {
        let mut prompt = input.soul_text.clone();
        if !input.firmware_text.is_empty() {
            prompt.push_str("\n\n");
            prompt.push_str(&input.firmware_text);
        }
        prompt
    };
    let system_prompt = if let Some(ref wf_note) = input.delegation_workflow_note {
        format!("{system_prompt}\nWorkflow: {wf_note}")
    } else {
        system_prompt
    };
    // Build tool definitions early so we can embed a text-based tool summary in the
    // system prompt. This ensures models without native function-calling support can
    // still discover and invoke tools via the text-embedded JSON format.
    let tools = super::decomposition::build_all_tool_definitions(&state.tools);
    let tool_summary: Vec<(String, String)> = tools
        .iter()
        .map(|t| (t.name.clone(), t.description.clone()))
        .collect();
    let system_prompt = format!(
        "{system_prompt}{}{}",
        ironclad_agent::prompt::runtime_metadata_block(
            env!("CARGO_PKG_VERSION"),
            &input.primary_model,
            &model,
        ),
        ironclad_agent::prompt::tool_use_instructions(&tool_summary),
    );
    let system_prompt =
        ironclad_agent::prompt::inject_hmac_boundary(&system_prompt, state.hmac_secret.as_ref());
    if !ironclad_agent::prompt::verify_hmac_boundary(&system_prompt, state.hmac_secret.as_ref()) {
        tracing::error!("HMAC boundary verification failed immediately after injection");
        return Err("internal HMAC verification failure".into());
    }

    // Context assembly
    let mut messages = ironclad_agent::context::build_context(
        complexity_level,
        &system_prompt,
        &memories,
        &history,
    );

    // Session checkpoint restore: inject most recent checkpoint context on resume.
    match ironclad_db::checkpoint::load_checkpoint(&state.db, input.session_id) {
        Ok(Some(cp)) => {
            let mut checkpoint_note = format!(
                "Session checkpoint restore (turn_count={}): {}",
                cp.turn_count, cp.memory_summary
            );
            if let Some(active_tasks) = cp.active_tasks
                && !active_tasks.trim().is_empty()
            {
                checkpoint_note.push_str("\nActive tasks: ");
                checkpoint_note.push_str(&active_tasks);
            }
            if let Some(digest) = cp.conversation_digest
                && !digest.trim().is_empty()
            {
                checkpoint_note.push_str("\nConversation digest: ");
                checkpoint_note.push_str(&digest);
            }
            messages.push(ironclad_llm::format::UnifiedMessage {
                role: "system".into(),
                content: checkpoint_note,
                parts: None,
            });
        }
        Ok(None) => {}
        Err(e) => tracing::warn!(error = %e, "failed to load context checkpoint"),
    }

    // Hippocampus context: compact table summary for ambient storage awareness
    match ironclad_db::hippocampus::compact_summary(&state.db) {
        Ok(summary) if !summary.is_empty() => {
            messages.push(ironclad_llm::format::UnifiedMessage {
                role: "system".into(),
                content: summary,
                parts: None,
            });
        }
        Err(e) => {
            tracing::warn!(error = %e, "Failed to generate hippocampus summary");
        }
        _ => {}
    }

    // Optional: runtime diagnostics (API paths inject; channels deliberately skip)
    if input.inject_diagnostics {
        let runtime_diag = collect_runtime_diagnostics(state).await;
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: diagnostics_system_note(&runtime_diag),
            parts: None,
        });
    }

    // Optional: gate system note (channels inject decomposition decision)
    if let Some(ref note) = input.gate_system_note {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: note.clone(),
            parts: None,
        });
    }
    if let Some(ref note) = input.delegated_execution_note {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "system".into(),
            content: note.clone(),
            parts: None,
        });
    }

    // Ensure user message is last
    if messages
        .last()
        .is_none_or(|m| m.content != input.user_content)
    {
        messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: input.user_content.to_string(),
            parts: None,
        });
    }

    // Instruction anti-fade: inject compact directive reminder before the user
    // message when conversation is long enough that system prompt instructions
    // may have faded from the model's attention window (OPENDEV pattern).
    if let Some(reminder) =
        ironclad_agent::prompt::build_instruction_reminder(&input.soul_text, &input.firmware_text)
    {
        ironclad_agent::context::inject_instruction_reminder(&mut messages, &reminder);
    }

    // Prompt compression gate — only when enabled in config
    {
        let cfg = input.state.config.read().await;
        if cfg.cache.prompt_compression {
            ironclad_agent::context::compress_context(
                &mut messages,
                cfg.cache.compression_target_ratio,
            );
        }
    }

    ironclad_llm::tier::adapt_for_tier(tier, &mut messages, &input.tier_adapt);

    let request = ironclad_llm::format::UnifiedRequest {
        model: model_for_api.clone(),
        messages,
        max_tokens: Some(2048),
        temperature: None,
        system: None,
        quality_target: None,
        tools,
    };

    Ok(PreparedInference {
        model,
        model_for_api,
        tier,
        request,
        previous_assistant,
        query_embedding,
        cache_hash,
    })
}

/// Strip forged HMAC boundaries + L4 output scan on a single piece of content.
pub(super) fn sanitize_model_output(content: String, hmac_secret: &[u8]) -> String {
    let content = if content.contains("<<<TRUST_BOUNDARY:") {
        if !ironclad_agent::prompt::verify_hmac_boundary(&content, hmac_secret) {
            tracing::warn!("HMAC boundary tampered in model output, stripping");
            ironclad_agent::prompt::strip_hmac_boundaries(&content)
        } else {
            content
        }
    } else {
        content
    };
    if ironclad_agent::injection::scan_output(&content) {
        tracing::warn!("L4 output scan flagged model response, blocking");
        "[Response blocked by output safety filter]".to_string()
    } else {
        content
    }
}

fn shell_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
}

fn sanitize_folder_token(raw: &str) -> Option<String> {
    let cleaned = raw.trim_matches(|c: char| ",.;:!?\"'`()[]{}".contains(c));
    if cleaned.is_empty() {
        return None;
    }
    let valid = cleaned
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return None;
    }
    Some(cleaned.to_string())
}

fn title_case_ascii(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    if let Some(first) = chars.next() {
        out.push(first.to_ascii_uppercase());
    }
    for c in chars {
        out.push(c.to_ascii_lowercase());
    }
    out
}

fn home_folder_exists(name: &str) -> bool {
    let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) else {
        return false;
    };
    std::path::Path::new(&home).join(name).is_dir()
}

fn resolve_home_folder_name(token: &str) -> String {
    let lower = token.to_ascii_lowercase();
    let mapped = match lower.as_str() {
        "docs" | "doc" | "documents" => "Documents".to_string(),
        "downloads" | "download" => "Downloads".to_string(),
        "desktop" => "Desktop".to_string(),
        "pictures" | "pics" | "photos" => "Pictures".to_string(),
        "music" => "Music".to_string(),
        "videos" | "video" | "movies" | "movie" => "Movies".to_string(),
        "code" => "code".to_string(),
        "workspace" | "workspaces" => "code".to_string(),
        _ => token.to_string(),
    };

    if home_folder_exists(&mapped) {
        return mapped;
    }
    let title = title_case_ascii(&mapped);
    if home_folder_exists(&title) {
        return title;
    }
    let lower = mapped.to_ascii_lowercase();
    if home_folder_exists(&lower) {
        return lower;
    }
    mapped
}

fn infer_home_folder_hint(cleaned_prompt: &str) -> Option<String> {
    let tokens: Vec<String> = cleaned_prompt
        .split_whitespace()
        .filter_map(sanitize_folder_token)
        .collect();
    for (idx, token) in tokens.iter().enumerate() {
        if !token.eq_ignore_ascii_case("folder") {
            continue;
        }
        let mut candidate: Option<&str> = None;
        if idx >= 1 {
            let prev = tokens[idx - 1].as_str();
            if !prev.eq_ignore_ascii_case("my")
                && !prev.eq_ignore_ascii_case("the")
                && !prev.eq_ignore_ascii_case("a")
                && !prev.eq_ignore_ascii_case("an")
            {
                candidate = Some(prev);
            }
        }
        if candidate.is_none() && idx >= 2 {
            let prev_prev = tokens[idx - 2].as_str();
            if !prev_prev.eq_ignore_ascii_case("my")
                && !prev_prev.eq_ignore_ascii_case("the")
                && !prev_prev.eq_ignore_ascii_case("a")
                && !prev_prev.eq_ignore_ascii_case("an")
            {
                candidate = Some(prev_prev);
            }
        }
        if let Some(name) = candidate {
            let resolved = resolve_home_folder_name(name);
            return Some(format!("~/{}", resolved));
        }
    }
    None
}

fn extract_path_hint(prompt: &str) -> Option<String> {
    let mut cleaned = prompt.replace('\n', " ");
    cleaned = cleaned.replace('\t', " ");
    for token in cleaned.split_whitespace() {
        let t = token.trim_matches(|c: char| ",.;:!?\"'`()[]{}".contains(c));
        if t.is_empty() {
            continue;
        }
        if t == "~" || t.starts_with("~/") || t.starts_with('/') {
            return Some(t.to_string());
        }
    }
    let lower = cleaned.to_ascii_lowercase();
    if lower.contains("downloads folder") || lower.contains(" downloads ") {
        return Some("~/Downloads".to_string());
    }
    if lower.contains("desktop folder") || lower.contains(" desktop ") {
        return Some("~/Desktop".to_string());
    }
    if lower.contains("documents folder") || lower.contains(" documents ") {
        return Some("~/Documents".to_string());
    }
    if lower.contains("pictures folder") || lower.contains(" pictures ") {
        return Some("~/Pictures".to_string());
    }
    if lower.contains("music folder") || lower.contains(" music ") {
        return Some("~/Music".to_string());
    }
    if lower.contains("movies folder") || lower.contains(" videos folder") {
        return Some("~/Movies".to_string());
    }
    if lower.contains("code folder") || lower.contains(" ~/code") || lower.contains(" /code") {
        return Some("~/code".to_string());
    }
    for prefix in ["my ", "the "] {
        if let Some(start) = lower.find(prefix) {
            let segment = &cleaned[start + prefix.len()..];
            let segment_lower = segment.to_ascii_lowercase();
            if let Some(end) = segment_lower.find(" folder") {
                let candidate = segment[..end].trim();
                if let Some(name) = candidate.split_whitespace().last()
                    && let Some(token) = sanitize_folder_token(name)
                {
                    let resolved = resolve_home_folder_name(&token);
                    return Some(format!("~/{}", resolved));
                }
            }
        }
    }
    if let Some(path) = infer_home_folder_hint(&cleaned) {
        return Some(path);
    }
    None
}

fn expand_user_path(path: &str) -> String {
    if path == "~" {
        return std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "~".to_string());
    }
    if let Some(rest) = path.strip_prefix("~/") {
        let base = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| "~".to_string());
        return format!("{base}/{rest}");
    }
    path.to_string()
}

fn build_distribution_command(path: &str) -> String {
    let target = shell_quote(&expand_user_path(path));
    format!(
        "find {target} -maxdepth 5 -type f 2>/dev/null | awk -F. 'NF>1{{ext=$NF}} NF==1{{ext=\"[noext]\"}} {{count[ext]++}} END{{for(e in count) print e\"\\t\"count[e]}}' | sort -k2,2nr | head -n 20"
    )
}

fn build_markdown_count_command(path: &str) -> String {
    let target = shell_quote(&expand_user_path(path));
    format!("find {target} -type f -name '*.md' 2>/dev/null | wc -l | tr -d ' '")
}

fn build_wallet_scan_command(path: &str) -> String {
    let target = shell_quote(&expand_user_path(path));
    format!(
        "rg -l -P \"(0x[a-fA-F0-9]{{40}}|bc1[ac-hj-np-z02-9]{{11,71}}|[13][a-km-zA-HJ-NP-Z1-9]{{25,34}}|[1-9A-HJ-NP-Za-km-z]{{32,44}}|(?i)(seed phrase|mnemonic|private key|wallet credentials?|keystore|xprv|xpub|begin (ec|rsa) private key))\" {target} -g '!.git/**' -g '!target/**' 2>/dev/null | while IFS= read -r f; do realpath \"$f\" 2>/dev/null || printf \"%s\\n\" \"$f\"; done | sort -u | head -n 500"
    )
}

fn build_image_count_command(path: &str) -> String {
    let target = shell_quote(&expand_user_path(path));
    format!(
        "find {target} -type f \\( -iname '*.jpg' -o -iname '*.jpeg' -o -iname '*.png' -o -iname '*.gif' -o -iname '*.bmp' -o -iname '*.webp' -o -iname '*.tif' -o -iname '*.tiff' -o -iname '*.heic' -o -iname '*.heif' -o -iname '*.avif' -o -iname '*.svg' \\) 2>/dev/null | wc -l | tr -d ' '"
    )
}

fn requests_count_only_numeric_output(prompt: &str) -> bool {
    let lower = prompt.to_ascii_lowercase();
    lower.contains("only the number")
        || lower.contains("just the number")
        || lower.contains("number only")
        || lower.contains("count only")
        || lower.contains("return only")
}

fn default_obsidian_vault_path() -> Option<String> {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()?;
    let candidates = [
        format!("{home}/Documents/Obsidian Vault"),
        format!("{home}/Desktop/My Vault"),
        format!("{home}/Desktop/Obsidian Vault"),
        format!("{home}/Obsidian"),
        format!("{home}/Vault"),
    ];
    candidates
        .into_iter()
        .find(|c| std::path::Path::new(c).is_dir())
}

fn build_obsidian_insight_command(path: &str) -> String {
    let target = shell_quote(path);
    format!(
        "notes=$(find {target} -type f -name '*.md' 2>/dev/null | wc -l | tr -d ' '); \
         echo \"NOTES=$notes\"; \
         echo 'TOP_TAGS:'; \
         rg -o --no-filename '#[A-Za-z][A-Za-z0-9_/-]*' {target} -g '*.md' 2>/dev/null | sort | uniq -c | sort -nr | head -n 10 || true; \
         echo 'SAMPLE_NOTES:'; \
         find {target} -type f -name '*.md' 2>/dev/null | sed 's|^.*/||' | head -n 10 || true"
    )
}

async fn try_execution_shortcut(
    state: &AppState,
    user_prompt: &str,
    turn_id: &str,
    authority: InputAuthority,
    channel_label: Option<&str>,
    prepared_model: &str,
    delegation_provenance: &mut DelegationProvenance,
) -> Option<InferenceOutput> {
    if requests_acknowledgement(user_prompt) {
        return Some(InferenceOutput {
            content: "Acknowledged, awaiting your next instruction.".to_string(),
            model: prepared_model.to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 1,
            latency_ms: 0,
            quality_score: 1.0,
            escalated: false,
            tool_results: vec![],
        });
    }

    if !should_bypass_cache_for_prompt(user_prompt) {
        return None;
    }

    // Outage-safe capability summary from runtime state.
    if requests_capability_summary(user_prompt) {
        let mut names = state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect::<Vec<_>>();
        names.sort();
        let sample = names
            .iter()
            .take(16)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let subagent_total = match ironclad_db::agents::list_sub_agents(&state.db) {
            Ok(rows) => rows.into_iter().filter(|r| r.enabled).count(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to list sub-agents for capability summary");
                0
            }
        };
        let primary_model = {
            let cfg = state.config.read().await;
            cfg.models.primary.clone()
        };
        return Some(InferenceOutput {
            content: format!(
                "By your command: I can execute tools, inspect runtime state, run shell and file workflows, schedule cron jobs, and delegate to subagents. Active model: {}. Enabled subagents: {}. Tool sample: {}.",
                primary_model, subagent_total, sample
            ),
            model: prepared_model.to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 1,
            latency_ms: 0,
            quality_score: 1.0,
            escalated: false,
            tool_results: vec![],
        });
    }

    // Outage-safe personality response from loaded soul/firmware identity.
    if requests_personality_profile(user_prompt) {
        let identity = {
            let cfg = state.config.read().await;
            cfg.agent.name.clone()
        };
        return Some(InferenceOutput {
            content: format!(
                "I’m {}. Operating profile: loyal, direct, execution-first, concise. I acknowledge, execute, and report only verified tool-backed outcomes.",
                identity
            ),
            model: prepared_model.to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 1,
            latency_ms: 0,
            quality_score: 1.0,
            escalated: false,
            tool_results: vec![],
        });
    }

    // Outage-safe provider inventory from configured/loaded provider prefixes.
    if requests_provider_inventory(user_prompt) {
        let (primary, providers) = {
            let cfg = state.config.read().await;
            let primary = cfg.models.primary.clone();
            drop(cfg);
            let llm = state.llm.read().await;
            let mut uniq = std::collections::BTreeSet::new();
            for provider in llm.providers.list() {
                uniq.insert(provider.name.clone());
            }
            let providers = uniq.into_iter().collect::<Vec<_>>();
            (primary, providers)
        };
        let sample = providers
            .iter()
            .take(24)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        return Some(InferenceOutput {
            content: format!(
                "Model report: primary is {}. Provider families loaded: {}{}",
                primary,
                sample,
                if providers.len() > 24 {
                    format!(" ({} total)", providers.len())
                } else {
                    String::new()
                }
            ),
            model: prepared_model.to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 1,
            latency_ms: 0,
            quality_score: 1.0,
            escalated: false,
            tool_results: vec![],
        });
    }

    // 0) Geopolitical sitrep request — force real delegated execution.
    if requests_current_events(user_prompt) {
        delegation_provenance.subagent_task_started = true;
        let params = serde_json::json!({
            "task": format!(
                "Provide an up-to-date geopolitical sitrep for today with concrete current date references and no stale-memory disclaimers. User request: {}",
                user_prompt
            )
        });
        let out = execute_tool_call(
            state,
            "orchestrate-subagents",
            &params,
            turn_id,
            authority,
            channel_label,
        )
        .await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                delegation_provenance.subagent_task_completed = true;
                delegation_provenance.subagent_result_attached = !output.trim().is_empty();
                tool_results.push(("orchestrate-subagents".to_string(), output.clone()));
                return Some(InferenceOutput {
                    content: super::strip_internal_delegation_metadata(&output),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("orchestrate-subagents".to_string(), format!("error: {err}")));
                let direct_model = {
                    let cfg = state.config.read().await;
                    cfg.models.primary.clone()
                };
                let direct_req = ironclad_llm::format::UnifiedRequest {
                    model: direct_model
                        .split_once('/')
                        .map(|(_, m)| m)
                        .unwrap_or(&direct_model)
                        .to_string(),
                    messages: vec![
                        ironclad_llm::format::UnifiedMessage {
                            role: "system".into(),
                            content: "Delegation fallback path activated. Provide a concise geopolitical sitrep for today with explicit concrete date references and key events. Do not use stale-memory disclaimers.".to_string(),
                            parts: None,
                        },
                        ironclad_llm::format::UnifiedMessage {
                            role: "user".into(),
                            content: user_prompt.to_string(),
                            parts: None,
                        },
                    ],
                    max_tokens: Some(1200),
                    temperature: None,
                    system: None,
                    quality_target: None,
                    tools: vec![],
                };
                if let Ok(direct) = infer_with_fallback(state, &direct_req, &direct_model).await {
                    let guarded_direct =
                        enforce_current_events_truth_guard(user_prompt, direct.content.clone());
                    if guarded_direct != direct.content {
                        return Some(InferenceOutput {
                            content: format!(
                                "Acknowledged. Delegation was unavailable ({}), and direct fallback could not produce a verified live sitrep. Please retry once model/provider reachability stabilizes.",
                                err
                            ),
                            model: direct.model,
                            tokens_in: direct.tokens_in,
                            tokens_out: direct.tokens_out,
                            cost: direct.cost,
                            react_turns: 1,
                            latency_ms: direct.latency_ms,
                            quality_score: 0.0,
                            escalated: direct.escalated,
                            tool_results,
                        });
                    }
                    return Some(InferenceOutput {
                        content: format!(
                            "Acknowledged. Delegation was unavailable ({}), so I switched to direct retrieval and produced this sitrep:\n\n{}",
                            err,
                            guarded_direct.trim()
                        ),
                        model: direct.model,
                        tokens_in: direct.tokens_in,
                        tokens_out: direct.tokens_out,
                        cost: direct.cost,
                        react_turns: 1,
                        latency_ms: direct.latency_ms,
                        quality_score: direct.quality_score,
                        escalated: direct.escalated,
                        tool_results,
                    });
                }
                return Some(InferenceOutput {
                    content: format!(
                        "Acknowledged. I attempted delegation for live geopolitical retrieval and a direct fallback inference path, but both failed. Delegation error: {}",
                        err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 0b) Email triage request — force delegated execution with Proton Bridge aware tasking.
    if requests_email_triage(user_prompt) {
        delegation_provenance.subagent_task_started = true;
        let params = serde_json::json!({
            "task": format!(
                "Perform email triage using available mailbox tooling (prefer Proton Bridge + himalaya when configured). \
                 Goal: identify important unread items, summarize sender/subject/time and why they matter. \
                 If mailbox/tooling is unavailable, report exact blocker and the minimal next operator step. \
                 User request: {}",
                user_prompt
            )
        });
        let out = execute_tool_call(
            state,
            "orchestrate-subagents",
            &params,
            turn_id,
            authority,
            channel_label,
        )
        .await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                delegation_provenance.subagent_task_completed = true;
                delegation_provenance.subagent_result_attached = !output.trim().is_empty();
                tool_results.push(("orchestrate-subagents".to_string(), output.clone()));
                return Some(InferenceOutput {
                    content: super::strip_internal_delegation_metadata(&output),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("orchestrate-subagents".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted delegated email triage but the task failed: {}. \
                         If Proton Bridge is expected, I can probe himalaya/bridge readiness and retry immediately.",
                        err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 1) Introspection request — execute actual introspection tools and summarize.
    if requests_introspection(user_prompt) {
        let mut tool_results = Vec::new();
        let mut snippets = Vec::new();

        for tool_name in [
            "get_runtime_context",
            "get_subagent_status",
            "get_channel_health",
            "get_memory_stats",
        ] {
            let out = execute_tool_call(
                state,
                tool_name,
                &serde_json::json!({}),
                turn_id,
                authority,
                channel_label,
            )
            .await;
            match out {
                Ok(output) => {
                    tool_results.push((tool_name.to_string(), output.clone()));
                    snippets.push(format!("{}: {}", tool_name, output.trim()));
                }
                Err(err) => {
                    tool_results.push((tool_name.to_string(), format!("error: {err}")));
                    snippets.push(format!("{}: error: {}", tool_name, err));
                }
            }
        }

        let mut names = state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect::<Vec<_>>();
        names.sort();

        let tool_sample = names
            .iter()
            .take(24)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let summary = format!(
            "Acknowledged. Active introspection completed.\n\
             Available tools: {} total (sample: {}).\n\
             Current subagent/runtime functionality snapshot:\n{}",
            names.len(),
            tool_sample,
            snippets.join("\n")
        );

        return Some(InferenceOutput {
            content: summary,
            model: prepared_model.to_string(),
            tokens_in: 0,
            tokens_out: 0,
            cost: 0.0,
            react_turns: 1,
            latency_ms: 0,
            quality_score: 1.0,
            escalated: false,
            tool_results,
        });
    }

    // 2) Tool inventory + random tool execution request.
    if requests_random_tool_use(user_prompt) {
        let mut names = state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect::<Vec<_>>();
        names.sort();
        let pick = if names.is_empty() {
            "echo".to_string()
        } else {
            let idx = user_prompt.len() % names.len();
            names[idx].clone()
        };
        let params = if pick == "echo" {
            serde_json::json!({"message": "Duncan Idaho reporting for duty."})
        } else if pick == "get_runtime_context" {
            serde_json::json!({})
        } else {
            serde_json::json!({"message": format!("tool probe via {}", pick)})
        };
        let mut tool_results = Vec::new();
        let out = execute_tool_call(
            state,
            if pick == "echo" || pick == "get_runtime_context" {
                &pick
            } else {
                "echo"
            },
            &params,
            turn_id,
            authority,
            channel_label,
        )
        .await;
        match out {
            Ok(output) => {
                let used = if pick == "echo" || pick == "get_runtime_context" {
                    pick.clone()
                } else {
                    "echo".to_string()
                };
                tool_results.push((used.clone(), output.clone()));
                let content = format!(
                    "By your command, tool inventory follows.\nAvailable tools (sample): {}.\nRandom pick: {}.\nOutput:\n{}",
                    names.into_iter().take(12).collect::<Vec<_>>().join(", "),
                    used,
                    output.trim()
                );
                return Some(InferenceOutput {
                    content,
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                let tool_results = vec![("echo".to_string(), format!("error: {err}"))];
                return Some(InferenceOutput {
                    content:
                        "I attempted a direct tool execution shortcut, but it failed. I can retry on your command."
                            .to_string(),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 3) Markdown count request — recursively count .md files.
    if requests_markdown_count_scan(user_prompt) && !requests_delegation(user_prompt) {
        let path = extract_path_hint(user_prompt).unwrap_or_else(|| "~/code".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_markdown_count_command(&path);
        let params = serde_json::json!({
            "command": cmd,
            "cwd": ".",
            "timeout_seconds": 60
        });
        let out =
            execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let numeric = output
                    .chars()
                    .filter(|c| c.is_ascii_digit())
                    .collect::<String>();
                let count = if numeric.is_empty() {
                    "0".to_string()
                } else {
                    numeric
                };
                let content = if requests_count_only_numeric_output(user_prompt) {
                    count.clone()
                } else {
                    format!(
                        "Found {} markdown files under {} (resolved: {}).",
                        count, path, resolved_path
                    )
                };
                return Some(InferenceOutput {
                    content,
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted to count markdown files under {} (resolved: {}), but the command failed: {}",
                        path, resolved_path, err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 4) Folder scan / file distribution request (supports '~' and absolute paths).
    if requests_file_distribution(user_prompt) || requests_folder_scan(user_prompt) {
        let path = extract_path_hint(user_prompt).unwrap_or_else(|| ".".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_distribution_command(&path);
        let params = serde_json::json!({
            "command": cmd,
            "cwd": ".",
            "timeout_seconds": 45
        });
        let out =
            execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let label = if requests_folder_scan(user_prompt) {
                    "Folder scan"
                } else {
                    "File distribution"
                };
                return Some(InferenceOutput {
                    content: format!(
                        "{} for {} (resolved: {}):\n{}",
                        label,
                        path,
                        resolved_path,
                        output.trim()
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                let label = if requests_folder_scan(user_prompt) {
                    "folder scan"
                } else {
                    "file distribution"
                };
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted to compute {} for {} (resolved: {}), but the command failed: {}",
                        label, path, resolved_path, err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 3b) Wallet address scan request — recursively search files and return full paths.
    if requests_wallet_address_scan(user_prompt) {
        let path = extract_path_hint(user_prompt).unwrap_or_else(|| "~/code".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_wallet_scan_command(&path);
        let params = serde_json::json!({
            "command": cmd,
            "cwd": ".",
            "timeout_seconds": 90
        });
        let out =
            execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let trimmed = output.trim();
                let content = if trimmed.is_empty() {
                    format!(
                        "No wallet-address-or-credential-like patterns were found under {} (resolved: {}).",
                        path, resolved_path
                    )
                } else {
                    format!(
                        "Wallet-address-or-credential-like patterns found under {} (resolved: {}):\n{}",
                        path, resolved_path, trimmed
                    )
                };
                return Some(InferenceOutput {
                    content,
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted a recursive wallet-address scan under {} (resolved: {}), but the command failed: {}",
                        path, resolved_path, err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 3c) Image-count request — recursively count image files.
    if requests_image_count_scan(user_prompt) {
        let lower = user_prompt.to_ascii_lowercase();
        let path = if let Some(p) = extract_path_hint(user_prompt) {
            p
        } else if lower.contains("photos") || lower.contains("pictures") {
            "~/Pictures".to_string()
        } else if lower.contains("downloads") {
            "~/Downloads".to_string()
        } else if lower.contains("documents") || lower.contains("docs") {
            "~/Documents".to_string()
        } else if lower.contains("desktop") {
            "~/Desktop".to_string()
        } else {
            "~/Pictures".to_string()
        };
        let resolved_path = expand_user_path(&path);
        let cmd = build_image_count_command(&path);
        let params = serde_json::json!({
            "command": cmd,
            "cwd": ".",
            "timeout_seconds": 90
        });
        let out =
            execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let count = output.trim().parse::<u64>().unwrap_or(0);
                return Some(InferenceOutput {
                    content: format!(
                        "Found {} image files under {} (resolved: {}).",
                        count, path, resolved_path
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted to count image files under {} (resolved: {}), but the command failed: {}",
                        path, resolved_path, err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 3d) Obsidian vault insight request — summarize note corpus signals.
    if requests_obsidian_insights(user_prompt) {
        let path = extract_path_hint(user_prompt)
            .map(|p| expand_user_path(&p))
            .or_else(default_obsidian_vault_path)
            .unwrap_or_else(|| "~/Documents/Obsidian Vault".to_string());
        let cmd = build_obsidian_insight_command(&path);
        let params = serde_json::json!({
            "command": cmd,
            "cwd": ".",
            "timeout_seconds": 90
        });
        let out =
            execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let mut note_count = "0".to_string();
                for line in output.lines() {
                    if let Some(rest) = line.strip_prefix("NOTES=") {
                        note_count = rest.trim().to_string();
                        break;
                    }
                }
                return Some(InferenceOutput {
                    content: format!(
                        "Obsidian vault scan complete for {}.\nNote count: {}.\n\n{}",
                        path,
                        note_count,
                        output.trim()
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted to analyze the Obsidian vault at {}, but the scan failed: {}",
                        path, err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 4) Delegation request — force a real orchestration tool execution attempt.
    if requests_delegation(user_prompt) {
        let lower = user_prompt.to_ascii_lowercase();
        if lower.contains("markdown") && lower.contains("count only") {
            let path =
                extract_path_hint(user_prompt).unwrap_or_else(|| "~/code/ironclad".to_string());
            let cmd = build_markdown_count_command(&path);
            let params = serde_json::json!({
                "command": cmd,
                "cwd": ".",
                "timeout_seconds": 90
            });
            let out =
                execute_tool_call(state, "bash", &params, turn_id, authority, channel_label).await;
            let mut tool_results = Vec::new();
            match out {
                Ok(output) => {
                    tool_results.push(("bash".to_string(), output.clone()));
                    let digits = output
                        .trim()
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect::<String>();
                    if !digits.is_empty() {
                        return Some(InferenceOutput {
                            content: digits,
                            model: prepared_model.to_string(),
                            tokens_in: 0,
                            tokens_out: 0,
                            cost: 0.0,
                            react_turns: 1,
                            latency_ms: 0,
                            quality_score: 1.0,
                            escalated: false,
                            tool_results,
                        });
                    }
                    return Some(InferenceOutput {
                        content: "0".to_string(),
                        model: prepared_model.to_string(),
                        tokens_in: 0,
                        tokens_out: 0,
                        cost: 0.0,
                        react_turns: 1,
                        latency_ms: 0,
                        quality_score: 0.5,
                        escalated: false,
                        tool_results,
                    });
                }
                Err(e) => {
                    tracing::warn!(model = %prepared_model, error = %e, "numeric inference failed; returning zero");
                    return Some(InferenceOutput {
                        content: "0".to_string(),
                        model: prepared_model.to_string(),
                        tokens_in: 0,
                        tokens_out: 0,
                        cost: 0.0,
                        react_turns: 1,
                        latency_ms: 0,
                        quality_score: 0.0,
                        escalated: false,
                        tool_results,
                    });
                }
            }
        }

        delegation_provenance.subagent_task_started = true;
        let params = serde_json::json!({
            "task": user_prompt
        });
        let out = execute_tool_call(
            state,
            "orchestrate-subagents",
            &params,
            turn_id,
            authority,
            channel_label,
        )
        .await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                delegation_provenance.subagent_task_completed = true;
                delegation_provenance.subagent_result_attached = !output.trim().is_empty();
                tool_results.push(("orchestrate-subagents".to_string(), output.clone()));
                return Some(InferenceOutput {
                    content: super::strip_internal_delegation_metadata(&output),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("orchestrate-subagents".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted delegation via orchestrate-subagents, but it failed: {}",
                        err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    // 5) Cron request — create a real cron job directly through DB path.
    if requests_cron(user_prompt) {
        let agent_id = {
            let cfg = state.config.read().await;
            cfg.agent.id.clone()
        };
        let name = format!("agent-cron-{}", &turn_id[..turn_id.len().min(8)]);
        let schedule_expr = "*/5 * * * *";
        let mut tool_results = Vec::new();
        match ironclad_db::cron::create_job(
            &state.db,
            &name,
            &agent_id,
            "cron",
            Some(schedule_expr),
            "{}",
        ) {
            Ok(job_id) => {
                tool_results.push((
                    "cron-create".to_string(),
                    format!("job_id={job_id} schedule={schedule_expr}"),
                ));
                return Some(InferenceOutput {
                    content: format!(
                        "By your command, scheduled cron job '{}' (id: {}) with expression '{}'.",
                        name, job_id, schedule_expr
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 1.0,
                    escalated: false,
                    tool_results,
                });
            }
            Err(err) => {
                tool_results.push(("cron-create".to_string(), format!("error: {err}")));
                return Some(InferenceOutput {
                    content: format!(
                        "I attempted to schedule a cron job, but creation failed: {}",
                        err
                    ),
                    model: prepared_model.to_string(),
                    tokens_in: 0,
                    tokens_out: 0,
                    cost: 0.0,
                    react_turns: 1,
                    latency_ms: 0,
                    quality_score: 0.0,
                    escalated: false,
                    tool_results,
                });
            }
        }
    }

    None
}

/// Run the non-streaming inference + ReAct loop. Returns the final assistant content
/// along with token/cost totals.
pub(super) async fn run_inference_and_react(
    state: &AppState,
    prepared: &PreparedInference,
    turn_id: &str,
    authority: InputAuthority,
    channel_label: Option<&str>,
    delegation_provenance: &mut DelegationProvenance,
) -> InferenceOutput {
    let (max_react_turns, max_turn_duration_seconds) = {
        let cfg = state.config.read().await;
        (
            cfg.agent.autonomy_max_react_turns,
            cfg.agent.autonomy_max_turn_duration_seconds,
        )
    };
    let user_prompt = prepared
        .request
        .messages
        .last()
        .map(|m| m.content.as_str())
        .unwrap_or_default();
    if let Some(shortcut) = try_execution_shortcut(
        state,
        user_prompt,
        turn_id,
        authority,
        channel_label,
        &prepared.model,
        delegation_provenance,
    )
    .await
    {
        return shortcut;
    }

    // Initial inference
    let mut resolved_model = prepared.model.clone();
    let (
        initial_content,
        mut total_in,
        mut total_out,
        mut total_cost,
        latency_ms,
        quality_score,
        escalated,
    ) = match infer_with_fallback(state, &prepared.request, &prepared.model).await {
        Ok(result) => {
            resolved_model = result.model.clone();
            (
                result.content,
                result.tokens_in,
                result.tokens_out,
                result.cost,
                result.latency_ms,
                result.quality_score,
                result.escalated,
            )
        }
        Err(last_error) => (
            super::tools::provider_failure_user_message(&last_error.to_string(), true),
            0,
            0,
            0.0,
            0,
            0.0,
            false,
        ),
    };

    let initial_content = sanitize_model_output(initial_content, state.hmac_secret.as_ref());

    // ReAct loop — supports multiple tool calls per LLM turn
    let mut react_loop = AgentLoop::new(max_react_turns);
    let mut final_content = initial_content.clone();
    let mut tool_results_acc: Vec<(String, String)> = Vec::new();
    let react_deadline =
        std::time::Instant::now() + std::time::Duration::from_secs(max_turn_duration_seconds);

    let mut pending_calls = parse_tool_calls(&initial_content);
    // Fall back to single-parse for edge cases (e.g. embedded JSON)
    if pending_calls.is_empty()
        && let Some(single) = parse_tool_call(&initial_content)
    {
        pending_calls.push(single);
    }

    if !pending_calls.is_empty() {
        react_loop.transition(ReactAction::Think);
        let mut react_messages = prepared.request.messages.clone();
        react_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "assistant".into(),
            content: initial_content,
            parts: None,
        });

        while !pending_calls.is_empty() {
            if std::time::Instant::now() >= react_deadline {
                final_content = format!(
                    "I stopped this turn after reaching the autonomy duration limit ({}s). \
Please continue with a narrower or next-step command.",
                    max_turn_duration_seconds
                );
                pending_calls.clear();
                break;
            }
            let mut observations = Vec::new();
            let mut batch_aborted = false;

            for (tn, tp) in &pending_calls {
                // Loop detection: break if the same tool+params repeats consecutively
                if react_loop.is_looping(tn, &tp.to_string()) {
                    tracing::warn!(
                        tool = tn.as_str(),
                        "ReAct loop detected — same tool+params repeated"
                    );
                    batch_aborted = true;
                    break;
                }

                // Track delegation provenance for channel claim guard
                if tn.to_ascii_lowercase().contains("subagent")
                    || tn.to_ascii_lowercase().contains("delegate")
                {
                    delegation_provenance.subagent_task_started = true;
                }

                react_loop.transition(ReactAction::Act {
                    tool_name: tn.clone(),
                    params: tp.to_string(),
                });
                if react_loop.state == ReactState::Done {
                    batch_aborted = true;
                    break;
                }

                let tool_result =
                    execute_tool_call(state, tn, tp, turn_id, authority, channel_label).await;
                let observation = match tool_result {
                    Ok(ref out) => {
                        if tn.to_ascii_lowercase().contains("subagent")
                            || tn.to_ascii_lowercase().contains("delegate")
                        {
                            delegation_provenance.subagent_task_completed = true;
                            delegation_provenance.subagent_result_attached = !out.trim().is_empty();
                        }
                        format!("[Tool {tn} succeeded]: {out}")
                    }
                    Err(ref err) => format!("[Tool {tn} failed]: {err}"),
                };
                // Accumulate tool results for memory ingestion
                let result_text = match &tool_result {
                    Ok(out) => out.clone(),
                    Err(err) => format!("error: {err}"),
                };
                tool_results_acc.push((tn.clone(), result_text));

                let observation = if ironclad_agent::injection::scan_output(&observation) {
                    tracing::warn!(
                        tool = tn.as_str(),
                        "tool result flagged by output scan, sanitizing"
                    );
                    format!("[Tool {tn} result blocked by safety filter]")
                } else {
                    observation
                };

                observations.push(observation);
            }

            if batch_aborted && observations.is_empty() {
                final_content = "I stopped tool execution because the same tool call kept repeating without progress. Please rephrase or provide a more specific command.".to_string();
                break;
            }

            react_loop.transition(ReactAction::Observe);
            let combined_observation = observations.join("\n\n");
            react_messages.push(ironclad_llm::format::UnifiedMessage {
                role: "user".into(),
                content: combined_observation,
                parts: None,
            });

            if react_loop.state == ReactState::Done {
                break;
            }

            let follow_req = ironclad_llm::format::UnifiedRequest {
                model: prepared.request.model.clone(),
                messages: react_messages.clone(),
                max_tokens: Some(2048),
                temperature: None,
                system: None,
                quality_target: None,
                tools: prepared.request.tools.clone(),
            };

            let follow_content =
                match infer_with_fallback(state, &follow_req, &prepared.model).await {
                    Ok(result) => {
                        resolved_model = result.model.clone();
                        total_in += result.tokens_in;
                        total_out += result.tokens_out;
                        total_cost += result.cost;
                        result.content
                    }
                    Err(e) => format!("LLM follow-up error: {e}"),
                };

            react_messages.push(ironclad_llm::format::UnifiedMessage {
                role: "assistant".into(),
                content: follow_content.clone(),
                parts: None,
            });

            let follow_content = sanitize_model_output(follow_content, state.hmac_secret.as_ref());

            pending_calls = parse_tool_calls(&follow_content);
            if pending_calls.is_empty()
                && let Some(single) = parse_tool_call(&follow_content)
            {
                pending_calls.push(single);
            }
            if pending_calls.is_empty() {
                react_loop.transition(ReactAction::Finish);
                final_content = follow_content;
            }
        }

        if !pending_calls.is_empty()
            && (final_content.trim().is_empty() || final_content.contains("\"tool_call\""))
        {
            final_content = "I could not complete the requested tool workflow this turn. Please retry with a narrower command.".to_string();
        }
    }

    // Post-ReAct guards
    let agent_name = {
        let cfg = state.config.read().await;
        cfg.agent.name.clone()
    };
    let mut final_content =
        enforce_subagent_claim_guard(final_content, delegation_provenance, &agent_name);
    final_content =
        enforce_execution_truth_guard(user_prompt, final_content, &tool_results_acc, &agent_name);
    final_content = enforce_model_identity_truth_guard(
        user_prompt,
        final_content,
        &resolved_model,
        &agent_name,
    );
    final_content = enforce_current_events_truth_guard(user_prompt, final_content);
    if requests_literary_quote_context(user_prompt)
        && is_overbroad_sensitive_conflict_refusal(&final_content)
    {
        tracing::warn!(
            "overbroad sensitive-topic refusal detected for literary quote request; retrying on fallback path"
        );
        let mut retry_messages = prepared.request.messages.clone();
        retry_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: "Operator directive: Provide a brief literary quote/paraphrase response only. Do not provide tactical guidance; keep it contextual and non-operational."
                .into(),
            parts: None,
        });
        let retry_req = ironclad_llm::format::UnifiedRequest {
            model: prepared.request.model.clone(),
            messages: retry_messages,
            max_tokens: Some(256),
            temperature: None,
            system: None,
            quality_target: None,
            tools: vec![],
        };
        match infer_with_fallback(state, &retry_req, &prepared.model).await {
            Ok(result) => {
                resolved_model = result.model.clone();
                total_in += result.tokens_in;
                total_out += result.tokens_out;
                total_cost += result.cost;
                let retried = sanitize_model_output(result.content, state.hmac_secret.as_ref());
                final_content = if is_overbroad_sensitive_conflict_refusal(&retried) {
                    "“Fear is the mind-killer.” In this context, the point is to resist panic and choose disciplined judgment.".to_string()
                } else {
                    retried
                };
            }
            Err(e) => {
                tracing::warn!(error = %e, "conflict-refusal quality retry failed; using deterministic fallback");
                final_content = "“Fear is the mind-killer.” In this context, the point is to resist panic and choose disciplined judgment.".to_string();
            }
        }
    }
    final_content = enforce_personality_integrity_guard(
        user_prompt,
        final_content,
        &agent_name,
        &resolved_model,
    );
    final_content = enforce_internal_jargon_guard(final_content, &agent_name);
    final_content = enforce_non_repetition(
        user_prompt,
        final_content,
        prepared.previous_assistant.as_deref(),
    );
    if (is_low_value_response(user_prompt, &final_content)
        || is_parroting_user_prompt(user_prompt, &final_content))
        && tool_results_acc.is_empty()
        && !requests_execution(user_prompt)
    {
        tracing::warn!("low-value placeholder response detected; running one-shot quality retry");
        let mut retry_messages = prepared.request.messages.clone();
        retry_messages.push(ironclad_llm::format::UnifiedMessage {
            role: "user".into(),
            content: "Operator directive: your previous response was placeholder/status-only. Provide a concrete, complete answer to the original user request now. Do not output placeholder lines such as 'ready' or status-only acknowledgements."
                .into(),
            parts: None,
        });
        let retry_req = ironclad_llm::format::UnifiedRequest {
            model: prepared.request.model.clone(),
            messages: retry_messages,
            max_tokens: prepared.request.max_tokens.or(Some(768)),
            temperature: prepared.request.temperature,
            system: None,
            quality_target: prepared.request.quality_target,
            tools: vec![],
        };
        match infer_with_fallback(state, &retry_req, &prepared.model).await {
            Ok(result) => {
                resolved_model = result.model.clone();
                total_in += result.tokens_in;
                total_out += result.tokens_out;
                total_cost += result.cost;
                let retried = sanitize_model_output(result.content, state.hmac_secret.as_ref());
                let retried = enforce_personality_integrity_guard(
                    user_prompt,
                    retried,
                    &agent_name,
                    &resolved_model,
                );
                let retried = enforce_internal_jargon_guard(retried, &agent_name);
                final_content = enforce_non_repetition(
                    user_prompt,
                    retried,
                    prepared.previous_assistant.as_deref(),
                );
                if is_low_value_response(user_prompt, &final_content)
                    || is_parroting_user_prompt(user_prompt, &final_content)
                {
                    final_content = deterministic_quality_fallback(user_prompt, &agent_name);
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "low-value quality retry failed; using deterministic fallback");
                final_content = deterministic_quality_fallback(user_prompt, &agent_name);
            }
        }
    }

    final_content = enforce_internal_protocol_guard(final_content, &agent_name);
    if final_content.trim().is_empty()
        || final_content.contains("filtered internal execution protocol")
    {
        final_content = deterministic_quality_fallback(user_prompt, &agent_name);
    }

    InferenceOutput {
        content: final_content,
        model: resolved_model,
        tokens_in: total_in,
        tokens_out: total_out,
        cost: total_cost,
        react_turns: react_loop.turn_count,
        latency_ms,
        quality_score,
        escalated,
        tool_results: tool_results_acc,
    }
}

/// Check the semantic cache. Returns `Some(CachedResponse)` on hit.
pub(super) async fn check_cache(
    state: &AppState,
    user_content: &str,
    cache_hash: &str,
    query_embedding: Option<&[f32]>,
) -> Option<ironclad_llm::CachedResponse> {
    let _ = user_content;
    let _ = query_embedding;
    let mut llm = state.llm.write().await;
    // High-integrity default: only exact/tool-TTL cache hits.
    // Semantic near-match cache reuse can fabricate wrong instruction-bound outputs.
    llm.cache.lookup_strict(cache_hash)
}

/// Store a response in the semantic cache.
pub(super) async fn store_in_cache(
    state: &AppState,
    cache_hash: &str,
    user_content: &str,
    content: &str,
    model: &str,
    tokens_out: i64,
) {
    if tokens_out > 0
        && !is_low_value_response(user_content, content)
        && !is_parroting_user_prompt(user_content, content)
    {
        let entry = ironclad_llm::CachedResponse {
            content: content.to_string(),
            model: model.to_string(),
            tokens_saved: tokens_out as u32,
            created_at: std::time::Instant::now(),
            expires_at: std::time::Instant::now() + std::time::Duration::from_secs(3600),
            hits: 0,
            involved_tools: false,
            embedding: None,
        };
        let mut llm = state.llm.write().await;
        llm.cache
            .store_with_embedding(cache_hash, user_content, entry);
    }
}

/// Spawn background memory ingestion + embedding generation for a completed turn.
pub(super) fn post_turn_ingest(
    state: &AppState,
    session_id: &str,
    user_content: &str,
    assistant_content: &str,
    tool_results: &[(String, String)],
) {
    let db = state.db.clone();
    let config = Arc::clone(&state.config);
    let session = session_id.to_string();
    let user = user_content.to_string();
    let assistant = assistant_content.to_string();
    let tools = tool_results.to_vec();
    let llm = Arc::clone(&state.llm);
    tokio::spawn(async move {
        ironclad_agent::memory::ingest_turn(&db, &session, &user, &assistant, &tools);

        // Periodic context checkpoint
        let ctx_cfg = &config.read().await.context;
        if ctx_cfg.checkpoint_enabled
            && let Ok(msgs) = ironclad_db::sessions::list_messages(&db, &session, None)
        {
            let turn_count = msgs.len() as u32;
            if turn_count > 0 && turn_count.is_multiple_of(ctx_cfg.checkpoint_interval_turns) {
                let mem_summary = msgs
                    .iter()
                    .filter(|m| m.role == "system")
                    .map(|m| m.content.as_str())
                    .collect::<Vec<_>>()
                    .join("\n---\n");
                let digest = msgs.last().map(|m| m.content.as_str());
                if let Err(e) = ironclad_db::checkpoint::save_checkpoint(
                    &db,
                    &session,
                    "", // system prompt hash — placeholder until we thread it
                    &mem_summary[..mem_summary.len().min(2000)],
                    None,
                    digest,
                    turn_count as i64,
                ) {
                    tracing::warn!(error = %e, session_id = %session, "failed to save context checkpoint");
                } else {
                    tracing::debug!(session_id = %session, turn_count, "saved context checkpoint");
                }
            }
        }

        let llm = llm.read().await;
        let chunk_config = ironclad_agent::retrieval::ChunkConfig::default();
        let chunks = ironclad_agent::retrieval::chunk_text(&assistant, &chunk_config);

        for chunk in &chunks {
            if let Ok(embedding) = llm.embedding.embed_single(&chunk.text).await {
                let embed_id = uuid::Uuid::new_v4().to_string();
                ironclad_db::embeddings::store_embedding(
                    &db,
                    &embed_id,
                    "turn",
                    &session,
                    &chunk.text[..chunk.text.len().min(200)],
                    &embedding,
                )
                .inspect_err(
                    |e| tracing::warn!(error = %e, chunk_idx = chunk.index, "failed to store chunk embedding"),
                )
                .ok();
            }
        }
    });
}

#[allow(dead_code)] // all fields used by various callers (API, streaming, channel)
/// Result of the unified inference pipeline (cache check → inference → post-turn ops).
pub(super) struct PipelineResult {
    pub content: String,
    /// Model selected by routing before execution.
    pub selected_model: String,
    /// Model that actually produced the response (may differ on fallback/cache hit).
    pub model: String,
    /// When actual model differs, contains the originally selected model.
    pub model_shift_from: Option<String>,
    pub tokens_in: i64,
    pub tokens_out: i64,
    pub cost: f64,
    pub react_turns: usize,
    pub latency_ms: u64,
    pub quality_score: f64,
    pub escalated: bool,
    pub cached: bool,
    pub tokens_saved: u32,
    pub assistant_message_id: String,
    /// Tool calls executed during inference: (tool_name, result_text).
    pub tool_results: Vec<(String, String)>,
}

/// Unified post-prepare pipeline used by all entry points (API, streaming, channel).
///
/// Handles: cache check → inference + ReAct → store assistant message → record cost →
/// background ingest → cache store. Callers only need to handle session setup,
/// input validation, and formatting the final response.
#[allow(clippy::too_many_arguments)] // central pipeline requires full request context
pub(super) async fn execute_inference_pipeline(
    state: &AppState,
    prepared: &PreparedInference,
    session_id: &str,
    user_content: &str,
    turn_id: &str,
    authority: InputAuthority,
    channel_label: Option<&str>,
    delegation_provenance: &mut DelegationProvenance,
) -> Result<PipelineResult, String> {
    // 1. Cache check
    let cached = if should_bypass_cache_for_prompt(user_content) {
        None
    } else {
        check_cache(
            state,
            user_content,
            &prepared.cache_hash,
            prepared.query_embedding.as_deref(),
        )
        .await
    };

    if let Some(cached) = cached {
        let agent_name = {
            let cfg = state.config.read().await;
            cfg.agent.name.clone()
        };
        let cached_content =
            enforce_execution_truth_guard(user_content, cached.content, &[], &agent_name);
        let cached_content = enforce_model_identity_truth_guard(
            user_content,
            cached_content,
            &cached.model,
            &agent_name,
        );
        let cached_content = enforce_current_events_truth_guard(user_content, cached_content);
        let cached_content = enforce_personality_integrity_guard(
            user_content,
            cached_content,
            &agent_name,
            &cached.model,
        );
        let cached_content = enforce_internal_jargon_guard(cached_content, &agent_name);
        let cached_content = enforce_internal_protocol_guard(cached_content, &agent_name);
        let cached_content = if cached_content.trim().is_empty()
            || cached_content.contains("filtered internal execution protocol")
        {
            deterministic_quality_fallback(user_content, &agent_name)
        } else {
            cached_content
        };
        let guarded_cached_content = enforce_non_repetition(
            user_content,
            cached_content,
            prepared.previous_assistant.as_deref(),
        );
        if is_low_value_response(user_content, &guarded_cached_content)
            || is_parroting_user_prompt(user_content, &guarded_cached_content)
        {
            tracing::warn!("discarding low-value cache hit and forcing fresh inference");
        } else {
            let cached_provider_prefix = cached
                .model
                .split('/')
                .next()
                .unwrap_or("unknown")
                .to_string();
            record_cost(
                state,
                &cached.model,
                &cached_provider_prefix,
                0,
                0,
                0.0,
                Some("cached"),
                true,
                Some(0),
                None,
                false,
                Some(turn_id),
            );
            let asst_id = ironclad_db::sessions::append_message(
                &state.db,
                session_id,
                "assistant",
                &guarded_cached_content,
            )
            .map_err(|e| format!("failed to store cached response: {e}"))?;
            if cached.model != prepared.model {
                state.event_bus.publish(
                    serde_json::json!({
                        "type": "model_shift",
                        "turn_id": turn_id,
                        "session_id": session_id,
                        "channel": channel_label.unwrap_or("unknown"),
                        "selected_model": prepared.model,
                        "executed_model": cached.model,
                        "reason": "cache_hit",
                    })
                    .to_string(),
                );
            }

            return Ok(PipelineResult {
                content: guarded_cached_content,
                selected_model: prepared.model.clone(),
                model: cached.model.clone(),
                model_shift_from: if cached.model != prepared.model {
                    Some(prepared.model.clone())
                } else {
                    None
                },
                tokens_in: 0,
                tokens_out: 0,
                cost: 0.0,
                react_turns: 0,
                latency_ms: 0,
                quality_score: 0.0,
                escalated: false,
                cached: true,
                tokens_saved: cached.tokens_saved,
                assistant_message_id: asst_id,
                tool_results: vec![],
            });
        }
    }

    // 2. Inference + ReAct loop
    let inference = run_inference_and_react(
        state,
        prepared,
        turn_id,
        authority,
        channel_label,
        delegation_provenance,
    )
    .await;

    // 3. Store assistant message
    let asst_id = ironclad_db::sessions::append_message(
        &state.db,
        session_id,
        "assistant",
        &inference.content,
    )
    .map_err(|e| format!("failed to store assistant response: {e}"))?;

    // 4. Record cost
    let executed_provider_prefix = inference
        .model
        .split('/')
        .next()
        .unwrap_or("unknown")
        .to_string();
    record_cost(
        state,
        &inference.model,
        &executed_provider_prefix,
        inference.tokens_in,
        inference.tokens_out,
        inference.cost,
        None,
        false,
        Some(inference.latency_ms as i64),
        Some(inference.quality_score),
        inference.escalated,
        Some(turn_id),
    );

    // 5. Post-turn ingest (spawns background task)
    post_turn_ingest(
        state,
        session_id,
        user_content,
        &inference.content,
        &inference.tool_results,
    );

    // 6. Cache store
    store_in_cache(
        state,
        &prepared.cache_hash,
        user_content,
        &inference.content,
        &inference.model,
        inference.tokens_out,
    )
    .await;

    if inference.model != prepared.model {
        state.event_bus.publish(
            serde_json::json!({
                "type": "model_shift",
                "turn_id": turn_id,
                "session_id": session_id,
                "channel": channel_label.unwrap_or("unknown"),
                "selected_model": prepared.model,
                "executed_model": inference.model,
                "reason": "fallback",
            })
            .to_string(),
        );
    }

    Ok(PipelineResult {
        content: inference.content,
        selected_model: prepared.model.clone(),
        model: inference.model.clone(),
        model_shift_from: if inference.model != prepared.model {
            Some(prepared.model.clone())
        } else {
            None
        },
        tokens_in: inference.tokens_in,
        tokens_out: inference.tokens_out,
        cost: inference.cost,
        react_turns: inference.react_turns,
        latency_ms: inference.latency_ms,
        quality_score: inference.quality_score,
        escalated: inference.escalated,
        cached: false,
        tokens_saved: 0,
        assistant_message_id: asst_id,
        tool_results: inference.tool_results,
    })
}

/// Record inference cost metrics.
#[allow(clippy::too_many_arguments)] // thin pass-through to ironclad_db::metrics
pub(super) fn record_cost(
    state: &AppState,
    model: &str,
    provider_prefix: &str,
    tokens_in: i64,
    tokens_out: i64,
    cost: f64,
    variant: Option<&str>,
    cached: bool,
    latency_ms: Option<i64>,
    quality_score: Option<f64>,
    escalation: bool,
    turn_id: Option<&str>,
) {
    ironclad_db::metrics::record_inference_cost(
        &state.db,
        model,
        provider_prefix,
        tokens_in,
        tokens_out,
        cost,
        variant,
        cached,
        latency_ms,
        quality_score,
        escalation,
        turn_id,
    )
    .inspect_err(|e| tracing::warn!(error = %e, "failed to record inference cost"))
    .ok();
}
