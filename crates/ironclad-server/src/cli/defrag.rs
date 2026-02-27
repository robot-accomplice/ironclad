#![allow(non_snake_case, unused_variables)]

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use regex::Regex;
use serde::{Deserialize, Serialize};

use super::{CRT_DRAW_MS, colors, icons, theme};

// ── Types ───────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragFinding {
    pub file: PathBuf,
    pub line: Option<usize>,
    pub severity: Severity,
    pub message: String,
    pub fix_description: Option<String>,
    pub fixable: bool,
    pub pass_name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefragSummary {
    pub total_findings: usize,
    pub fixable_count: usize,
    pub by_severity: SeverityCounts,
    pub by_pass: Vec<PassSummary>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SeverityCounts {
    pub info: usize,
    pub warning: usize,
    pub error: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassSummary {
    pub name: String,
    pub findings: usize,
    pub status: PassStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PassStatus {
    Clean,
    Findings,
    Error,
}

// ── Helpers ─────────────────────────────────────────────────────

fn migration_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("openclaw", "ironclad");
    m.insert("open_claw", "ironclad");
    m.insert("OpenClaw", "Ironclad");
    m.insert("oclaw", "ironclad");
    m
}

fn walk_files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
    let mut result = Vec::new();
    walk_files_inner(dir, extensions, &mut result);
    result
}

fn walk_files_inner(dir: &Path, extensions: &[&str], out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            // Skip hidden directories and common build artifacts
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if name.starts_with('.') || name == "target" || name == "node_modules" {
                continue;
            }
            walk_files_inner(&path, extensions, out);
        } else if path.is_file() {
            if extensions.is_empty() {
                out.push(path);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str())
                && extensions.contains(&ext)
            {
                out.push(path);
            }
        }
    }
}

fn walk_all_entries(dir: &Path) -> Vec<PathBuf> {
    let mut result = Vec::new();
    walk_all_entries_inner(dir, &mut result);
    result
}

fn walk_all_entries_inner(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        out.push(path.clone());
        if path.is_dir() {
            walk_all_entries_inner(&path, out);
        }
    }
}

fn is_dir_empty(path: &Path) -> bool {
    match fs::read_dir(path) {
        Ok(mut entries) => entries.next().is_none(),
        Err(_) => false,
    }
}

// ── Pass 1: Stale references ────────────────────────────────────

pub fn pass_refs(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let map = migration_map();
    let extensions = &["md", "sh", "py", "js", "toml", "json"];
    let files = walk_files(workspace, extensions);

    // Build a single regex matching all old names (case-sensitive)
    let pattern = map.keys().copied().collect::<Vec<_>>().join("|");
    let re = match Regex::new(&pattern) {
        Ok(r) => r,
        Err(_) => return findings,
    };

    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (line_num, line) in content.lines().enumerate() {
            for m in re.find_iter(line) {
                let old = m.as_str();
                let new = map.get(old).copied().unwrap_or("ironclad");
                findings.push(DefragFinding {
                    file: file.clone(),
                    line: Some(line_num + 1),
                    severity: Severity::Warning,
                    message: format!("stale reference '{old}' should be '{new}'"),
                    fix_description: Some(format!("replace '{old}' with '{new}'")),
                    fixable: true,
                    pass_name: "refs".to_string(),
                });
            }
        }
    }
    findings
}

// ── Pass 2: Config drift ────────────────────────────────────────

pub fn pass_drift(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let config_path = workspace.join("ironclad.toml");
    let config_content = match fs::read_to_string(&config_path) {
        Ok(c) => c,
        Err(_) => return findings, // No config file, nothing to check
    };

    let config: toml::Value = match config_content.parse() {
        Ok(v) => v,
        Err(_) => return findings,
    };

    // Extract config values to check
    let port = config
        .get("server")
        .and_then(|s| s.get("port"))
        .and_then(|p| p.as_integer())
        .map(|p| p.to_string());
    let bind = config
        .get("server")
        .and_then(|s| s.get("bind"))
        .and_then(|b| b.as_str())
        .map(|s| s.to_string());
    let agent_name = config
        .get("agent")
        .and_then(|a| a.get("name"))
        .and_then(|n| n.as_str())
        .map(|s| s.to_string());

    let port_re = Regex::new(r"(?:port\s*[=:]\s*|localhost:)(\d{4,5})").ok();
    let bind_re = Regex::new(r"(?:bind\s*[=:]\s*)([0-9]+\.[0-9]+\.[0-9]+\.[0-9]+)").ok();
    let name_re = Regex::new(r#"(?:agent[_\s-]?name\s*[=:"]\s*)(\w+)"#).ok();

    let md_files = walk_files(workspace, &["md"]);
    for file in md_files {
        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        // Check for port references that differ from config
        if let Some(ref cfg_port) = port
            && let Some(ref port_re) = port_re
        {
            for (line_num, line) in content.lines().enumerate() {
                for cap in port_re.captures_iter(line) {
                    if let Some(found_port) = cap.get(1) {
                        let found = found_port.as_str();
                        if found != cfg_port.as_str() {
                            findings.push(DefragFinding {
                                file: file.clone(),
                                line: Some(line_num + 1),
                                severity: Severity::Info,
                                message: format!(
                                    "references port {found} but config uses {cfg_port}"
                                ),
                                fix_description: None,
                                fixable: false,
                                pass_name: "drift".to_string(),
                            });
                        }
                    }
                }
            }
        }
        // Check for bind address references that differ from config
        if let Some(ref cfg_bind) = bind
            && let Some(ref bind_re) = bind_re
        {
            for (line_num, line) in content.lines().enumerate() {
                for cap in bind_re.captures_iter(line) {
                    if let Some(found_bind) = cap.get(1) {
                        let found = found_bind.as_str();
                        if found != cfg_bind.as_str() {
                            findings.push(DefragFinding {
                                file: file.clone(),
                                line: Some(line_num + 1),
                                severity: Severity::Info,
                                message: format!(
                                    "references bind address {found} but config uses {cfg_bind}"
                                ),
                                fix_description: None,
                                fixable: false,
                                pass_name: "drift".to_string(),
                            });
                        }
                    }
                }
            }
        }
        // Check for agent name references that differ from config
        if let Some(ref cfg_name) = agent_name
            && let Some(ref name_re) = name_re
        {
            for (line_num, line) in content.lines().enumerate() {
                for cap in name_re.captures_iter(line) {
                    if let Some(found_name) = cap.get(1) {
                        let found = found_name.as_str();
                        if found != cfg_name.as_str() {
                            findings.push(DefragFinding {
                                file: file.clone(),
                                line: Some(line_num + 1),
                                severity: Severity::Info,
                                message: format!(
                                    "references agent name '{found}' but config uses '{cfg_name}'"
                                ),
                                fix_description: None,
                                fixable: false,
                                pass_name: "drift".to_string(),
                            });
                        }
                    }
                }
            }
        }
    }
    findings
}

// ── Pass 3: Build artifacts ─────────────────────────────────────

pub fn pass_artifacts(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let skills_dir = workspace.join("skills");
    if !skills_dir.is_dir() {
        return findings;
    }

    let entries = walk_all_entries(&skills_dir);
    for path in entries {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        if ext == "log" && path.is_file() {
            findings.push(DefragFinding {
                file: path,
                line: None,
                severity: Severity::Info,
                message: "stale log file in skills directory".to_string(),
                fix_description: Some("delete log file".to_string()),
                fixable: true,
                pass_name: "artifacts".to_string(),
            });
        } else if name == "__pycache__" && path.is_dir() {
            findings.push(DefragFinding {
                file: path,
                line: None,
                severity: Severity::Info,
                message: "__pycache__ directory in skills".to_string(),
                fix_description: Some("delete __pycache__ directory".to_string()),
                fixable: true,
                pass_name: "artifacts".to_string(),
            });
        } else if ext == "bak" && path.is_file() {
            findings.push(DefragFinding {
                file: path,
                line: None,
                severity: Severity::Warning,
                message: "backup file in skills directory".to_string(),
                fix_description: Some("delete backup file".to_string()),
                fixable: true,
                pass_name: "artifacts".to_string(),
            });
        } else if path.is_dir() && is_dir_empty(&path) {
            findings.push(DefragFinding {
                file: path,
                line: None,
                severity: Severity::Info,
                message: "empty directory in skills".to_string(),
                fix_description: Some("remove empty directory".to_string()),
                fixable: true,
                pass_name: "artifacts".to_string(),
            });
        }
    }
    findings
}

// ── Pass 4: Stale state entries ─────────────────────────────────

pub fn pass_stale(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let state_path = workspace.join(".ironclad").join("update_state.json");
    let content = match fs::read_to_string(&state_path) {
        Ok(c) => c,
        Err(_) => return findings,
    };

    let state: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(_) => return findings,
    };

    // Check for entries that reference files which no longer exist
    if let Some(obj) = state.as_object() {
        for (key, value) in obj {
            if let Some(path_str) = value.as_str() {
                let target = workspace.join(path_str);
                if !target.exists() {
                    findings.push(DefragFinding {
                        file: state_path.clone(),
                        line: None,
                        severity: Severity::Warning,
                        message: format!("ghost entry '{key}' references missing file: {path_str}"),
                        fix_description: Some(format!(
                            "remove entry '{key}' from update_state.json"
                        )),
                        fixable: true,
                        pass_name: "stale".to_string(),
                    });
                }
            }
            // Also handle entries where the value is an object with a "path" field
            if let Some(inner_obj) = value.as_object()
                && let Some(path_str) = inner_obj.get("path").and_then(|p| p.as_str())
            {
                let target = workspace.join(path_str);
                if !target.exists() {
                    findings.push(DefragFinding {
                        file: state_path.clone(),
                        line: None,
                        severity: Severity::Warning,
                        message: format!("ghost entry '{key}' references missing file: {path_str}"),
                        fix_description: Some(format!(
                            "remove entry '{key}' from update_state.json"
                        )),
                        fixable: true,
                        pass_name: "stale".to_string(),
                    });
                }
            }
        }
    }
    findings
}

// ── Pass 5: Brand identity ──────────────────────────────────────

pub fn pass_identity(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let map = migration_map();
    let files = walk_files(workspace, &["toml", "json"]);

    let old_brands: Vec<&str> = map.keys().copied().collect();
    let brand_pattern = old_brands.join("|");
    let re = match Regex::new(&format!(
        r#"(?:generated_by|brand)\s*[=:]\s*["']?({brand_pattern})"#
    )) {
        Ok(r) => r,
        Err(_) => return findings,
    };

    for file in files {
        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        for (line_num, line) in content.lines().enumerate() {
            for cap in re.captures_iter(line) {
                if let Some(m) = cap.get(1) {
                    let old = m.as_str();
                    let new = map.get(old).copied().unwrap_or("ironclad");
                    findings.push(DefragFinding {
                        file: file.clone(),
                        line: Some(line_num + 1),
                        severity: Severity::Warning,
                        message: format!("brand identity field references '{old}'"),
                        fix_description: Some(format!("replace '{old}' with '{new}'")),
                        fixable: true,
                        pass_name: "identity".to_string(),
                    });
                }
            }
        }
    }
    findings
}

// ── Pass 6: Script validation ───────────────────────────────────

pub fn pass_scripts(workspace: &Path) -> Vec<DefragFinding> {
    let mut findings = Vec::new();
    let skills_dir = workspace.join("skills");
    if !skills_dir.is_dir() {
        return findings;
    }

    let script_files = walk_files(&skills_dir, &["sh", "py", "rb", "pl"]);
    let shebang_re = Regex::new(r"^#!.+").unwrap();
    let hardcoded_re = Regex::new(r"/usr/local/bin/openclaw").unwrap();

    for file in script_files {
        let content = match fs::read_to_string(&file) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let lines: Vec<&str> = content.lines().collect();

        // Check for missing or invalid shebang
        if lines.is_empty() || !shebang_re.is_match(lines[0]) {
            findings.push(DefragFinding {
                file: file.clone(),
                line: Some(1),
                severity: Severity::Warning,
                message: "script missing valid shebang line".to_string(),
                fix_description: None,
                fixable: false,
                pass_name: "scripts".to_string(),
            });
        }

        // Check for hardcoded old paths
        for (line_num, line) in lines.iter().enumerate() {
            if hardcoded_re.is_match(line) {
                findings.push(DefragFinding {
                    file: file.clone(),
                    line: Some(line_num + 1),
                    severity: Severity::Error,
                    message: "hardcoded path '/usr/local/bin/openclaw'".to_string(),
                    fix_description: None,
                    fixable: false,
                    pass_name: "scripts".to_string(),
                });
            }
        }
    }
    findings
}

// ── Fix application ─────────────────────────────────────────────

fn apply_fixes(workspace: &Path, findings: &[DefragFinding]) -> usize {
    let mut fixed = 0;

    // Group fixable findings by pass
    let refs_findings: Vec<&DefragFinding> = findings
        .iter()
        .filter(|f| f.fixable && f.pass_name == "refs")
        .collect();
    let artifact_findings: Vec<&DefragFinding> = findings
        .iter()
        .filter(|f| f.fixable && f.pass_name == "artifacts")
        .collect();
    let stale_findings: Vec<&DefragFinding> = findings
        .iter()
        .filter(|f| f.fixable && f.pass_name == "stale")
        .collect();
    let identity_findings: Vec<&DefragFinding> = findings
        .iter()
        .filter(|f| f.fixable && f.pass_name == "identity")
        .collect();

    // Fix refs: replace old names with new in files
    if !refs_findings.is_empty() {
        let map = migration_map();
        let mut patched_files: HashMap<PathBuf, String> = HashMap::new();
        for f in &refs_findings {
            let content = patched_files
                .entry(f.file.clone())
                .or_insert_with(|| fs::read_to_string(&f.file).unwrap_or_default());
            // not yet replaced — we'll do all replacements at end
        }
        for (path, content) in &mut patched_files {
            let mut updated = content.clone();
            for (old, new) in &map {
                updated = updated.replace(old, new);
            }
            if updated != *content && fs::write(path, &updated).is_ok() {
                fixed += 1;
            }
        }
    }

    // Fix artifacts: delete files/dirs
    for f in &artifact_findings {
        let ok = if f.file.is_dir() {
            fs::remove_dir_all(&f.file).is_ok()
        } else {
            fs::remove_file(&f.file).is_ok()
        };
        if ok {
            fixed += 1;
        }
    }

    // Fix stale: remove ghost entries from update_state.json
    if !stale_findings.is_empty() {
        let state_path = workspace.join(".ironclad").join("update_state.json");
        if let Ok(content) = fs::read_to_string(&state_path)
            && let Ok(mut state) = serde_json::from_str::<serde_json::Value>(&content)
            && let Some(obj) = state.as_object_mut()
        {
            for f in &stale_findings {
                // Extract key name from the message
                if let Some(key) = f
                    .message
                    .strip_prefix("ghost entry '")
                    .and_then(|s| s.split('\'').next())
                {
                    obj.remove(key);
                }
            }
            if let Ok(json) = serde_json::to_string_pretty(&state)
                && fs::write(&state_path, json).is_ok()
            {
                fixed += stale_findings.len();
            }
        }
    }

    // Fix identity: replace old brand names in TOML/JSON files
    if !identity_findings.is_empty() {
        let map = migration_map();
        let mut patched_files: HashMap<PathBuf, String> = HashMap::new();
        for f in &identity_findings {
            patched_files
                .entry(f.file.clone())
                .or_insert_with(|| fs::read_to_string(&f.file).unwrap_or_default());
        }
        for (path, content) in &mut patched_files {
            let mut updated = content.clone();
            for (old, new) in &map {
                updated = updated.replace(old, new);
            }
            if updated != *content && fs::write(path, &updated).is_ok() {
                fixed += 1;
            }
        }
    }

    fixed
}

// ── Main entry point ────────────────────────────────────────────

pub fn cmd_defrag(
    workspace: &Path,
    fix: bool,
    yes: bool,
    json_output: bool,
) -> ironclad_core::Result<()> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();

    // Run all 6 passes
    let pass_names = ["refs", "drift", "artifacts", "stale", "identity", "scripts"];
    let pass_results: Vec<(&str, Vec<DefragFinding>)> = vec![
        ("refs", pass_refs(workspace)),
        ("drift", pass_drift(workspace)),
        ("artifacts", pass_artifacts(workspace)),
        ("stale", pass_stale(workspace)),
        ("identity", pass_identity(workspace)),
        ("scripts", pass_scripts(workspace)),
    ];

    // Build summary
    let mut all_findings: Vec<DefragFinding> = Vec::new();
    let mut by_pass: Vec<PassSummary> = Vec::new();
    let mut severity_counts = SeverityCounts::default();

    for (name, findings) in &pass_results {
        let count = findings.len();
        let status = if count == 0 {
            PassStatus::Clean
        } else {
            PassStatus::Findings
        };
        by_pass.push(PassSummary {
            name: name.to_string(),
            findings: count,
            status,
        });
        for f in findings {
            match f.severity {
                Severity::Info => severity_counts.info += 1,
                Severity::Warning => severity_counts.warning += 1,
                Severity::Error => severity_counts.error += 1,
            }
        }
        all_findings.extend(findings.iter().cloned());
    }

    let fixable_count = all_findings.iter().filter(|f| f.fixable).count();
    let total_findings = all_findings.len();

    let summary = DefragSummary {
        total_findings,
        fixable_count,
        by_severity: severity_counts.clone(),
        by_pass: by_pass.clone(),
    };

    if json_output {
        let output = serde_json::json!({
            "findings": all_findings,
            "summary": summary,
        });
        let json_str = serde_json::to_string_pretty(&output).unwrap_or_else(|_| "{}".to_string());
        // Write JSON to stdout directly, bypassing CRT effect
        let _ = std::io::stdout().write_all(json_str.as_bytes());
        let _ = std::io::stdout().write_all(b"\n");
        let _ = std::io::stdout().flush();
        return Ok(());
    }

    // Human-readable output
    eprintln!();
    eprintln!("  {BOLD}Workspace Defrag{RESET}");
    eprintln!("  {DIM}{}{RESET}", "\u{2500}".repeat(40));
    eprintln!();

    for ps in &by_pass {
        let dots = ".".repeat(20usize.saturating_sub(ps.name.len()));
        let status_str = if ps.findings == 0 {
            format!("{GREEN}clean{RESET}")
        } else {
            let plural = if ps.findings == 1 {
                "finding"
            } else {
                "findings"
            };
            format!("{YELLOW}{} {plural}{RESET}", ps.findings)
        };
        eprintln!(
            "  {DIM}\u{25a0}{RESET} {BOLD}{}{RESET} {DIM}{dots}{RESET} {status_str}",
            ps.name
        );
    }

    eprintln!();
    let fixable_str = if fixable_count > 0 {
        format!(" ({fixable_count} fixable)")
    } else {
        String::new()
    };
    eprintln!("  {BOLD}Summary:{RESET} {total_findings} findings{fixable_str}");
    eprintln!(
        "    {CYAN}info:{RESET} {} {DIM}|{RESET} {YELLOW}warning:{RESET} {} {DIM}|{RESET} {RED}error:{RESET} {}",
        severity_counts.info, severity_counts.warning, severity_counts.error
    );

    // Show individual findings
    if !all_findings.is_empty() {
        eprintln!();
        for f in &all_findings {
            let sev_color = match f.severity {
                Severity::Info => CYAN,
                Severity::Warning => YELLOW,
                Severity::Error => RED,
            };
            let sev_label = match f.severity {
                Severity::Info => "info",
                Severity::Warning => "warn",
                Severity::Error => "error",
            };
            let loc = match f.line {
                Some(l) => format!("{}:{l}", f.file.display()),
                None => format!("{}", f.file.display()),
            };
            eprintln!(
                "  {sev_color}[{sev_label}]{RESET} {DIM}{}{RESET} {loc}",
                f.pass_name
            );
            eprintln!("         {}", f.message);
        }
    }

    // Fix mode
    if fix && fixable_count > 0 {
        let proceed = if yes {
            true
        } else {
            eprint!("\n  Apply {fixable_count} fixable findings? [y/N] ");
            std::io::stderr().flush().ok();
            let mut input = String::new();
            std::io::stdin().read_line(&mut input).unwrap_or(0);
            matches!(input.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
        };

        if proceed {
            let fixed = apply_fixes(workspace, &all_findings);
            eprintln!();
            eprintln!("  {OK} {GREEN}Applied fixes ({fixed} items){RESET}");
        } else {
            eprintln!();
            eprintln!("  {DIM}No changes made.{RESET}");
        }
    } else if fix && fixable_count == 0 {
        eprintln!();
        eprintln!("  {OK} {GREEN}Nothing to fix{RESET}");
    }

    eprintln!();
    Ok(())
}
