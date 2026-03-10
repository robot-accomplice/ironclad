//! Learning loop — detect successful multi-step tool sequences from completed
//! sessions and synthesize reusable skill documents.
//!
//! # Architecture
//!
//! When a session closes (TTL expiry or rotation), the governor calls
//! [`learn_on_close`].  This function:
//!
//! 1. Loads all tool calls for the session via `get_tool_calls_for_session()`
//! 2. Flattens them chronologically and runs [`detect_candidate_procedures`]
//! 3. For each candidate, either reinforces an existing learned skill or
//!    synthesises a new SKILL.md and persists it
//!
//! No LLM call is involved — learning is pure template synthesis from observed
//! tool-call data, keeping the hot path fast and deterministic.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use ironclad_core::config::LearningConfig;
use ironclad_db::Database;
use ironclad_db::sessions::Session;
use ironclad_db::tools::ToolCallRecord;
use tracing::{debug, info, warn};

// ── Types ──────────────────────────────────────────────────────

/// A candidate procedure detected from a session's tool-call history.
#[derive(Debug, Clone)]
pub struct CandidateProcedure {
    /// Auto-generated name based on the tool chain (e.g. "read-edit-bash").
    pub name: String,
    /// Human-readable description.
    pub description: String,
    /// Ordered tool names that compose this procedure.
    pub tool_sequence: Vec<String>,
    /// Fraction of tools in the sequence that succeeded (0.0–1.0).
    pub success_ratio: f64,
    /// The individual steps with input/output summaries.
    pub steps: Vec<ProcedureStep>,
}

/// A single step within a candidate procedure.
#[derive(Debug, Clone)]
pub struct ProcedureStep {
    pub tool_name: String,
    pub input_summary: String,
    pub output_summary: Option<String>,
    pub status: String,
}

// ── Detection ──────────────────────────────────────────────────

/// Flatten tool calls from all turns into chronological order, then detect
/// consecutive sequences of ≥ `min_length` tools where the success ratio
/// meets the threshold.
pub fn detect_candidate_procedures(
    tool_calls_by_turn: &HashMap<String, Vec<ToolCallRecord>>,
    min_length: usize,
    min_success_ratio: f64,
) -> Vec<CandidateProcedure> {
    // Flatten all tool calls and sort by created_at.
    let mut all_calls: Vec<&ToolCallRecord> =
        tool_calls_by_turn.values().flat_map(|v| v.iter()).collect();
    all_calls.sort_by(|a, b| a.created_at.cmp(&b.created_at));

    if all_calls.len() < min_length {
        return Vec::new();
    }

    // Sliding-window: find contiguous runs of ≥ min_length tools that meet
    // the success threshold.  We skip trivially repeated single-tool runs
    // (e.g. three consecutive "bash" calls).
    let mut candidates: Vec<CandidateProcedure> = Vec::new();

    // Try windows of min_length up to 2×min_length (cap to avoid noise).
    let max_window = (min_length * 2).min(all_calls.len());
    for window_size in min_length..=max_window {
        for window in all_calls.windows(window_size) {
            let success_count = window.iter().filter(|c| c.status == "success").count();
            let ratio = success_count as f64 / window.len() as f64;
            if ratio < min_success_ratio {
                continue;
            }

            let tool_seq: Vec<String> = window.iter().map(|c| c.tool_name.clone()).collect();

            // Skip if all tools are identical (e.g. ["bash","bash","bash"]).
            let distinct: std::collections::HashSet<&str> =
                tool_seq.iter().map(|s| s.as_str()).collect();
            if distinct.len() < 2 {
                continue;
            }

            let name = tool_seq.join("-");
            // De-duplicate: skip if we already captured a candidate with same name.
            if candidates.iter().any(|c| c.name == name) {
                continue;
            }

            let steps: Vec<ProcedureStep> = window
                .iter()
                .map(|c| ProcedureStep {
                    tool_name: c.tool_name.clone(),
                    input_summary: truncate(&c.input, 120),
                    output_summary: c.output.as_deref().map(|o| truncate(o, 120)),
                    status: c.status.clone(),
                })
                .collect();

            let description = format!(
                "{}-step procedure using {}",
                steps.len(),
                distinct_tools_display(&tool_seq),
            );

            candidates.push(CandidateProcedure {
                name,
                description,
                tool_sequence: tool_seq,
                success_ratio: ratio,
                steps,
            });
        }
    }

    candidates
}

// ── Synthesis ──────────────────────────────────────────────────

/// Generate a SKILL.md document from a candidate procedure.
///
/// Format: YAML frontmatter (`---` delimited, matching `parse_instruction_md`)
/// followed by markdown body (steps, tool chain, when to use).
pub fn synthesize_skill_md(candidate: &CandidateProcedure) -> String {
    let triggers: Vec<&str> = {
        let mut seen = std::collections::HashSet::new();
        candidate
            .tool_sequence
            .iter()
            .filter(|t| seen.insert(t.as_str()))
            .map(|t| t.as_str())
            .collect()
    };

    let mut md = String::new();

    // YAML frontmatter (must use `---` delimiters to match SkillLoader)
    md.push_str("---\n");
    md.push_str(&format!("name: {}\n", sanitize_name(&candidate.name)));
    md.push_str(&format!(
        "description: \"{}\"\n",
        candidate.description.replace('"', "'")
    ));
    // triggers as YAML list
    md.push_str("triggers:\n");
    for t in &triggers {
        md.push_str(&format!("  - {t}\n"));
    }
    md.push_str("priority: 50\n");
    md.push_str("version: \"0.0.1\"\n");
    md.push_str("author: learned\n");
    md.push_str("---\n\n");

    // Markdown body
    md.push_str(&format!("# {}\n\n", candidate.description));
    md.push_str(&format!(
        "Learned from a successful {}-step tool sequence.\n\n",
        candidate.steps.len()
    ));

    md.push_str("## Steps\n\n");
    for (i, step) in candidate.steps.iter().enumerate() {
        md.push_str(&format!(
            "{}. **{}** ({})\n",
            i + 1,
            step.tool_name,
            step.status
        ));
        md.push_str(&format!("   - Input: `{}`\n", step.input_summary));
        if let Some(ref out) = step.output_summary {
            md.push_str(&format!("   - Output: `{}`\n", out));
        }
    }
    md.push('\n');

    md.push_str("## When to Use\n\n");
    md.push_str(&format!(
        "This procedure applies when the agent needs to use {} in sequence.\n",
        distinct_tools_display(&candidate.tool_sequence),
    ));

    md
}

/// Write a learned skill to disk under `{skills_dir}/learned/`.
pub fn write_learned_skill(
    skills_dir: &Path,
    candidate: &CandidateProcedure,
    md_content: &str,
) -> ironclad_core::Result<PathBuf> {
    let learned_dir = skills_dir.join("learned");
    std::fs::create_dir_all(&learned_dir).map_err(|e| {
        ironclad_core::IroncladError::Config(format!("failed to create learned skills dir: {e}"))
    })?;

    let filename = format!("{}.md", sanitize_name(&candidate.name));
    let path = learned_dir.join(&filename);
    std::fs::write(&path, md_content).map_err(|e| {
        ironclad_core::IroncladError::Config(format!("failed to write learned skill: {e}"))
    })?;
    Ok(path)
}

// ── Orchestrator ───────────────────────────────────────────────

/// Main entry point: called by the governor when a session closes.
///
/// Mirrors the pattern of `digest_on_close()` — guard on config, extract data,
/// synthesise artifacts, persist, log.
pub fn learn_on_close(
    db: &Database,
    config: &LearningConfig,
    session: &Session,
    skills_dir: &Path,
) {
    if !config.enabled {
        debug!(session_id = %session.id, "learning disabled");
        return;
    }

    // Cap enforcement
    match ironclad_db::learned_skills::count_learned_skills(db) {
        Ok(count) if count >= config.max_learned_skills => {
            debug!(
                count,
                max = config.max_learned_skills,
                "learned skills cap reached, skipping"
            );
            return;
        }
        Err(e) => {
            warn!(error = %e, "failed to count learned skills");
            return;
        }
        _ => {}
    }

    // Load tool calls for this session
    let tool_calls = match ironclad_db::tools::get_tool_calls_for_session(db, &session.id) {
        Ok(tc) => tc,
        Err(e) => {
            warn!(error = %e, session_id = %session.id, "failed to load tool calls for learning");
            return;
        }
    };

    if tool_calls.is_empty() {
        debug!(session_id = %session.id, "no tool calls to learn from");
        return;
    }

    // Detect candidate procedures
    let candidates = detect_candidate_procedures(
        &tool_calls,
        config.min_tool_sequence,
        config.min_success_ratio,
    );

    if candidates.is_empty() {
        debug!(session_id = %session.id, "no candidate procedures detected");
        return;
    }

    for candidate in &candidates {
        // Check if already known
        match ironclad_db::learned_skills::get_learned_skill_by_name(db, &candidate.name) {
            Ok(Some(_existing)) => {
                // Reinforce existing skill
                if let Err(e) =
                    ironclad_db::learned_skills::record_learned_skill_success(db, &candidate.name)
                {
                    warn!(error = %e, name = %candidate.name, "failed to reinforce learned skill");
                }
                debug!(name = %candidate.name, "reinforced existing learned skill");
            }
            Ok(None) => {
                // New skill: store in DB + write .md file
                let trigger_tools_json =
                    serde_json::to_string(&candidate.tool_sequence).unwrap_or_else(|_| "[]".into());
                let steps_json = serde_json::to_string(&steps_to_serializable(&candidate.steps))
                    .unwrap_or_else(|_| "[]".into());

                if let Err(e) = ironclad_db::learned_skills::store_learned_skill(
                    db,
                    &candidate.name,
                    &candidate.description,
                    &trigger_tools_json,
                    &steps_json,
                    Some(&session.id),
                ) {
                    warn!(error = %e, name = %candidate.name, "failed to store learned skill");
                    continue;
                }

                // Synthesise and write .md
                let md = synthesize_skill_md(candidate);
                match write_learned_skill(skills_dir, candidate, &md) {
                    Ok(path) => {
                        let path_str = path.to_string_lossy().to_string();
                        if let Err(e) = ironclad_db::learned_skills::set_learned_skill_md_path(
                            db,
                            &candidate.name,
                            &path_str,
                        ) {
                            warn!(error = %e, "failed to record skill md path");
                        }
                        info!(
                            name = %candidate.name,
                            path = %path_str,
                            steps = candidate.steps.len(),
                            "learned new skill"
                        );
                    }
                    Err(e) => {
                        warn!(error = %e, name = %candidate.name, "failed to write skill .md");
                    }
                }
            }
            Err(e) => {
                warn!(error = %e, "failed to check existing learned skill");
            }
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        // Use floor_char_boundary to avoid panicking on multi-byte UTF-8.
        let boundary = s.floor_char_boundary(max_len);
        format!("{}…", &s[..boundary])
    }
}

fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

fn distinct_tools_display(seq: &[String]) -> String {
    let mut seen = std::collections::HashSet::new();
    let unique: Vec<&str> = seq
        .iter()
        .filter(|t| seen.insert(t.as_str()))
        .map(|t| t.as_str())
        .collect();
    unique.join(" → ")
}

/// Convert steps to a JSON-serializable form.
fn steps_to_serializable(steps: &[ProcedureStep]) -> Vec<HashMap<String, String>> {
    steps
        .iter()
        .map(|s| {
            let mut m = HashMap::new();
            m.insert("tool".into(), s.tool_name.clone());
            m.insert("input".into(), s.input_summary.clone());
            m.insert("status".into(), s.status.clone());
            if let Some(ref out) = s.output_summary {
                m.insert("output".into(), out.clone());
            }
            m
        })
        .collect()
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_call(name: &str, status: &str, time: &str) -> ToolCallRecord {
        ToolCallRecord {
            id: uuid::Uuid::new_v4().to_string(),
            turn_id: "t1".into(),
            tool_name: name.into(),
            input: format!(r#"{{"action":"{name}"}}"#),
            output: Some(format!("{name} output")),
            skill_id: None,
            skill_name: None,
            skill_hash: None,
            status: status.into(),
            duration_ms: Some(50),
            created_at: time.into(),
        }
    }

    fn sample_calls() -> HashMap<String, Vec<ToolCallRecord>> {
        let mut map = HashMap::new();
        map.insert(
            "t1".into(),
            vec![
                make_call("read", "success", "2025-01-01T00:00:01Z"),
                make_call("edit", "success", "2025-01-01T00:00:02Z"),
                make_call("bash", "success", "2025-01-01T00:00:03Z"),
            ],
        );
        map
    }

    #[test]
    fn detect_three_step_procedure() {
        let calls = sample_calls();
        let candidates = detect_candidate_procedures(&calls, 3, 0.7);
        assert!(!candidates.is_empty());
        let c = &candidates[0];
        assert_eq!(c.tool_sequence, vec!["read", "edit", "bash"]);
        assert!((c.success_ratio - 1.0).abs() < f64::EPSILON);
        assert_eq!(c.steps.len(), 3);
    }

    #[test]
    fn no_detection_below_min_length() {
        let mut map = HashMap::new();
        map.insert(
            "t1".into(),
            vec![
                make_call("read", "success", "2025-01-01T00:00:01Z"),
                make_call("edit", "success", "2025-01-01T00:00:02Z"),
            ],
        );
        let candidates = detect_candidate_procedures(&map, 3, 0.7);
        assert!(candidates.is_empty());
    }

    #[test]
    fn no_detection_below_success_ratio() {
        let mut map = HashMap::new();
        map.insert(
            "t1".into(),
            vec![
                make_call("read", "error", "2025-01-01T00:00:01Z"),
                make_call("edit", "error", "2025-01-01T00:00:02Z"),
                make_call("bash", "success", "2025-01-01T00:00:03Z"),
            ],
        );
        // ratio = 1/3 = 0.33 < 0.7
        let candidates = detect_candidate_procedures(&map, 3, 0.7);
        assert!(candidates.is_empty());
    }

    #[test]
    fn skip_identical_tools() {
        let mut map = HashMap::new();
        map.insert(
            "t1".into(),
            vec![
                make_call("bash", "success", "2025-01-01T00:00:01Z"),
                make_call("bash", "success", "2025-01-01T00:00:02Z"),
                make_call("bash", "success", "2025-01-01T00:00:03Z"),
            ],
        );
        let candidates = detect_candidate_procedures(&map, 3, 0.7);
        assert!(
            candidates.is_empty(),
            "all-same-tool sequences should be skipped"
        );
    }

    #[test]
    fn synthesize_skill_md_format() {
        let candidate = CandidateProcedure {
            name: "read-edit-bash".into(),
            description: "3-step procedure using read → edit → bash".into(),
            tool_sequence: vec!["read".into(), "edit".into(), "bash".into()],
            success_ratio: 1.0,
            steps: vec![
                ProcedureStep {
                    tool_name: "read".into(),
                    input_summary: "file.rs".into(),
                    output_summary: Some("contents".into()),
                    status: "success".into(),
                },
                ProcedureStep {
                    tool_name: "edit".into(),
                    input_summary: "change line 5".into(),
                    output_summary: None,
                    status: "success".into(),
                },
                ProcedureStep {
                    tool_name: "bash".into(),
                    input_summary: "cargo test".into(),
                    output_summary: Some("ok".into()),
                    status: "success".into(),
                },
            ],
        };
        let md = synthesize_skill_md(&candidate);
        // YAML frontmatter delimiters
        assert!(md.contains("---"), "expected YAML frontmatter delimiter");
        assert!(md.contains("name: read-edit-bash"));
        assert!(
            md.contains("triggers:") && md.contains("  - read"),
            "expected YAML triggers list"
        );
        assert!(md.contains("## Steps"));
        assert!(md.contains("1. **read**"));
        assert!(md.contains("2. **edit**"));
        assert!(md.contains("3. **bash**"));
        assert!(md.contains("## When to Use"));
    }

    #[test]
    fn write_learned_skill_creates_file() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = CandidateProcedure {
            name: "test-skill".into(),
            description: "test".into(),
            tool_sequence: vec!["a".into(), "b".into()],
            success_ratio: 1.0,
            steps: vec![],
        };
        let md = "# Test\n";
        let path = write_learned_skill(dir.path(), &candidate, md).unwrap();
        assert!(path.exists());
        assert!(path.starts_with(dir.path().join("learned")));
        let content = std::fs::read_to_string(&path).unwrap();
        assert_eq!(content, md);
    }

    #[test]
    fn sanitize_name_handles_special_chars() {
        assert_eq!(sanitize_name("read/edit.bash"), "read-edit-bash");
        assert_eq!(sanitize_name("Read-EDIT_Bash"), "read-edit_bash");
    }

    #[test]
    fn learn_on_close_disabled_is_noop() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let sid = ironclad_db::sessions::find_or_create(&db, "learn-agent", None).unwrap();
        let session = ironclad_db::sessions::get_session(&db, &sid)
            .unwrap()
            .unwrap();
        let dir = tempfile::tempdir().unwrap();

        let config = LearningConfig {
            enabled: false,
            ..LearningConfig::default()
        };
        learn_on_close(&db, &config, &session, dir.path());

        assert_eq!(
            ironclad_db::learned_skills::count_learned_skills(&db).unwrap(),
            0
        );
    }

    #[test]
    fn learn_on_close_with_tool_calls_creates_skill() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let sid = ironclad_db::sessions::find_or_create(&db, "learn-agent", None).unwrap();
        let session = ironclad_db::sessions::get_session(&db, &sid)
            .unwrap()
            .unwrap();

        // Create a turn and tool calls
        {
            let conn = db.conn();
            conn.execute(
                "INSERT INTO turns (id, session_id) VALUES ('lt1', ?1)",
                [&sid],
            )
            .unwrap();
        }
        ironclad_db::tools::record_tool_call(
            &db,
            "lt1",
            "read",
            r#"{"file":"a.rs"}"#,
            Some("contents"),
            "success",
            Some(10),
        )
        .unwrap();
        ironclad_db::tools::record_tool_call(
            &db,
            "lt1",
            "edit",
            r#"{"file":"a.rs"}"#,
            Some("ok"),
            "success",
            Some(20),
        )
        .unwrap();
        ironclad_db::tools::record_tool_call(
            &db,
            "lt1",
            "bash",
            r#"{"cmd":"cargo test"}"#,
            Some("passed"),
            "success",
            Some(30),
        )
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let config = LearningConfig::default();
        learn_on_close(&db, &config, &session, dir.path());

        // A learned skill should now exist
        let count = ironclad_db::learned_skills::count_learned_skills(&db).unwrap();
        assert!(count > 0, "should have learned at least one skill");

        // A .md file should have been written
        let learned_dir = dir.path().join("learned");
        assert!(learned_dir.exists());
        let files: Vec<_> = std::fs::read_dir(&learned_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(!files.is_empty(), "should have written at least one .md");
    }

    #[test]
    fn learn_on_close_respects_cap() {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        let sid = ironclad_db::sessions::find_or_create(&db, "cap-agent", None).unwrap();
        let session = ironclad_db::sessions::get_session(&db, &sid)
            .unwrap()
            .unwrap();

        // Pre-fill to max
        let config = LearningConfig {
            max_learned_skills: 2,
            ..LearningConfig::default()
        };
        ironclad_db::learned_skills::store_learned_skill(&db, "existing-a", "A", "[]", "[]", None)
            .unwrap();
        ironclad_db::learned_skills::store_learned_skill(&db, "existing-b", "B", "[]", "[]", None)
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        learn_on_close(&db, &config, &session, dir.path());

        // Should still be 2 — cap prevented new skills
        assert_eq!(
            ironclad_db::learned_skills::count_learned_skills(&db).unwrap(),
            2
        );
    }
}
