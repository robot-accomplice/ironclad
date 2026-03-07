fn prompt_yes_no(question: &str) -> bool {
    // In non-interactive contexts, default to "no" to avoid surprise installs.
    if std::env::var("IRONCLAD_YES")
        .ok()
        .as_deref()
        .map(|v| v == "1")
        .unwrap_or(false)
    {
        return true;
    }

    print!("{question} [y/N] ");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

/// Check if a TOML string contains a `[section]` header as an actual section
/// (not inside a comment or string value). Checks that the trimmed line
/// matches exactly.
fn has_toml_section(raw: &str, section: &str) -> bool {
    raw.lines().any(|line| line.trim() == section)
}

fn migrate_removed_legacy_config_if_needed(
    config_path: &Path,
    repair: bool,
) -> Result<Option<ironclad_core::config::ConfigMigrationReport>, Box<dyn std::error::Error>> {
    if !repair {
        return Ok(None);
    }
    crate::config_maintenance::migrate_removed_legacy_config_file(config_path)
}

/// Read a line of input from the user, returning the trimmed string.
fn prompt_line(prompt: &str) -> String {
    print!("{prompt}");
    let _ = io::stdout().flush();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return String::new();
    }
    input.trim().to_string()
}

fn path_contains_dir_in(dir: &Path, path_var: &std::ffi::OsStr) -> bool {
    std::env::split_paths(path_var).any(|p| {
        #[cfg(windows)]
        {
            p.to_string_lossy().to_ascii_lowercase() == dir.to_string_lossy().to_ascii_lowercase()
        }
        #[cfg(not(windows))]
        {
            p == dir
        }
    })
}

fn path_contains_dir(dir: &Path) -> bool {
    let Some(path_var) = std::env::var_os("PATH") else {
        return false;
    };
    path_contains_dir_in(dir, &path_var)
}

fn go_bin_candidates_with(gopath: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();

    if let Some(gopath) = gopath {
        out.push(PathBuf::from(gopath).join("bin"));
    }

    #[cfg(windows)]
    if let Ok(profile) = std::env::var("USERPROFILE") {
        out.push(PathBuf::from(profile).join("go").join("bin"));
    }

    #[cfg(not(windows))]
    if let Ok(home) = std::env::var("HOME") {
        out.push(PathBuf::from(home).join("go").join("bin"));
    }

    out
}

fn go_bin_candidates() -> Vec<PathBuf> {
    go_bin_candidates_with(std::env::var("GOPATH").ok().as_deref())
}

fn find_gosh_in_go_bins_with(gopath: Option<&str>) -> Option<PathBuf> {
    #[cfg(windows)]
    let gosh_name = "gosh.exe";
    #[cfg(not(windows))]
    let gosh_name = "gosh";

    go_bin_candidates_with(gopath)
        .into_iter()
        .map(|d| d.join(gosh_name))
        .find(|p| p.is_file())
}

fn find_gosh_in_go_bins() -> Option<PathBuf> {
    find_gosh_in_go_bins_with(std::env::var("GOPATH").ok().as_deref())
}

fn recent_log_snapshot(log_dir: &Path, max_bytes: usize) -> Option<String> {
    let entries = std::fs::read_dir(log_dir).ok()?;
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = entries
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            if !(name.starts_with("ironclad.log") || name == "ironclad.stderr.log") {
                return None;
            }
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, path))
        })
        .collect();
    candidates.sort_by_key(|(modified, _)| *modified);
    let (_, newest) = candidates.into_iter().last()?;
    let data = std::fs::read(newest).ok()?;
    let start = data.len().saturating_sub(max_bytes);
    Some(String::from_utf8_lossy(&data[start..]).to_string())
}

fn count_occurrences(haystack: &str, needle: &str) -> usize {
    haystack.match_indices(needle).count()
}

const INTERNALIZED_SKILLS: &[&str] = &[
    "update-and-rollback",
    "workflow-design",
    "skill-creation",
    "session-operator",
    "claims-auditor",
    "efficacy-assessment",
    "fast-cache",
    "model-routing-tuner",
];

const DEPRECATED_GENERIC_SKILLS: &[&str] =
    &["hello", "explain", "plan", "summarize", "review", "search"];

