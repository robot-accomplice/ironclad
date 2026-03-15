//! Pure utility functions for config file management (backup, validation).
//!
//! These live in `ironclad-core` so that both `ironclad-cli` and `ironclad-api`
//! can use them without creating a circular dependency.

use std::path::{Path, PathBuf};

use chrono::Utc;

use crate::IroncladConfig;

/// Error type for config file operations.
#[derive(Debug)]
pub enum ConfigFileError {
    Io(std::io::Error),
    TomlDeserialize(toml::de::Error),
    Validation(String),
    MissingParent(PathBuf),
}

impl std::fmt::Display for ConfigFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O error: {e}"),
            Self::TomlDeserialize(e) => write!(f, "TOML parse error: {e}"),
            Self::Validation(e) => write!(f, "validation failed: {e}"),
            Self::MissingParent(p) => {
                write!(
                    f,
                    "config parent directory is missing for '{}'",
                    p.display()
                )
            }
        }
    }
}

impl std::error::Error for ConfigFileError {}

impl From<std::io::Error> for ConfigFileError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<toml::de::Error> for ConfigFileError {
    fn from(value: toml::de::Error) -> Self {
        Self::TomlDeserialize(value)
    }
}

/// Creates a timestamped backup of a config file. Returns `None` if the file
/// does not exist.
pub fn backup_config_file(path: &Path) -> Result<Option<PathBuf>, ConfigFileError> {
    if !path.exists() {
        return Ok(None);
    }
    let parent = path
        .parent()
        .ok_or_else(|| ConfigFileError::MissingParent(path.to_path_buf()))?;
    std::fs::create_dir_all(parent)?;
    let stamp = Utc::now().format("%Y%m%dT%H%M%S%.3fZ");
    let file_name = path
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("ironclad.toml");
    let backup_name = format!("{file_name}.bak.{stamp}");
    let backup_path = parent.join(backup_name);
    std::fs::copy(path, &backup_path)?;
    Ok(Some(backup_path))
}

/// Parses and validates a TOML config string.
pub fn parse_and_validate_toml(content: &str) -> Result<IroncladConfig, ConfigFileError> {
    IroncladConfig::from_str(content).map_err(|e| ConfigFileError::Validation(e.to_string()))
}

/// Parses and validates a config file from disk.
pub fn parse_and_validate_file(path: &Path) -> Result<IroncladConfig, ConfigFileError> {
    let content = std::fs::read_to_string(path)?;
    parse_and_validate_toml(&content)
}

/// Resolves the default config file path, checking `./ironclad.toml` first,
/// then `~/.ironclad/ironclad.toml`.
pub fn resolve_default_config_path() -> PathBuf {
    let local = PathBuf::from("ironclad.toml");
    if local.exists() {
        return local;
    }
    let home_cfg = crate::home_dir().join(".ironclad").join("ironclad.toml");
    if home_cfg.exists() {
        return home_cfg;
    }
    local
}

/// Migrate removed legacy config keys, backing up the original first.
pub fn migrate_removed_legacy_config_file(
    path: &Path,
) -> Result<Option<crate::config::ConfigMigrationReport>, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(path)?;
    let Some((rewritten, report)) = crate::config::migrate_removed_legacy_config(&raw)? else {
        return Ok(None);
    };

    backup_config_file(path)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, rewritten)?;
    std::fs::rename(&tmp, path)?;
    Ok(Some(report))
}
