//! Generate isolated TOML config files for sandboxed server instances.
//!
//! Each sandbox gets a unique config pointing at its own temp directory
//! for database, logs, and workspace — ensuring full isolation.

use std::path::{Path, PathBuf};

/// Overrides for the generated config. All fields are optional —
/// defaults produce a minimal working config.
#[derive(Debug, Default)]
pub struct ConfigOverrides {
    pub agent_name: Option<String>,
    pub agent_id: Option<String>,
    pub api_key: Option<String>,
    pub primary_model: Option<String>,
    /// If set, all LLM provider URLs point here (for WireMock).
    pub mock_llm_url: Option<String>,
    /// Raw TOML fragment appended to the generated config.
    pub extra_toml: Option<String>,
}

/// Write a TOML config file into `tmp_dir` with the given port and overrides.
/// Returns the path to the generated config file.
pub fn generate_config(
    tmp_dir: &Path,
    port: u16,
    overrides: &ConfigOverrides,
) -> std::io::Result<PathBuf> {
    let config_path = tmp_dir.join("ironclad.toml");

    let agent_name = overrides.agent_name.as_deref().unwrap_or("HarnessBot");
    let agent_id = overrides.agent_id.as_deref().unwrap_or("harness-test");
    let primary_model = overrides
        .primary_model
        .as_deref()
        .unwrap_or("ollama/qwen3:8b");

    let db_path = tmp_dir.join("state.db");
    let log_dir = tmp_dir.join("logs");
    let workspace = tmp_dir.join("workspace");
    let skills_dir = tmp_dir.join("skills");

    std::fs::create_dir_all(&log_dir)?;
    std::fs::create_dir_all(&workspace)?;
    std::fs::create_dir_all(&skills_dir)?;

    let api_key_line = match overrides.api_key {
        Some(ref key) => format!("api_key = \"{key}\""),
        None => String::new(),
    };

    let mut toml = format!(
        r#"[agent]
name = "{agent_name}"
id = "{agent_id}"
workspace = "{workspace}"

[server]
bind = "127.0.0.1"
port = {port}
{api_key_line}

[database]
path = "{db_path}"

[models]
primary = "{primary_model}"

[skills]
skills_dir = "{skills_dir}"
"#,
        workspace = workspace.display(),
        db_path = db_path.display(),
        skills_dir = skills_dir.display(),
    );

    if let Some(ref url) = overrides.mock_llm_url {
        toml.push_str(&format!(
            r#"
[providers.mock]
url = "{url}"
tier = "T1"
format = "openai"
chat_path = "/v1/chat/completions"
is_local = true
cost_per_input_token = 0.0
cost_per_output_token = 0.0
api_key_env = "MOCK_API_KEY"
"#
        ));
    }

    if let Some(ref extra) = overrides.extra_toml {
        toml.push('\n');
        toml.push_str(extra);
        toml.push('\n');
    }

    std::fs::write(&config_path, toml)?;
    Ok(config_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generates_valid_config() {
        let tmp = tempfile::tempdir().unwrap();
        let path = generate_config(tmp.path(), 50000, &ConfigOverrides::default()).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("port = 50000"));
        assert!(content.contains("HarnessBot"));
        assert!(content.contains("state.db"));
    }

    #[test]
    fn api_key_inlined_in_server_section() {
        let tmp = tempfile::tempdir().unwrap();
        let overrides = ConfigOverrides {
            api_key: Some("my-secret-key".to_string()),
            ..Default::default()
        };
        let path = generate_config(tmp.path(), 50001, &overrides).unwrap();
        let content = std::fs::read_to_string(&path).unwrap();
        // Should have api_key in the SAME [server] section, no duplicate header
        assert!(content.contains("api_key = \"my-secret-key\""));
        assert_eq!(
            content.matches("[server]").count(),
            1,
            "must have exactly one [server] section"
        );
    }
}
