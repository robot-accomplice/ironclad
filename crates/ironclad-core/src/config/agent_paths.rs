#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub name: String,
    pub id: String,
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,
    #[serde(default = "default_log_level")]
    pub log_level: String,
    #[serde(default = "default_true")]
    pub delegation_enabled: bool,
    #[serde(default = "default_min_decomposition_complexity")]
    pub delegation_min_complexity: f64,
    #[serde(default = "default_min_delegation_utility_margin")]
    pub delegation_min_utility_margin: f64,
    #[serde(default = "default_true")]
    pub specialist_creation_requires_approval: bool,
    #[serde(default = "default_autonomy_max_react_turns")]
    pub autonomy_max_react_turns: usize,
    #[serde(default = "default_autonomy_max_turn_duration_seconds")]
    pub autonomy_max_turn_duration_seconds: u64,
}

fn default_workspace() -> PathBuf {
    dirs_next().join("workspace")
}

fn default_log_level() -> String {
    "info".into()
}

fn default_min_decomposition_complexity() -> f64 {
    0.35
}

fn default_min_delegation_utility_margin() -> f64 {
    0.15
}

fn default_autonomy_max_react_turns() -> usize {
    10
}

fn default_autonomy_max_turn_duration_seconds() -> u64 {
    90
}

fn default_log_dir() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".ironclad").join("logs")
}

fn default_log_max_days() -> u32 {
    7
}

fn dirs_next() -> PathBuf {
    home_dir().join(".ironclad")
}

/// Returns the user's home directory, checking `HOME` first (Unix / MSYS2 / Git Bash)
/// then `USERPROFILE` (native Windows). Falls back to the platform temp directory.
pub fn home_dir() -> PathBuf {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir())
}

/// Resolves the configuration file path using a standard precedence chain:
///
/// 1. Explicit path (from `--config` flag or `IRONCLAD_CONFIG` env var)
/// 2. `~/.ironclad/ironclad.toml` (if it exists)
/// 3. `./ironclad.toml` in the current working directory (if it exists)
/// 4. `None` — caller decides the fallback (e.g., built-in defaults or error)
pub fn resolve_config_path(explicit: Option<&str>) -> Option<PathBuf> {
    if let Some(p) = explicit {
        return Some(expand_tilde(Path::new(p)));
    }
    let home_config = home_dir().join(".ironclad").join("ironclad.toml");
    if home_config.exists() {
        return Some(home_config);
    }
    let cwd_config = PathBuf::from("ironclad.toml");
    if cwd_config.exists() {
        return Some(cwd_config);
    }
    None
}

/// Expands a leading `~` in `path` to the user's home directory; otherwise returns the path unchanged.
fn expand_tilde(path: &Path) -> PathBuf {
    if let Ok(stripped) = path.strip_prefix("~") {
        home_dir().join(stripped)
    } else {
        path.to_path_buf()
    }
}

