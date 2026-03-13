// Phase 4: ShortcutDispatcher is wired into run_inference_and_react() in core.rs.
// Helper functions duplicated from core.rs will be consolidated in Phase 6.
//! Unified shortcut dispatcher — replaces the 983-line `try_execution_shortcut()` god function.
//!
//! Each pre-LLM shortcut is a standalone `ShortcutHandler` struct registered in the
//! `ShortcutDispatcher`.  The dispatcher iterates handlers in priority order, skipping
//! shortcuts when:
//!   - `is_correction_turn` is true (corrections always reach the LLM), or
//!   - `requires_cache_bypass()` is true but no cache-bypass intent is present.

use ironclad_core::InputAuthority;

use super::AppState;
use super::core::InferenceOutput;
use super::decomposition::DelegationProvenance;
use super::intent_registry::Intent;

// ── Context ──────────────────────────────────────────────────────────────

/// Everything a shortcut handler needs to execute.
pub(super) struct ShortcutContext<'a> {
    pub state: &'a AppState,
    pub user_content: &'a str,
    pub turn_id: &'a str,
    pub intents: &'a [Intent],
    pub agent_name: &'a str,
    pub channel_label: &'a str,
    pub prepared_model: &'a str,
    pub authority: InputAuthority,
    pub delegation_provenance: &'a mut DelegationProvenance,
    pub is_correction_turn: bool,
}

/// Build a zero-cost `InferenceOutput` with the shortcut's content and quality.
fn shortcut_output(
    model: &str,
    content: String,
    quality: f64,
    tool_results: Vec<(String, String)>,
) -> InferenceOutput {
    InferenceOutput {
        content,
        model: model.to_string(),
        tokens_in: 0,
        tokens_out: 0,
        cost: 0.0,
        react_turns: 1,
        latency_ms: 0,
        quality_score: quality,
        escalated: false,
        tool_results,
    }
}

// ── Trait & Dispatcher ───────────────────────────────────────────────────

#[async_trait::async_trait]
pub(super) trait ShortcutHandler: Send + Sync {
    /// Check if this handler should fire given the classified intents.
    fn handles(&self, intents: &[Intent]) -> bool;

    /// Whether the handler requires at least one cache-bypass intent.
    /// Default: `true`.  Override to `false` for shortcuts like Acknowledgement
    /// that should fire even when no cache-bypass intent is present.
    fn requires_cache_bypass(&self) -> bool {
        true
    }

    /// Execute the shortcut.  Returns `Ok(Some(...))` to short-circuit the pipeline,
    /// `Ok(None)` to fall through to the next handler, or `Err(...)` on failure.
    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String>;
}

/// Registry of all shortcut handlers, iterated in priority order.
pub(super) struct ShortcutDispatcher {
    handlers: Vec<Box<dyn ShortcutHandler>>,
}

impl ShortcutDispatcher {
    /// Build the default dispatcher with all 15 handlers in priority order.
    pub fn default_dispatcher() -> Self {
        Self {
            handlers: vec![
                // Always-eligible (no cache bypass required)
                Box::new(AcknowledgementShortcut),
                // Static responses from runtime state
                Box::new(CapabilitySummaryShortcut),
                Box::new(PersonalityProfileShortcut),
                Box::new(ProviderInventoryShortcut),
                // Delegation-backed shortcuts (before generic delegation)
                Box::new(CurrentEventsShortcut),
                Box::new(EmailTriageShortcut),
                // Tool-calling shortcuts
                Box::new(IntrospectionShortcut),
                Box::new(RandomToolUseShortcut),
                // Bash-command shortcuts
                Box::new(MarkdownCountScanShortcut),
                Box::new(FolderScanShortcut),
                Box::new(WalletAddressScanShortcut),
                Box::new(ImageCountScanShortcut),
                Box::new(ObsidianInsightsShortcut),
                // Generic delegation (catches remaining delegation intents)
                Box::new(DelegationShortcut),
                // Cron (lowest priority among shortcuts)
                Box::new(CronShortcut),
            ],
        }
    }

    /// Try to dispatch a shortcut.
    ///
    /// `bypass_cache`: whether the classified intents include at least one
    /// cache-bypass intent (from `IntentRegistry::should_bypass_cache()`).
    ///
    /// Returns `Ok(None)` when no shortcut matches.
    pub async fn try_dispatch(
        &self,
        ctx: &mut ShortcutContext<'_>,
        bypass_cache: bool,
    ) -> Result<Option<InferenceOutput>, String> {
        if ctx.is_correction_turn {
            return Ok(None);
        }
        for handler in &self.handlers {
            if !handler.handles(ctx.intents) {
                continue;
            }
            if handler.requires_cache_bypass() && !bypass_cache {
                continue;
            }
            if let result @ Some(_) = handler.execute(ctx).await? {
                return Ok(result);
            }
        }
        Ok(None)
    }
}

// ── Common execution helpers ─────────────────────────────────────────────

/// Execute a bash command through the tool layer and return raw output.
async fn execute_bash_tool(
    ctx: &ShortcutContext<'_>,
    command: &str,
    timeout_seconds: u32,
) -> Result<String, String> {
    let params = serde_json::json!({
        "command": command,
        "cwd": ".",
        "timeout_seconds": timeout_seconds,
    });
    super::tools::execute_tool_call(
        ctx.state,
        "bash",
        &params,
        ctx.turn_id,
        ctx.authority,
        Some(ctx.channel_label),
    )
    .await
}

/// Execute a delegation task through orchestrate-subagents and update provenance.
async fn execute_delegation_tool(
    ctx: &mut ShortcutContext<'_>,
    task: &str,
) -> (Result<String, String>, Vec<(String, String)>) {
    ctx.delegation_provenance.subagent_task_started = true;
    let params = serde_json::json!({ "task": task });
    let out = super::tools::execute_tool_call(
        ctx.state,
        "orchestrate-subagents",
        &params,
        ctx.turn_id,
        ctx.authority,
        Some(ctx.channel_label),
    )
    .await;
    let mut tool_results = Vec::new();
    match &out {
        Ok(output) => {
            ctx.delegation_provenance.subagent_task_completed = true;
            ctx.delegation_provenance.subagent_result_attached = !output.trim().is_empty();
            tool_results.push(("orchestrate-subagents".to_string(), output.clone()));
        }
        Err(err) => {
            tool_results.push(("orchestrate-subagents".to_string(), format!("error: {err}")));
        }
    }
    (out, tool_results)
}

// ── Path / shell helpers (duplicated from core.rs, consolidate in Phase 6) ──

fn shell_quote(raw: &str) -> String {
    format!("'{}'", raw.replace('\'', "'\"'\"'"))
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

fn sanitize_folder_token(raw: &str) -> Option<String> {
    let cleaned = raw.trim_matches(|c: char| ",.;:!?\"'`()[]{}".contains(c));
    if cleaned.is_empty() {
        return None;
    }
    if !cleaned
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
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
            if !matches!(
                prev.to_ascii_lowercase().as_str(),
                "my" | "the" | "a" | "an"
            ) {
                candidate = Some(prev);
            }
        }
        if candidate.is_none() && idx >= 2 {
            let prev_prev = tokens[idx - 2].as_str();
            if !matches!(
                prev_prev.to_ascii_lowercase().as_str(),
                "my" | "the" | "a" | "an"
            ) {
                candidate = Some(prev_prev);
            }
        }
        if let Some(name) = candidate {
            let resolved = resolve_home_folder_name(name);
            return Some(format!("~/{resolved}"));
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
                    return Some(format!("~/{resolved}"));
                }
            }
        }
    }
    if let Some(path) = infer_home_folder_hint(&cleaned) {
        return Some(path);
    }
    None
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

// ── Build-command helpers ────────────────────────────────────────────────

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

// ── Introspection prose formatter ────────────────────────────────────────

fn truncate_for_channel(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}…", &s[..max_len])
    }
}

fn format_introspection_prose(tool_name: &str, raw_json: &str) -> String {
    let v: serde_json::Value = match serde_json::from_str(raw_json) {
        Ok(v) => v,
        Err(_) => return format!("{tool_name}: {}", truncate_for_channel(raw_json, 200)),
    };

    match tool_name {
        "get_runtime_context" => {
            let agent = v["agent_id"].as_str().unwrap_or("unknown");
            let session = v["session_id"].as_str().unwrap_or("unknown");
            let channel = v["channel"].as_str().unwrap_or("unknown");
            let workspace = v["workspace_root"].as_str().unwrap_or("unknown");
            let authority = v["authority"].as_str().unwrap_or("unknown");
            format!(
                "Runtime: agent \"{agent}\", session {session}, channel {channel}, \
                 workspace {workspace}, authority {authority}."
            )
        }
        "get_memory_stats" => {
            let method = v["retrieval_method"].as_str().unwrap_or("hybrid retrieval");
            let mut tier_parts = Vec::new();
            if let Some(tiers) = v["tiers"].as_object() {
                let mut entries: Vec<_> = tiers
                    .iter()
                    .filter_map(|(name, obj)| {
                        let pct = obj["budget_pct"].as_u64()?;
                        Some((name.clone(), pct))
                    })
                    .collect();
                entries.sort_by(|a, b| b.1.cmp(&a.1));
                for (name, pct) in entries {
                    tier_parts.push(format!("{name} {pct}%"));
                }
            }
            if tier_parts.is_empty() {
                format!("Memory: {method}.")
            } else {
                format!("Memory: {method} — {}.", tier_parts.join(", "))
            }
        }
        "get_channel_health" => {
            let channel = v["channel"].as_str().unwrap_or("unknown");
            let status = v["status"].as_str().unwrap_or("unknown");
            format!("Channel health: {channel} is {status}.")
        }
        "get_subagent_status" => {
            let sub_count = v["subagent_count"].as_u64().unwrap_or(0);
            let task_count = v["open_task_count"].as_u64().unwrap_or(0);
            let mut parts = Vec::new();

            if sub_count == 0 {
                parts.push("No subagents registered".to_string());
            } else {
                let names: Vec<String> = v["subagents"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .filter_map(|s| {
                                let name =
                                    s["display_name"].as_str().or_else(|| s["name"].as_str())?;
                                let enabled = s["enabled"].as_bool().unwrap_or(false);
                                Some(if enabled {
                                    name.to_string()
                                } else {
                                    format!("{name} (disabled)")
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                parts.push(format!(
                    "{sub_count} subagent{}: {}",
                    if sub_count == 1 { "" } else { "s" },
                    names.join(", ")
                ));
            }

            if task_count == 0 {
                parts.push("no open tasks".to_string());
            } else {
                let task_titles: Vec<String> = v["tasks"]
                    .as_array()
                    .map(|arr| {
                        arr.iter()
                            .take(5)
                            .filter_map(|t| {
                                let title = t["title"].as_str()?;
                                let status = t["status"].as_str().unwrap_or("pending");
                                Some(format!("\"{title}\" ({status})"))
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let suffix = if task_count > 5 {
                    format!(" (+{} more)", task_count - 5)
                } else {
                    String::new()
                };
                parts.push(format!(
                    "{task_count} open task{}: {}{suffix}",
                    if task_count == 1 { "" } else { "s" },
                    task_titles.join(", "),
                ));
            }

            format!("Subagents & tasks: {}.", parts.join("; "))
        }
        _ => format!(
            "{tool_name}: {}",
            truncate_for_channel(raw_json.trim(), 200)
        ),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Handler Implementations
// ═══════════════════════════════════════════════════════════════════════════

// ── 1. Acknowledgement (always eligible, no cache bypass) ────────────────

pub(super) struct AcknowledgementShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for AcknowledgementShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::Acknowledgement)
    }

    fn requires_cache_bypass(&self) -> bool {
        false
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        Ok(Some(shortcut_output(
            ctx.prepared_model,
            "Acknowledged, awaiting your next instruction.".to_string(),
            1.0,
            vec![],
        )))
    }
}

// ── 2. Capability Summary ────────────────────────────────────────────────

pub(super) struct CapabilitySummaryShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for CapabilitySummaryShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::CapabilitySummary)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let mut names: Vec<String> = ctx
            .state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        names.sort();
        let sample = names
            .iter()
            .take(16)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let subagent_total = match ironclad_db::agents::list_sub_agents(&ctx.state.db) {
            Ok(rows) => rows.into_iter().filter(|r| r.enabled).count(),
            Err(e) => {
                tracing::warn!(error = %e, "failed to list sub-agents for capability summary");
                0
            }
        };
        let primary_model = {
            let cfg = ctx.state.config.read().await;
            cfg.models.primary.clone()
        };
        Ok(Some(shortcut_output(
            ctx.prepared_model,
            format!(
                "By your command: I can execute tools, inspect runtime state, run shell and file workflows, schedule cron jobs, and delegate to subagents. Active model: {}. Enabled subagents: {}. Tool sample: {}.",
                primary_model, subagent_total, sample
            ),
            1.0,
            vec![],
        )))
    }
}

// ── 3. Personality Profile ───────────────────────────────────────────────

pub(super) struct PersonalityProfileShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for PersonalityProfileShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::PersonalityProfile)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let identity = {
            let cfg = ctx.state.config.read().await;
            cfg.agent.name.clone()
        };
        Ok(Some(shortcut_output(
            ctx.prepared_model,
            format!(
                "I'm {}. Operating profile: loyal, direct, execution-first, concise. I acknowledge, execute, and report only verified tool-backed outcomes.",
                identity
            ),
            1.0,
            vec![],
        )))
    }
}

// ── 4. Provider Inventory ────────────────────────────────────────────────

pub(super) struct ProviderInventoryShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for ProviderInventoryShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::ProviderInventory)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let (primary, providers) = {
            let cfg = ctx.state.config.read().await;
            let primary = cfg.models.primary.clone();
            drop(cfg);
            let llm = ctx.state.llm.read().await;
            let mut uniq = std::collections::BTreeSet::new();
            for provider in llm.providers.list() {
                uniq.insert(provider.name.clone());
            }
            (primary, uniq.into_iter().collect::<Vec<_>>())
        };
        let sample = providers
            .iter()
            .take(24)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");
        let suffix = if providers.len() > 24 {
            format!(" ({} total)", providers.len())
        } else {
            String::new()
        };
        Ok(Some(shortcut_output(
            ctx.prepared_model,
            format!(
                "Model report: primary is {}. Provider families loaded: {}{suffix}",
                primary, sample
            ),
            1.0,
            vec![],
        )))
    }
}

// ── 5. Current Events (delegation + direct fallback) ─────────────────────

pub(super) struct CurrentEventsShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for CurrentEventsShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::CurrentEvents)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let task = format!(
            "Provide an up-to-date geopolitical sitrep for today with concrete current date references and no stale-memory disclaimers. User request: {}",
            ctx.user_content
        );
        let (result, mut tool_results) = execute_delegation_tool(ctx, &task).await;
        match result {
            Ok(output) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                super::guards::strip_internal_delegation_metadata(&output),
                1.0,
                tool_results,
            ))),
            Err(err) => {
                // Fallback: direct LLM inference for sitrep
                let direct_model = {
                    let cfg = ctx.state.config.read().await;
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
                            content: ctx.user_content.to_string(),
                            parts: None,
                        },
                    ],
                    max_tokens: Some(1200),
                    temperature: None,
                    system: None,
                    quality_target: None,
                    tools: vec![],
                };
                if let Ok(direct) =
                    super::routing::infer_with_fallback(ctx.state, &direct_req, &direct_model).await
                {
                    let guarded = super::guards::enforce_current_events_truth_guard(
                        ctx.user_content,
                        direct.content.clone(),
                    );
                    if guarded != direct.content {
                        return Ok(Some(InferenceOutput {
                            content: format!(
                                "Acknowledged. Delegation was unavailable ({err}), and direct fallback could not produce a verified live sitrep. Please retry once model/provider reachability stabilizes."
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
                        }));
                    }
                    return Ok(Some(InferenceOutput {
                        content: format!(
                            "Acknowledged. Delegation was unavailable ({err}), so I switched to direct retrieval and produced this sitrep:\n\n{}",
                            guarded.trim()
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
                    }));
                }
                // Both delegation and direct fallback failed
                tool_results.push(("direct-inference".to_string(), "failed".to_string()));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "Acknowledged. I attempted delegation for live geopolitical retrieval and a direct fallback inference path, but both failed. Delegation error: {err}"
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 6. Email Triage (delegation) ─────────────────────────────────────────

pub(super) struct EmailTriageShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for EmailTriageShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::EmailTriage)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let task = format!(
            "Perform email triage using available mailbox tooling (prefer Proton Bridge + himalaya when configured). \
             Goal: identify important unread items, summarize sender/subject/time and why they matter. \
             If mailbox/tooling is unavailable, report exact blocker and the minimal next operator step. \
             User request: {}",
            ctx.user_content
        );
        let (result, tool_results) = execute_delegation_tool(ctx, &task).await;
        match result {
            Ok(output) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                super::guards::strip_internal_delegation_metadata(&output),
                1.0,
                tool_results,
            ))),
            Err(err) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                format!(
                    "I attempted delegated email triage but the task failed: {err}. \
                     If Proton Bridge is expected, I can probe himalaya/bridge readiness and retry immediately."
                ),
                0.0,
                tool_results,
            ))),
        }
    }
}

// ── 7. Introspection (multi-tool) ────────────────────────────────────────

pub(super) struct IntrospectionShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for IntrospectionShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::Introspection)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let mut tool_results = Vec::new();
        let mut prose_lines = Vec::new();

        for tool_name in [
            "get_runtime_context",
            "get_subagent_status",
            "get_channel_health",
            "get_memory_stats",
        ] {
            let out = super::tools::execute_tool_call(
                ctx.state,
                tool_name,
                &serde_json::json!({}),
                ctx.turn_id,
                ctx.authority,
                Some(ctx.channel_label),
            )
            .await;
            match out {
                Ok(output) => {
                    prose_lines.push(format_introspection_prose(tool_name, &output));
                    tool_results.push((tool_name.to_string(), output));
                }
                Err(err) => {
                    tool_results.push((tool_name.to_string(), format!("error: {err}")));
                    prose_lines.push(format!("{tool_name}: error — {err}"));
                }
            }
        }

        let mut names: Vec<String> = ctx
            .state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        names.sort();
        let tool_sample = names
            .iter()
            .take(24)
            .cloned()
            .collect::<Vec<_>>()
            .join(", ");

        Ok(Some(shortcut_output(
            ctx.prepared_model,
            format!(
                "Introspection complete.\nAvailable tools: {} total (sample: {}).\n{}",
                names.len(),
                tool_sample,
                prose_lines.join("\n")
            ),
            1.0,
            tool_results,
        )))
    }
}

// ── 8. Random Tool Use ───────────────────────────────────────────────────

pub(super) struct RandomToolUseShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for RandomToolUseShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::RandomToolUse)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let mut names: Vec<String> = ctx
            .state
            .tools
            .list()
            .iter()
            .map(|t| t.name().to_string())
            .collect();
        names.sort();
        let pick = if names.is_empty() {
            "echo".to_string()
        } else {
            let idx = ctx.user_content.len() % names.len();
            names[idx].clone()
        };
        let (tool_to_call, params) = if pick == "echo" {
            (
                "echo",
                serde_json::json!({"message": format!("{} reporting for duty.", ctx.agent_name)}),
            )
        } else if pick == "get_runtime_context" {
            ("get_runtime_context", serde_json::json!({}))
        } else {
            (
                "echo",
                serde_json::json!({"message": format!("tool probe via {}", pick)}),
            )
        };

        let out = super::tools::execute_tool_call(
            ctx.state,
            tool_to_call,
            &params,
            ctx.turn_id,
            ctx.authority,
            Some(ctx.channel_label),
        )
        .await;
        match out {
            Ok(output) => {
                let prose = format_introspection_prose(tool_to_call, &output);
                let tool_results = vec![(tool_to_call.to_string(), output)];
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "Tool inventory follows.\nAvailable tools (sample): {}.\nRandom pick: {}.\nResult: {}",
                        names.into_iter().take(12).collect::<Vec<_>>().join(", "),
                        tool_to_call,
                        prose
                    ),
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                "I attempted a direct tool execution shortcut, but it failed. I can retry on your command.".to_string(),
                0.0,
                vec![("echo".to_string(), format!("error: {err}"))],
            ))),
        }
    }
}

// ── 9. Markdown Count Scan ───────────────────────────────────────────────

pub(super) struct MarkdownCountScanShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for MarkdownCountScanShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        // Only fires when delegation is NOT also requested (delegation sub-branch handles that)
        intents.contains(&Intent::MarkdownCountScan) && !intents.contains(&Intent::Delegation)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let path = extract_path_hint(ctx.user_content).unwrap_or_else(|| "~/code".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_markdown_count_command(&path);
        let out = execute_bash_tool(ctx, &cmd, 60).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let numeric: String = output.chars().filter(|c| c.is_ascii_digit()).collect();
                let count = if numeric.is_empty() {
                    "0".to_string()
                } else {
                    numeric
                };
                let content = if requests_count_only_numeric_output(ctx.user_content) {
                    count.clone()
                } else {
                    format!(
                        "Found {count} markdown files under {path} (resolved: {resolved_path})."
                    )
                };
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    content,
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "I attempted to count markdown files under {path} (resolved: {resolved_path}), but the command failed: {err}"
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 10. Folder Scan / File Distribution ──────────────────────────────────

pub(super) struct FolderScanShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for FolderScanShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::FileDistribution) || intents.contains(&Intent::FolderScan)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let path = extract_path_hint(ctx.user_content).unwrap_or_else(|| ".".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_distribution_command(&path);
        let label = if ctx.intents.contains(&Intent::FolderScan) {
            "Folder scan"
        } else {
            "File distribution"
        };
        let out = execute_bash_tool(ctx, &cmd, 45).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "{label} for {path} (resolved: {resolved_path}):\n{}",
                        output.trim()
                    ),
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "I attempted to compute {} for {path} (resolved: {resolved_path}), but the command failed: {err}",
                        label.to_ascii_lowercase()
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 11. Wallet Address Scan ──────────────────────────────────────────────

pub(super) struct WalletAddressScanShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for WalletAddressScanShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::WalletAddressScan)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let path = extract_path_hint(ctx.user_content).unwrap_or_else(|| "~/code".to_string());
        let resolved_path = expand_user_path(&path);
        let cmd = build_wallet_scan_command(&path);
        let out = execute_bash_tool(ctx, &cmd, 90).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let trimmed = output.trim();
                let content = if trimmed.is_empty() {
                    format!(
                        "No wallet-address-or-credential-like patterns were found under {path} (resolved: {resolved_path})."
                    )
                } else {
                    format!(
                        "Wallet-address-or-credential-like patterns found under {path} (resolved: {resolved_path}):\n{trimmed}"
                    )
                };
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    content,
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "I attempted a recursive wallet-address scan under {path} (resolved: {resolved_path}), but the command failed: {err}"
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 12. Image Count Scan ─────────────────────────────────────────────────

pub(super) struct ImageCountScanShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for ImageCountScanShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::ImageCountScan)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let lower = ctx.user_content.to_ascii_lowercase();
        let path = if let Some(p) = extract_path_hint(ctx.user_content) {
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
        let out = execute_bash_tool(ctx, &cmd, 90).await;
        let mut tool_results = Vec::new();
        match out {
            Ok(output) => {
                tool_results.push(("bash".to_string(), output.clone()));
                let count = output.trim().parse::<u64>().unwrap_or(0);
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!("Found {count} image files under {path} (resolved: {resolved_path})."),
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "I attempted to count image files under {path} (resolved: {resolved_path}), but the command failed: {err}"
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 13. Obsidian Insights ────────────────────────────────────────────────

pub(super) struct ObsidianInsightsShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for ObsidianInsightsShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::ObsidianInsights)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let path = extract_path_hint(ctx.user_content)
            .map(|p| expand_user_path(&p))
            .or_else(default_obsidian_vault_path)
            .unwrap_or_else(|| "~/Documents/Obsidian Vault".to_string());
        let cmd = build_obsidian_insight_command(&path);
        let out = execute_bash_tool(ctx, &cmd, 90).await;
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
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "Obsidian vault scan complete for {}.\nNote count: {}.\n\n{}",
                        path,
                        note_count,
                        output.trim()
                    ),
                    1.0,
                    tool_results,
                )))
            }
            Err(err) => {
                tool_results.push(("bash".to_string(), format!("error: {err}")));
                Ok(Some(shortcut_output(
                    ctx.prepared_model,
                    format!(
                        "I attempted to analyze the Obsidian vault at {path}, but the scan failed: {err}"
                    ),
                    0.0,
                    tool_results,
                )))
            }
        }
    }
}

// ── 14. Generic Delegation ───────────────────────────────────────────────

pub(super) struct DelegationShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for DelegationShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::Delegation)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        // Sub-branch: delegation + markdown count → numeric-only output via bash
        let lower = ctx.user_content.to_ascii_lowercase();
        if lower.contains("markdown") && lower.contains("count only") {
            let path = extract_path_hint(ctx.user_content)
                .unwrap_or_else(|| "~/code/ironclad".to_string());
            let cmd = build_markdown_count_command(&path);
            let out = execute_bash_tool(ctx, &cmd, 90).await;
            let mut tool_results = Vec::new();
            match out {
                Ok(output) => {
                    tool_results.push(("bash".to_string(), output.clone()));
                    let digits: String = output
                        .trim()
                        .chars()
                        .filter(|c| c.is_ascii_digit())
                        .collect();
                    let content = if digits.is_empty() {
                        "0".to_string()
                    } else {
                        digits
                    };
                    let quality = if content == "0" { 0.5 } else { 1.0 };
                    return Ok(Some(shortcut_output(
                        ctx.prepared_model,
                        content,
                        quality,
                        tool_results,
                    )));
                }
                Err(e) => {
                    tracing::warn!(model = %ctx.prepared_model, error = %e, "numeric inference failed; returning zero");
                    return Ok(Some(shortcut_output(
                        ctx.prepared_model,
                        "0".to_string(),
                        0.0,
                        vec![("bash".to_string(), format!("error: {e}"))],
                    )));
                }
            }
        }

        // Generic delegation via orchestrate-subagents
        let (result, tool_results) = execute_delegation_tool(ctx, ctx.user_content).await;
        match result {
            Ok(output) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                super::guards::strip_internal_delegation_metadata(&output),
                1.0,
                tool_results,
            ))),
            Err(err) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                format!("I attempted delegation via orchestrate-subagents, but it failed: {err}"),
                0.0,
                tool_results,
            ))),
        }
    }
}

// ── 15. Cron ─────────────────────────────────────────────────────────────
// NOTE: The original implementation bypasses the tool layer and accesses the DB
// directly.  This is a security concern (documented in the plan).  For Phase 1
// we preserve the original behavior; in Phase 5 this should route through
// execute_tool_call() once a proper cron tool is registered.

pub(super) struct CronShortcut;

#[async_trait::async_trait]
impl ShortcutHandler for CronShortcut {
    fn handles(&self, intents: &[Intent]) -> bool {
        intents.contains(&Intent::Cron)
    }

    async fn execute(
        &self,
        ctx: &mut ShortcutContext<'_>,
    ) -> Result<Option<InferenceOutput>, String> {
        let agent_id = {
            let cfg = ctx.state.config.read().await;
            cfg.agent.id.clone()
        };
        let name = format!("agent-cron-{}", &ctx.turn_id[..ctx.turn_id.len().min(8)]);
        let schedule_expr = "*/5 * * * *";
        match ironclad_db::cron::create_job(
            &ctx.state.db,
            &name,
            &agent_id,
            "cron",
            Some(schedule_expr),
            "{}",
        ) {
            Ok(job_id) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                format!(
                    "By your command, scheduled cron job '{}' (id: {}) with expression '{}'.",
                    name, job_id, schedule_expr
                ),
                1.0,
                vec![(
                    "cron-create".to_string(),
                    format!("job_id={job_id} schedule={schedule_expr}"),
                )],
            ))),
            Err(err) => Ok(Some(shortcut_output(
                ctx.prepared_model,
                format!("I attempted to schedule a cron job, but creation failed: {err}"),
                0.0,
                vec![("cron-create".to_string(), format!("error: {err}"))],
            ))),
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
//  Tests
// ═══════════════════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acknowledgement_handles_intent() {
        let s = AcknowledgementShortcut;
        assert!(s.handles(&[Intent::Acknowledgement]));
        assert!(!s.handles(&[Intent::Execution]));
        assert!(!s.requires_cache_bypass());
    }

    #[test]
    fn capability_summary_handles_intent() {
        let s = CapabilitySummaryShortcut;
        assert!(s.handles(&[Intent::CapabilitySummary]));
        assert!(!s.handles(&[Intent::ModelIdentity]));
        assert!(s.requires_cache_bypass());
    }

    #[test]
    fn personality_profile_handles_intent() {
        let s = PersonalityProfileShortcut;
        assert!(s.handles(&[Intent::PersonalityProfile]));
        assert!(!s.handles(&[Intent::Execution]));
    }

    #[test]
    fn provider_inventory_handles_intent() {
        let s = ProviderInventoryShortcut;
        assert!(s.handles(&[Intent::ProviderInventory]));
        assert!(!s.handles(&[Intent::Execution]));
    }

    #[test]
    fn current_events_handles_intent() {
        let s = CurrentEventsShortcut;
        assert!(s.handles(&[Intent::CurrentEvents]));
        assert!(!s.handles(&[Intent::Introspection]));
    }

    #[test]
    fn email_triage_handles_intent() {
        let s = EmailTriageShortcut;
        assert!(s.handles(&[Intent::EmailTriage]));
        assert!(!s.handles(&[Intent::CurrentEvents]));
    }

    #[test]
    fn introspection_handles_intent() {
        let s = IntrospectionShortcut;
        assert!(s.handles(&[Intent::Introspection]));
        assert!(!s.handles(&[Intent::Delegation]));
    }

    #[test]
    fn random_tool_use_handles_intent() {
        let s = RandomToolUseShortcut;
        assert!(s.handles(&[Intent::RandomToolUse]));
        assert!(!s.handles(&[Intent::Introspection]));
    }

    #[test]
    fn markdown_count_scan_excludes_delegation() {
        let s = MarkdownCountScanShortcut;
        assert!(s.handles(&[Intent::MarkdownCountScan]));
        // When delegation is also present, this shortcut should NOT fire
        assert!(!s.handles(&[Intent::MarkdownCountScan, Intent::Delegation]));
    }

    #[test]
    fn folder_scan_handles_both_intents() {
        let s = FolderScanShortcut;
        assert!(s.handles(&[Intent::FolderScan]));
        assert!(s.handles(&[Intent::FileDistribution]));
        assert!(s.handles(&[Intent::FolderScan, Intent::FileDistribution]));
        assert!(!s.handles(&[Intent::Execution]));
    }

    #[test]
    fn wallet_scan_handles_intent() {
        let s = WalletAddressScanShortcut;
        assert!(s.handles(&[Intent::WalletAddressScan]));
    }

    #[test]
    fn image_count_scan_handles_intent() {
        let s = ImageCountScanShortcut;
        assert!(s.handles(&[Intent::ImageCountScan]));
    }

    #[test]
    fn obsidian_insights_handles_intent() {
        let s = ObsidianInsightsShortcut;
        assert!(s.handles(&[Intent::ObsidianInsights]));
    }

    #[test]
    fn delegation_handles_intent() {
        let s = DelegationShortcut;
        assert!(s.handles(&[Intent::Delegation]));
    }

    #[test]
    fn cron_handles_intent() {
        let s = CronShortcut;
        assert!(s.handles(&[Intent::Cron]));
    }

    #[test]
    fn shortcut_output_fields_correct() {
        let out = shortcut_output("test-model", "hello".to_string(), 0.8, vec![]);
        assert_eq!(out.content, "hello");
        assert_eq!(out.model, "test-model");
        assert_eq!(out.tokens_in, 0);
        assert_eq!(out.tokens_out, 0);
        assert_eq!(out.cost, 0.0);
        assert_eq!(out.react_turns, 1);
        assert_eq!(out.quality_score, 0.8);
        assert!(!out.escalated);
        assert!(out.tool_results.is_empty());
    }

    // ── Helper tests ─────────────────────────────────────────────────

    #[test]
    fn extract_path_hint_tilde() {
        assert_eq!(
            extract_path_hint("count files in ~/Documents"),
            Some("~/Documents".to_string())
        );
    }

    #[test]
    fn extract_path_hint_absolute() {
        assert_eq!(
            extract_path_hint("scan /var/log for errors"),
            Some("/var/log".to_string())
        );
    }

    #[test]
    fn extract_path_hint_named_folder() {
        assert_eq!(
            extract_path_hint("look at my downloads folder"),
            Some("~/Downloads".to_string())
        );
    }

    #[test]
    fn extract_path_hint_none() {
        assert_eq!(extract_path_hint("hello world"), None);
    }

    #[test]
    fn expand_user_path_tilde() {
        let expanded = expand_user_path("~/test");
        assert!(!expanded.starts_with("~/") || std::env::var("HOME").is_err());
    }

    #[test]
    fn shell_quote_simple() {
        assert_eq!(shell_quote("hello"), "'hello'");
    }

    #[test]
    fn shell_quote_with_quotes() {
        assert_eq!(shell_quote("it's"), "'it'\"'\"'s'");
    }

    #[test]
    fn requests_count_only_numeric() {
        assert!(requests_count_only_numeric_output("give me count only"));
        assert!(requests_count_only_numeric_output("return only the number"));
        assert!(!requests_count_only_numeric_output("how many files"));
    }

    #[test]
    fn format_introspection_prose_runtime() {
        let json = r#"{"agent_id":"test","session_id":"s1","channel":"api","workspace_root":"/tmp","authority":"admin"}"#;
        let prose = format_introspection_prose("get_runtime_context", json);
        assert!(prose.contains("test"));
        assert!(prose.contains("api"));
    }

    #[test]
    fn format_introspection_prose_invalid_json() {
        let prose = format_introspection_prose("get_runtime_context", "not json");
        assert!(prose.contains("get_runtime_context"));
        assert!(prose.contains("not json"));
    }

    #[test]
    fn dispatcher_default_has_all_handlers() {
        let d = ShortcutDispatcher::default_dispatcher();
        assert_eq!(d.handlers.len(), 15);
    }

    #[test]
    fn dispatcher_handler_order_acknowledgement_first() {
        let d = ShortcutDispatcher::default_dispatcher();
        // First handler should handle Acknowledgement
        assert!(d.handlers[0].handles(&[Intent::Acknowledgement]));
        // And it should not require cache bypass
        assert!(!d.handlers[0].requires_cache_bypass());
    }
}
