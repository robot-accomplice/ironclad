use super::*;
use ironclad_llm::oauth::check_and_repair_oauth_storage;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

include!("misc/helpers.rs");
include!("misc/mechanic_json.rs");
#[cfg(windows)]
fn add_dir_to_user_path_windows(dir: &Path) -> Result<(), String> {
    let dir_str = dir.display().to_string().replace('\'', "''");
    let script = format!(
        "$dir='{dir_str}'; \
         $current=[Environment]::GetEnvironmentVariable('Path','User'); \
         if ([string]::IsNullOrWhiteSpace($current)) {{ \
             [Environment]::SetEnvironmentVariable('Path',$dir,'User'); exit 0 \
         }}; \
         $parts=$current -split ';' | Where-Object {{ -not [string]::IsNullOrWhiteSpace($_) }}; \
         $exists=$false; \
         foreach ($p in $parts) {{ if ($p.Trim().ToLowerInvariant() -eq $dir.Trim().ToLowerInvariant()) {{ $exists=$true; break }} }}; \
         if (-not $exists) {{ [Environment]::SetEnvironmentVariable('Path', ($current.TrimEnd(';') + ';' + $dir), 'User') }}"
    );

    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-Command", &script])
        .status()
        .map_err(|e| format!("failed to launch PowerShell: {e}"))?;

    if status.success() {
        Ok(())
    } else {
        Err("PowerShell failed to update user PATH".to_string())
    }
}

// ── Circuit breaker ───────────────────────────────────────────

include!("misc/channel_ops.rs");
include!("misc/mechanic_text.rs");
include!("misc/local_ops.rs");
#[cfg(test)]
mod tests;
