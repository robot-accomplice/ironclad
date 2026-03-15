use std::path::Path;

use ironclad_core::config::{BackupsConfig, ConfigMigrationReport, migrate_removed_legacy_config};

use crate::config_runtime::backup_config_file;

pub fn migrate_removed_legacy_config_file(
    path: &Path,
) -> Result<Option<ConfigMigrationReport>, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(None);
    }

    let raw = std::fs::read_to_string(path)?;
    let Some((rewritten, report)) = migrate_removed_legacy_config(&raw)? else {
        return Ok(None);
    };

    let defaults = BackupsConfig::default();
    backup_config_file(path, defaults.max_count, defaults.max_age_days)?;
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, rewritten)?;
    std::fs::rename(&tmp, path)?;
    Ok(Some(report))
}
