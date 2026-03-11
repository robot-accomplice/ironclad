use std::collections::HashMap;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use ironclad_core::config::IroncladConfig;
use ironclad_core::home_dir;
use ironclad_llm::oauth::check_and_repair_oauth_storage;

use super::{colors, heading, icons};
use crate::cli::{CRT_DRAW_MS, theme};

pub(crate) const DEFAULT_REGISTRY_URL: &str = "https://roboticus.ai/registry/manifest.json";
const CRATES_IO_API: &str = "https://crates.io/api/v1/crates/ironclad-server";
const CRATE_NAME: &str = "ironclad-server";
const RELEASE_BASE_URL: &str = "https://github.com/robot-accomplice/ironclad/releases/download";
const GITHUB_RELEASES_API: &str =
    "https://api.github.com/repos/robot-accomplice/ironclad/releases?per_page=100";

// ── Registry manifest (remote) ───────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistryManifest {
    pub version: String,
    pub packs: Packs,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Packs {
    pub providers: ProviderPack,
    pub skills: SkillPack,
    #[serde(default)]
    pub plugins: Option<ironclad_plugin_sdk::catalog::PluginCatalog>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPack {
    pub sha256: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillPack {
    pub sha256: Option<String>,
    pub path: String,
    pub files: HashMap<String, String>,
}

// ── Local update state ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UpdateState {
    pub binary_version: String,
    pub last_check: String,
    pub registry_url: String,
    pub installed_content: InstalledContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct InstalledContent {
    pub providers: Option<ContentRecord>,
    pub skills: Option<SkillsRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContentRecord {
    pub version: String,
    pub sha256: String,
    pub installed_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SkillsRecord {
    pub version: String,
    pub files: HashMap<String, String>,
    pub installed_at: String,
}

impl UpdateState {
    pub fn load() -> Self {
        let path = state_path();
        if path.exists() {
            match std::fs::read_to_string(&path) {
                Ok(content) => serde_json::from_str(&content)
                    .inspect_err(|e| tracing::warn!(error = %e, "corrupted update state file, resetting to default"))
                    .unwrap_or_default(),
                Err(e) => {
                    tracing::warn!(error = %e, "failed to read update state file, resetting to default");
                    Self::default()
                }
            }
        } else {
            Self::default()
        }
    }

    pub fn save(&self) -> io::Result<()> {
        let path = state_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(io::Error::other)?;
        std::fs::write(&path, json)
    }
}

fn state_path() -> PathBuf {
    home_dir().join(".ironclad").join("update_state.json")
}

fn ironclad_home() -> PathBuf {
    home_dir().join(".ironclad")
}

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

// ── Helpers ──────────────────────────────────────────────────

pub fn file_sha256(path: &Path) -> io::Result<String> {
    let bytes = std::fs::read(path)?;
    let hash = Sha256::digest(&bytes);
    Ok(hex::encode(hash))
}

pub fn bytes_sha256(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex::encode(hash)
}

pub(crate) fn resolve_registry_url(cli_override: Option<&str>, config_path: &str) -> String {
    if let Some(url) = cli_override {
        return url.to_string();
    }
    if let Ok(val) = std::env::var("IRONCLAD_REGISTRY_URL")
        && !val.is_empty()
    {
        return val;
    }
    if let Ok(content) = std::fs::read_to_string(config_path)
        && let Ok(config) = content.parse::<toml::Value>()
        && let Some(url) = config
            .get("update")
            .and_then(|u| u.get("registry_url"))
            .and_then(|v| v.as_str())
        && !url.is_empty()
    {
        return url.to_string();
    }
    DEFAULT_REGISTRY_URL.to_string()
}

pub(crate) fn registry_base_url(manifest_url: &str) -> String {
    if let Some(pos) = manifest_url.rfind('/') {
        manifest_url[..pos].to_string()
    } else {
        manifest_url.to_string()
    }
}

fn confirm_action(prompt: &str, default_yes: bool) -> bool {
    let hint = if default_yes { "[Y/n]" } else { "[y/N]" };
    print!("    {prompt} {hint} ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return default_yes;
    }
    let answer = input.trim().to_lowercase();
    if answer.is_empty() {
        return default_yes;
    }
    matches!(answer.as_str(), "y" | "yes")
}

fn confirm_overwrite(filename: &str) -> OverwriteChoice {
    let (_, _, _, _, YELLOW, _, _, RESET, _) = colors();
    print!("    Overwrite {YELLOW}{filename}{RESET}? [y/N/backup] ");
    io::stdout().flush().ok();
    let mut input = String::new();
    if io::stdin().read_line(&mut input).is_err() {
        return OverwriteChoice::Skip;
    }
    match input.trim().to_lowercase().as_str() {
        "y" | "yes" => OverwriteChoice::Overwrite,
        "b" | "backup" => OverwriteChoice::Backup,
        _ => OverwriteChoice::Skip,
    }
}

#[derive(Debug, PartialEq)]
enum OverwriteChoice {
    Overwrite,
    Backup,
    Skip,
}

fn http_client() -> Result<reqwest::Client, Box<dyn std::error::Error>> {
    Ok(reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .user_agent(format!("ironclad/{}", env!("CARGO_PKG_VERSION")))
        .build()?)
}

fn run_oauth_storage_maintenance() {
    let (OK, _, WARN, DETAIL, _) = icons();
    let oauth_health = check_and_repair_oauth_storage(true);
    if oauth_health.needs_attention() {
        if oauth_health.repaired {
            println!("    {OK} OAuth token storage repaired/migrated");
        } else if !oauth_health.keystore_available {
            println!("    {WARN} OAuth migration check skipped (keystore unavailable)");
            println!("    {DETAIL} Run `ironclad mechanic --repair` after fixing keystore access.");
        } else {
            println!("    {WARN} OAuth token storage requires manual attention");
            println!("    {DETAIL} Run `ironclad mechanic --repair` to attempt recovery.");
        }
    } else {
        println!("    {OK} OAuth token storage is healthy");
    }
}

fn run_mechanic_checks_maintenance(config_path: &str) {
    let (OK, _, WARN, DETAIL, _) = icons();
    let state_db = IroncladConfig::from_file(Path::new(config_path))
        .map(|cfg| cfg.database.path)
        .unwrap_or_else(|_| home_dir().join(".ironclad").join("state.db"));
    match crate::state_hygiene::run_state_hygiene(&state_db) {
        Ok(report) if report.changed => {
            println!(
                "    {OK} Mechanic checks repaired {} row(s) (subagents={}, cron_payloads={}, invalid_cron_disabled={})",
                report.changed_rows,
                report.subagent_rows_normalized,
                report.cron_payload_rows_repaired,
                report.cron_jobs_disabled_invalid_expr
            );
        }
        Ok(_) => println!("    {OK} Mechanic checks found no repairs needed"),
        Err(e) => {
            println!("    {WARN} Mechanic checks failed: {e}");
            println!("    {DETAIL} Run `ironclad mechanic --repair` for detailed diagnostics.");
        }
    }
}

fn apply_removed_legacy_config_migration(
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(config_path);
    let (_, _, WARN, DETAIL, _) = icons();
    if let Some(report) = crate::config_maintenance::migrate_removed_legacy_config_file(path)? {
        println!("    {WARN} Removed legacy config compatibility settings during update");
        if report.renamed_server_host_to_bind {
            println!("    {DETAIL} Renamed [server].host to [server].bind");
        }
        if report.routing_mode_heuristic_rewritten {
            println!("    {DETAIL} Rewrote models.routing.mode from heuristic to metascore");
        }
        if report.deny_on_empty_allowlist_hardened {
            println!("    {DETAIL} Hardened security.deny_on_empty_allowlist to true");
        }
        if report.removed_credit_cooldown_seconds {
            println!("    {DETAIL} Removed deprecated circuit_breaker.credit_cooldown_seconds");
        }
    }
    Ok(())
}

// ── Version comparison ───────────────────────────────────────

fn parse_semver(v: &str) -> (u32, u32, u32) {
    let v = v.trim_start_matches('v');
    let v = v.split_once('+').map(|(core, _)| core).unwrap_or(v);
    let v = v.split_once('-').map(|(core, _)| core).unwrap_or(v);
    let parts: Vec<&str> = v.split('.').collect();
    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

pub(crate) fn is_newer(remote: &str, local: &str) -> bool {
    parse_semver(remote) > parse_semver(local)
}

fn platform_archive_name(version: &str) -> Option<String> {
    let (arch, os, ext) = platform_archive_parts()?;
    Some(format!("ironclad-{version}-{arch}-{os}.{ext}"))
}

fn platform_archive_parts() -> Option<(&'static str, &'static str, &'static str)> {
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        _ => return None,
    };
    let os = match std::env::consts::OS {
        "linux" => "linux",
        "macos" => "darwin",
        "windows" => "windows",
        _ => return None,
    };
    let ext = if os == "windows" { "zip" } else { "tar.gz" };
    Some((arch, os, ext))
}

fn parse_sha256sums_for_artifact(sha256sums: &str, artifact: &str) -> Option<String> {
    for raw in sha256sums.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let hash = parts.next()?;
        let file = parts.next()?;
        if file == artifact {
            return Some(hash.to_ascii_lowercase());
        }
    }
    None
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    published_at: Option<String>,
    assets: Vec<GitHubAsset>,
}

#[derive(Debug, Clone, Deserialize)]
struct GitHubAsset {
    name: String,
}

fn core_version(s: &str) -> &str {
    let s = s.trim_start_matches('v');
    let s = s.split_once('+').map(|(core, _)| core).unwrap_or(s);
    s.split_once('-').map(|(core, _)| core).unwrap_or(s)
}

fn select_archive_asset_name(release: &GitHubRelease, version: &str) -> Option<String> {
    let exact = platform_archive_name(version)?;
    if release.assets.iter().any(|a| a.name == exact) {
        return Some(exact);
    }
    let (arch, os, ext) = platform_archive_parts()?;
    let suffix = format!("-{arch}-{os}.{ext}");
    let core_prefix = format!("ironclad-{}", core_version(version));
    release
        .assets
        .iter()
        .find(|a| a.name.ends_with(&suffix) && a.name.starts_with(&core_prefix))
        .map(|a| a.name.clone())
}

fn select_release_for_download(
    releases: &[GitHubRelease],
    version: &str,
) -> Option<(String, String)> {
    let canonical = format!("v{version}");

    if let Some(exact) = releases
        .iter()
        .find(|r| !r.draft && !r.prerelease && r.tag_name == canonical)
    {
        let has_sums = exact.assets.iter().any(|a| a.name == "SHA256SUMS.txt");
        if has_sums && let Some(archive) = select_archive_asset_name(exact, version) {
            return Some((exact.tag_name.clone(), archive));
        }
    }

    releases
        .iter()
        .filter(|r| !r.draft && !r.prerelease)
        .filter(|r| core_version(&r.tag_name) == core_version(version))
        .filter(|r| r.assets.iter().any(|a| a.name == "SHA256SUMS.txt"))
        .filter_map(|r| select_archive_asset_name(r, version).map(|archive| (r, archive)))
        .max_by_key(|(r, _)| r.published_at.as_deref().unwrap_or(""))
        .map(|(r, archive)| (r.tag_name.clone(), archive))
}

async fn resolve_download_release(
    client: &reqwest::Client,
    version: &str,
) -> Result<(String, String), Box<dyn std::error::Error>> {
    let resp = client.get(GITHUB_RELEASES_API).send().await?;
    if !resp.status().is_success() {
        return Err(format!("Failed to query GitHub releases: HTTP {}", resp.status()).into());
    }
    let releases: Vec<GitHubRelease> = resp.json().await?;
    select_release_for_download(&releases, version).ok_or_else(|| {
        format!(
            "No downloadable release found for v{version} with required platform archive and SHA256SUMS.txt"
        )
        .into()
    })
}

fn find_file_recursive(root: &Path, filename: &str) -> io::Result<Option<PathBuf>> {
    for entry in std::fs::read_dir(root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, filename)? {
                return Ok(Some(found));
            }
        } else if path
            .file_name()
            .and_then(|n| n.to_str())
            .map(|n| n == filename)
            .unwrap_or(false)
        {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn install_binary_bytes(bytes: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe()?;
        let staging_dir = std::env::temp_dir().join(format!(
            "ironclad-update-{}-{}",
            std::process::id(),
            chrono::Utc::now().timestamp_millis()
        ));
        std::fs::create_dir_all(&staging_dir)?;
        let staged_exe = staging_dir.join("ironclad-staged.exe");
        std::fs::write(&staged_exe, bytes)?;
        let log_file = staging_dir.join("apply-update.log");
        let script_path = staging_dir.join("apply-update.cmd");
        // The script retries the copy for up to 60 seconds, logs success/failure,
        // and cleans up the staging directory on success.
        let script = format!(
            "@echo off\r\n\
             setlocal\r\n\
             set SRC={src}\r\n\
             set DST={dst}\r\n\
             set LOG={log}\r\n\
             echo [%DATE% %TIME%] Starting binary replacement >> \"%LOG%\"\r\n\
             for /L %%i in (1,1,60) do (\r\n\
               copy /Y \"%SRC%\" \"%DST%\" >nul 2>nul && goto :ok\r\n\
               timeout /t 1 /nobreak >nul\r\n\
             )\r\n\
             echo [%DATE% %TIME%] FAILED: could not replace binary after 60 attempts >> \"%LOG%\"\r\n\
             exit /b 1\r\n\
             :ok\r\n\
             echo [%DATE% %TIME%] SUCCESS: binary replaced >> \"%LOG%\"\r\n\
             del /Q \"%SRC%\" >nul 2>nul\r\n\
             del /Q \"%~f0\" >nul 2>nul\r\n\
             exit /b 0\r\n",
            src = staged_exe.display(),
            dst = exe.display(),
            log = log_file.display(),
        );
        std::fs::write(&script_path, &script)?;
        let _child = std::process::Command::new("cmd")
            .arg("/C")
            .arg(script_path.to_string_lossy().as_ref())
            .creation_flags(0x00000008) // DETACHED_PROCESS
            .spawn()?;
        return Ok(());
    }

    #[cfg(not(windows))]
    {
        let exe = std::env::current_exe()?;
        let tmp = exe.with_extension("new");
        std::fs::write(&tmp, bytes)?;
        #[cfg(unix)]
        {
            let mode = std::fs::metadata(&exe)
                .map(|m| m.permissions().mode())
                .unwrap_or(0o755);
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(mode))?;
        }
        std::fs::rename(&tmp, &exe)?;
        Ok(())
    }
}

async fn apply_binary_download_update(
    client: &reqwest::Client,
    latest: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let _archive_probe = platform_archive_name(latest).ok_or_else(|| {
        format!(
            "No release archive mapping for platform {}/{}",
            std::env::consts::OS,
            std::env::consts::ARCH
        )
    })?;
    let (tag, archive) = resolve_download_release(client, latest).await?;
    let sha_url = format!("{RELEASE_BASE_URL}/{tag}/SHA256SUMS.txt");
    let archive_url = format!("{RELEASE_BASE_URL}/{tag}/{archive}");

    let sha_resp = client.get(&sha_url).send().await?;
    if !sha_resp.status().is_success() {
        return Err(format!("Failed to fetch SHA256SUMS.txt: HTTP {}", sha_resp.status()).into());
    }
    let sha_body = sha_resp.text().await?;
    let expected = parse_sha256sums_for_artifact(&sha_body, &archive)
        .ok_or_else(|| format!("No checksum found for artifact {archive}"))?;

    let archive_resp = client.get(&archive_url).send().await?;
    if !archive_resp.status().is_success() {
        return Err(format!(
            "Failed to download release archive: HTTP {}",
            archive_resp.status()
        )
        .into());
    }
    let archive_bytes = archive_resp.bytes().await?.to_vec();
    let actual = bytes_sha256(&archive_bytes);
    if actual != expected {
        return Err(
            format!("SHA256 mismatch for {archive}: expected {expected}, got {actual}").into(),
        );
    }

    let temp_root = std::env::temp_dir().join(format!(
        "ironclad-update-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    ));
    std::fs::create_dir_all(&temp_root)?;
    let archive_path = if archive.ends_with(".zip") {
        temp_root.join("ironclad.zip")
    } else {
        temp_root.join("ironclad.tar.gz")
    };
    std::fs::write(&archive_path, &archive_bytes)?;

    if archive.ends_with(".zip") {
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                &format!(
                    "Expand-Archive -Path \"{}\" -DestinationPath \"{}\" -Force",
                    archive_path.display(),
                    temp_root.display()
                ),
            ])
            .status()?;
        if !status.success() {
            let _ = std::fs::remove_dir_all(&temp_root);
            return Err(
                format!("Failed to extract {archive} with PowerShell Expand-Archive").into(),
            );
        }
    } else {
        let status = std::process::Command::new("tar")
            .arg("-xzf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&temp_root)
            .status()?;
        if !status.success() {
            let _ = std::fs::remove_dir_all(&temp_root);
            return Err(format!("Failed to extract {archive} with tar").into());
        }
    }

    let bin_name = if std::env::consts::OS == "windows" {
        "ironclad.exe"
    } else {
        "ironclad"
    };
    let extracted = find_file_recursive(&temp_root, bin_name)?
        .ok_or_else(|| format!("Could not locate extracted {bin_name} binary"))?;
    let bytes = std::fs::read(&extracted)?;
    install_binary_bytes(&bytes)?;
    let _ = std::fs::remove_dir_all(&temp_root);
    Ok(())
}

fn c_compiler_available() -> bool {
    #[cfg(windows)]
    {
        if std::process::Command::new("cmd")
            .args(["/C", "where", "cl"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        if std::process::Command::new("gcc")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        return std::process::Command::new("clang")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
    }

    #[cfg(not(windows))]
    {
        if std::process::Command::new("cc")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        if std::process::Command::new("clang")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
        {
            return true;
        }
        std::process::Command::new("gcc")
            .arg("--version")
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }
}

fn apply_binary_cargo_update(latest: &str) -> bool {
    let (DIM, _, _, _, _, _, _, RESET, _) = colors();
    let (OK, _, WARN, DETAIL, ERR) = icons();
    if !c_compiler_available() {
        println!("    {WARN} Local build toolchain check failed: no C compiler found in PATH");
        println!(
            "    {DETAIL} `--method build` requires a C compiler (and related native build tools)."
        );
        println!("    {DETAIL} Recommended: use `ironclad update binary --method download --yes`.");
        #[cfg(windows)]
        {
            println!(
                "    {DETAIL} Windows: install Visual Studio Build Tools (MSVC) or clang/gcc."
            );
        }
        #[cfg(target_os = "macos")]
        {
            println!("    {DETAIL} macOS: run `xcode-select --install`.");
        }
        #[cfg(target_os = "linux")]
        {
            println!(
                "    {DETAIL} Linux: install build tools (for example `build-essential` on Debian/Ubuntu)."
            );
        }
        return false;
    }
    println!("    Installing v{latest} via cargo install...");
    println!("    {DIM}This may take a few minutes.{RESET}");

    let status = std::process::Command::new("cargo")
        .args(["install", CRATE_NAME])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("    {OK} Binary updated to v{latest}");
            true
        }
        Ok(s) => {
            println!(
                "    {ERR} cargo install exited with code {}",
                s.code().unwrap_or(-1)
            );
            false
        }
        Err(e) => {
            println!("    {ERR} Failed to run cargo install: {e}");
            println!("    {DIM}Ensure cargo is in your PATH{RESET}");
            false
        }
    }
}

// ── TOML diff ────────────────────────────────────────────────

pub fn diff_lines(old: &str, new: &str) -> Vec<DiffLine> {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();
    let mut result = Vec::new();

    let max = old_lines.len().max(new_lines.len());
    for i in 0..max {
        match (old_lines.get(i), new_lines.get(i)) {
            (Some(o), Some(n)) if o == n => {
                result.push(DiffLine::Same((*o).to_string()));
            }
            (Some(o), Some(n)) => {
                result.push(DiffLine::Removed((*o).to_string()));
                result.push(DiffLine::Added((*n).to_string()));
            }
            (Some(o), None) => {
                result.push(DiffLine::Removed((*o).to_string()));
            }
            (None, Some(n)) => {
                result.push(DiffLine::Added((*n).to_string()));
            }
            (None, None) => {}
        }
    }
    result
}

#[derive(Debug, PartialEq)]
pub enum DiffLine {
    Same(String),
    Added(String),
    Removed(String),
}

fn print_diff(old: &str, new: &str) {
    let (DIM, _, _, GREEN, _, RED, _, RESET, _) = colors();
    let lines = diff_lines(old, new);
    let changes: Vec<&DiffLine> = lines
        .iter()
        .filter(|l| !matches!(l, DiffLine::Same(_)))
        .collect();

    if changes.is_empty() {
        println!("      {DIM}(no changes){RESET}");
        return;
    }

    for line in &changes {
        match line {
            DiffLine::Removed(s) => println!("      {RED}- {s}{RESET}"),
            DiffLine::Added(s) => println!("      {GREEN}+ {s}{RESET}"),
            DiffLine::Same(_) => {}
        }
    }
}

// ── Binary update ────────────────────────────────────────────

pub(crate) async fn check_binary_version(
    client: &reqwest::Client,
) -> Result<Option<String>, Box<dyn std::error::Error>> {
    let resp = client.get(CRATES_IO_API).send().await?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let body: serde_json::Value = resp.json().await?;
    let latest = body
        .pointer("/crate/max_version")
        .and_then(|v| v.as_str())
        .map(String::from);
    Ok(latest)
}

async fn apply_binary_update(yes: bool, method: &str) -> Result<bool, Box<dyn std::error::Error>> {
    let (DIM, BOLD, _, GREEN, _, _, _, RESET, MONO) = colors();
    let (OK, _, WARN, DETAIL, ERR) = icons();
    let current = env!("CARGO_PKG_VERSION");
    let client = http_client()?;
    let method = method.to_ascii_lowercase();

    println!("\n  {BOLD}Binary Update{RESET}\n");
    println!("    Current version: {MONO}v{current}{RESET}");

    let latest = match check_binary_version(&client).await? {
        Some(v) => v,
        None => {
            println!("    {WARN} Could not reach crates.io");
            return Ok(false);
        }
    };

    println!("    Latest version:  {MONO}v{latest}{RESET}");

    if !is_newer(&latest, current) {
        println!("    {OK} Already on latest version");
        return Ok(false);
    }

    println!("    {GREEN}New version available: v{latest}{RESET}");
    println!();

    if std::env::consts::OS == "windows" && method == "build" {
        println!("    {WARN} Build method is not supported in-process on Windows");
        println!(
            "    {DETAIL} Running executables are file-locked. Use `--method download` (recommended),"
        );
        println!(
            "    {DETAIL} or run `cargo install {CRATE_NAME} --force` from a separate PowerShell session."
        );
        return Ok(false);
    }

    if !yes && !confirm_action("Proceed with binary update?", true) {
        println!("    Skipped.");
        return Ok(false);
    }

    let mut updated = false;
    if method == "download" {
        println!("    Attempting platform binary download + fingerprint verification...");
        match apply_binary_download_update(&client, &latest).await {
            Ok(()) => {
                println!("    {OK} Binary downloaded and verified (SHA256)");
                if std::env::consts::OS == "windows" {
                    println!(
                        "    {DETAIL} Update staged. The replacement finalizes after this process exits."
                    );
                    println!("    {DETAIL} Re-run `ironclad version` in a few seconds to confirm.");
                }
                updated = true;
            }
            Err(e) => {
                println!("    {WARN} Download update failed: {e}");
                if std::env::consts::OS == "windows" {
                    println!(
                        "    {DETAIL} On Windows, fallback build-in-place is blocked by executable locks."
                    );
                    println!(
                        "    {DETAIL} Retry `ironclad update binary --method download` or run build update from a separate shell."
                    );
                } else if confirm_action(
                    "Download failed. Fall back to cargo build update? (slower, compiles from source)",
                    true,
                ) {
                    // BUG-020: Always prompt for build fallback regardless of --yes flag.
                    // The user chose download method explicitly; silently switching to a
                    // cargo build is a different operation (slower, requires Rust toolchain).
                    updated = apply_binary_cargo_update(&latest);
                } else {
                    println!("    Skipped fallback build.");
                }
            }
        }
    } else {
        updated = apply_binary_cargo_update(&latest);
    }

    if updated {
        println!("    {OK} Binary updated to v{latest}");
        let mut state = UpdateState::load();
        state.binary_version = latest;
        state.last_check = now_iso();
        state
            .save()
            .inspect_err(
                |e| tracing::warn!(error = %e, "failed to save update state after version check"),
            )
            .ok();
        Ok(true)
    } else {
        if method == "download" {
            println!("    {ERR} Binary update did not complete");
        }
        Ok(false)
    }
}

// ── Content update (providers + skills) ──────────────────────

pub(crate) async fn fetch_manifest(
    client: &reqwest::Client,
    registry_url: &str,
) -> Result<RegistryManifest, Box<dyn std::error::Error>> {
    let resp = client.get(registry_url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("Registry returned HTTP {}", resp.status()).into());
    }
    let manifest: RegistryManifest = resp.json().await?;
    Ok(manifest)
}

async fn fetch_file(
    client: &reqwest::Client,
    base_url: &str,
    relative_path: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let url = format!("{base_url}/{relative_path}");
    let resp = client.get(&url).send().await?;
    if !resp.status().is_success() {
        return Err(format!("Failed to fetch {relative_path}: HTTP {}", resp.status()).into());
    }
    Ok(resp.text().await?)
}

async fn apply_providers_update(
    yes: bool,
    registry_url: &str,
    config_path: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let (DIM, BOLD, _, GREEN, YELLOW, _, _, RESET, MONO) = colors();
    let (OK, _, WARN, DETAIL, _) = icons();
    let client = http_client()?;

    println!("\n  {BOLD}Provider Configs{RESET}\n");

    let manifest = match fetch_manifest(&client, registry_url).await {
        Ok(m) => m,
        Err(e) => {
            println!("    {WARN} Could not fetch registry manifest: {e}");
            return Ok(false);
        }
    };

    let base_url = registry_base_url(registry_url);
    let remote_content = match fetch_file(&client, &base_url, &manifest.packs.providers.path).await
    {
        Ok(c) => c,
        Err(e) => {
            println!("    {WARN} Could not fetch providers.toml: {e}");
            return Ok(false);
        }
    };

    let remote_hash = bytes_sha256(remote_content.as_bytes());
    let state = UpdateState::load();

    let local_path = providers_local_path(config_path);
    let local_exists = local_path.exists();
    let local_content = if local_exists {
        std::fs::read_to_string(&local_path).unwrap_or_default()
    } else {
        String::new()
    };

    if local_exists {
        let local_hash = bytes_sha256(local_content.as_bytes());
        if local_hash == remote_hash {
            println!("    {OK} Provider configs are up to date");
            return Ok(false);
        }
    }

    let user_modified = if let Some(ref record) = state.installed_content.providers {
        if local_exists {
            let current_hash = file_sha256(&local_path).unwrap_or_default();
            current_hash != record.sha256
        } else {
            false
        }
    } else {
        local_exists
    };

    if !local_exists {
        println!("    {GREEN}+ New provider configuration available{RESET}");
        print_diff("", &remote_content);
    } else if user_modified {
        println!("    {YELLOW}Provider config has been modified locally{RESET}");
        println!("    Changes from registry:");
        print_diff(&local_content, &remote_content);
    } else {
        println!("    Updated provider configuration available");
        print_diff(&local_content, &remote_content);
    }

    println!();

    if user_modified {
        match confirm_overwrite("providers config") {
            OverwriteChoice::Overwrite => {}
            OverwriteChoice::Backup => {
                let backup = local_path.with_extension("toml.bak");
                std::fs::copy(&local_path, &backup)?;
                println!("    {DETAIL} Backed up to {}", backup.display());
            }
            OverwriteChoice::Skip => {
                println!("    Skipped.");
                return Ok(false);
            }
        }
    } else if !yes && !confirm_action("Apply provider updates?", true) {
        println!("    Skipped.");
        return Ok(false);
    }

    if let Some(parent) = local_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&local_path, &remote_content)?;

    let mut state = UpdateState::load();
    state.installed_content.providers = Some(ContentRecord {
        version: manifest.version.clone(),
        sha256: remote_hash,
        installed_at: now_iso(),
    });
    state.last_check = now_iso();
    state
        .save()
        .inspect_err(
            |e| tracing::warn!(error = %e, "failed to save update state after provider install"),
        )
        .ok();

    println!("    {OK} Provider configs updated to v{}", manifest.version);
    Ok(true)
}

fn providers_local_path(config_path: &str) -> PathBuf {
    if let Ok(content) = std::fs::read_to_string(config_path)
        && let Ok(config) = content.parse::<toml::Value>()
        && let Some(path) = config.get("providers_file").and_then(|v| v.as_str())
    {
        return PathBuf::from(path);
    }
    ironclad_home().join("providers.toml")
}

async fn apply_skills_update(
    yes: bool,
    registry_url: &str,
    config_path: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let (DIM, BOLD, _, GREEN, YELLOW, _, _, RESET, MONO) = colors();
    let (OK, _, WARN, DETAIL, _) = icons();
    let client = http_client()?;

    println!("\n  {BOLD}Skills{RESET}\n");

    let manifest = match fetch_manifest(&client, registry_url).await {
        Ok(m) => m,
        Err(e) => {
            println!("    {WARN} Could not fetch registry manifest: {e}");
            return Ok(false);
        }
    };

    let base_url = registry_base_url(registry_url);
    let state = UpdateState::load();
    let skills_dir = skills_local_dir(config_path);

    if !skills_dir.exists() {
        std::fs::create_dir_all(&skills_dir)?;
    }

    let mut new_files = Vec::new();
    let mut updated_unmodified = Vec::new();
    let mut updated_modified = Vec::new();
    let mut up_to_date = Vec::new();

    for (filename, remote_hash) in &manifest.packs.skills.files {
        // Path traversal guard: reject filenames containing ".." or absolute paths.
        if filename.contains("..") || Path::new(filename).is_absolute() {
            tracing::warn!(filename, "skipping manifest entry with suspicious path");
            continue;
        }

        let local_file = skills_dir.join(filename);
        let installed_hash = state
            .installed_content
            .skills
            .as_ref()
            .and_then(|s| s.files.get(filename))
            .cloned();

        if !local_file.exists() {
            new_files.push(filename.clone());
            continue;
        }

        let current_hash = file_sha256(&local_file).unwrap_or_default();
        if &current_hash == remote_hash {
            up_to_date.push(filename.clone());
            continue;
        }

        let user_modified = match &installed_hash {
            Some(ih) => current_hash != *ih,
            None => true,
        };

        if user_modified {
            updated_modified.push(filename.clone());
        } else {
            updated_unmodified.push(filename.clone());
        }
    }

    if new_files.is_empty() && updated_unmodified.is_empty() && updated_modified.is_empty() {
        println!(
            "    {OK} All skills are up to date ({} files)",
            up_to_date.len()
        );
        return Ok(false);
    }

    let total_changes = new_files.len() + updated_unmodified.len() + updated_modified.len();
    println!(
        "    {total_changes} change(s): {} new, {} updated, {} with local modifications",
        new_files.len(),
        updated_unmodified.len(),
        updated_modified.len()
    );
    println!();

    for f in &new_files {
        println!("    {GREEN}+ {f}{RESET} (new)");
    }
    for f in &updated_unmodified {
        println!("    {DIM}  {f}{RESET} (unmodified -- will auto-update)");
    }
    for f in &updated_modified {
        println!("    {YELLOW}  {f}{RESET} (YOU MODIFIED THIS FILE)");
    }

    println!();
    if !yes && !confirm_action("Apply skill updates?", true) {
        println!("    Skipped.");
        return Ok(false);
    }

    let mut applied = 0u32;
    let mut file_hashes: HashMap<String, String> = state
        .installed_content
        .skills
        .as_ref()
        .map(|s| s.files.clone())
        .unwrap_or_default();

    for filename in new_files.iter().chain(updated_unmodified.iter()) {
        let remote_content = fetch_file(
            &client,
            &base_url,
            &format!("{}{}", manifest.packs.skills.path, filename),
        )
        .await?;
        std::fs::write(skills_dir.join(filename), &remote_content)?;
        file_hashes.insert(filename.clone(), bytes_sha256(remote_content.as_bytes()));
        applied += 1;
    }

    for filename in &updated_modified {
        let local_file = skills_dir.join(filename);
        let local_content = std::fs::read_to_string(&local_file).unwrap_or_default();
        let remote_content = fetch_file(
            &client,
            &base_url,
            &format!("{}{}", manifest.packs.skills.path, filename),
        )
        .await?;

        println!();
        println!("    {YELLOW}{filename}{RESET} -- local modifications detected:");
        print_diff(&local_content, &remote_content);

        match confirm_overwrite(filename) {
            OverwriteChoice::Overwrite => {
                std::fs::write(&local_file, &remote_content)?;
                file_hashes.insert(filename.clone(), bytes_sha256(remote_content.as_bytes()));
                applied += 1;
            }
            OverwriteChoice::Backup => {
                let backup = local_file.with_extension("md.bak");
                std::fs::copy(&local_file, &backup)?;
                println!("    {DETAIL} Backed up to {}", backup.display());
                std::fs::write(&local_file, &remote_content)?;
                file_hashes.insert(filename.clone(), bytes_sha256(remote_content.as_bytes()));
                applied += 1;
            }
            OverwriteChoice::Skip => {
                println!("    Skipped {filename}.");
            }
        }
    }

    let mut state = UpdateState::load();
    state.installed_content.skills = Some(SkillsRecord {
        version: manifest.version.clone(),
        files: file_hashes,
        installed_at: now_iso(),
    });
    state.last_check = now_iso();
    state
        .save()
        .inspect_err(
            |e| tracing::warn!(error = %e, "failed to save update state after skills install"),
        )
        .ok();

    println!();
    println!(
        "    {OK} Applied {applied} skill update(s) (v{})",
        manifest.version
    );
    Ok(true)
}

// ── Multi-registry support ───────────────────────────────────

/// Compare two semver-style version strings.  Returns `true` when
/// `local >= remote`, meaning an update is unnecessary.  Gracefully
/// falls back to string comparison for non-numeric segments.
fn semver_gte(local: &str, remote: &str) -> bool {
    /// Decompose a version string into (core_parts, has_pre_release).
    /// Per semver, a pre-release version has *lower* precedence than the
    /// same core version without a pre-release suffix: 1.0.0-rc.1 < 1.0.0.
    fn parse(v: &str) -> (Vec<u64>, bool) {
        let v = v.trim_start_matches('v');
        // Strip build metadata first (has no effect on precedence).
        let v = v.split_once('+').map(|(core, _)| core).unwrap_or(v);
        // Detect and strip pre-release suffix.
        let (core, has_pre) = match v.split_once('-') {
            Some((c, _)) => (c, true),
            None => (v, false),
        };
        let parts = core
            .split('.')
            .map(|s| s.parse::<u64>().unwrap_or(0))
            .collect();
        (parts, has_pre)
    }
    let (l, l_pre) = parse(local);
    let (r, r_pre) = parse(remote);
    let len = l.len().max(r.len());
    for i in 0..len {
        let lv = l.get(i).copied().unwrap_or(0);
        let rv = r.get(i).copied().unwrap_or(0);
        match lv.cmp(&rv) {
            std::cmp::Ordering::Greater => return true,
            std::cmp::Ordering::Less => return false,
            std::cmp::Ordering::Equal => {}
        }
    }
    // Core versions are equal.  A pre-release is *less than* the release:
    // local=1.0.0-rc.1 vs remote=1.0.0  →  local < remote  →  false
    // local=1.0.0      vs remote=1.0.0-rc.1 → local > remote → true
    if l_pre && !r_pre {
        return false;
    }
    true
}

/// Apply skills updates from all configured registries.
///
/// Registries are processed in priority order (highest first). When two
/// registries publish a skill with the same filename, the higher-priority
/// one wins.  Non-default registries are namespaced into subdirectories
/// (e.g. `skills/community/`) so they coexist with the default set.
pub(crate) async fn apply_multi_registry_skills_update(
    yes: bool,
    cli_registry_override: Option<&str>,
    config_path: &str,
) -> Result<bool, Box<dyn std::error::Error>> {
    let (_, BOLD, _, _, _, _, _, RESET, _) = colors();
    let (OK, _, WARN, _, _) = icons();

    // If the user supplied a CLI override, fall through to the single-registry path.
    if let Some(url) = cli_registry_override {
        return apply_skills_update(yes, url, config_path).await;
    }

    // Parse just the [update] section to avoid requiring a full valid config.
    let registries = match std::fs::read_to_string(config_path).ok().and_then(|raw| {
        let table: toml::Value = toml::from_str(&raw).ok()?;
        let update_val = table.get("update")?.clone();
        let update_cfg: ironclad_core::config::UpdateConfig = update_val.try_into().ok()?;
        Some(update_cfg.resolve_registries())
    }) {
        Some(regs) => regs,
        None => {
            // Fallback: single default registry from legacy resolution.
            let url = resolve_registry_url(None, config_path);
            return apply_skills_update(yes, &url, config_path).await;
        }
    };

    // Only one "default" registry (the common case) — delegate directly.
    // Non-default registries always need the namespace logic below.
    if registries.len() <= 1
        && registries
            .first()
            .map(|r| r.name == "default")
            .unwrap_or(true)
    {
        let url = registries
            .first()
            .map(|r| r.url.as_str())
            .unwrap_or(DEFAULT_REGISTRY_URL);
        return apply_skills_update(yes, url, config_path).await;
    }

    // Multiple registries — process in priority order (highest first).
    let mut sorted = registries.clone();
    sorted.sort_by(|a, b| b.priority.cmp(&a.priority));

    println!("\n  {BOLD}Skills (multi-registry){RESET}\n");

    // Show configured registries and prompt before fetching from non-default sources.
    let non_default: Vec<_> = sorted
        .iter()
        .filter(|r| r.enabled && r.name != "default")
        .collect();
    if !non_default.is_empty() {
        for r in &non_default {
            println!(
                "    {WARN} Non-default registry: {BOLD}{}{RESET} ({})",
                r.name, r.url
            );
        }
        if !yes && !confirm_action("Install skills from non-default registries?", false) {
            println!("    Skipped non-default registries.");
            // Fall back to default-only.
            let url = sorted
                .iter()
                .find(|r| r.name == "default")
                .map(|r| r.url.as_str())
                .unwrap_or(DEFAULT_REGISTRY_URL);
            return apply_skills_update(yes, url, config_path).await;
        }
    }

    let client = http_client()?;
    let skills_dir = skills_local_dir(config_path);
    if !skills_dir.exists() {
        std::fs::create_dir_all(&skills_dir)?;
    }

    let state = UpdateState::load();
    let mut any_changed = false;
    // Track claimed filenames to resolve cross-registry conflicts.
    let mut claimed_files: HashMap<String, String> = HashMap::new();

    for reg in &sorted {
        if !reg.enabled {
            continue;
        }

        let manifest = match fetch_manifest(&client, &reg.url).await {
            Ok(m) => m,
            Err(e) => {
                println!(
                    "    {WARN} [{name}] Could not fetch manifest: {e}",
                    name = reg.name
                );
                continue;
            }
        };

        // Version-based skip: if local installed version >= remote, skip.
        let installed_version = state
            .installed_content
            .skills
            .as_ref()
            .map(|s| s.version.as_str())
            .unwrap_or("0.0.0");
        if semver_gte(installed_version, &manifest.version) {
            // Also verify all file hashes still match before declaring up-to-date.
            let all_match = manifest.packs.skills.files.iter().all(|(fname, hash)| {
                let local = skills_dir.join(fname);
                local.exists() && file_sha256(&local).unwrap_or_default() == *hash
            });
            if all_match {
                println!(
                    "    {OK} [{name}] All skills are up to date (v{ver})",
                    name = reg.name,
                    ver = manifest.version
                );
                continue;
            }
        }

        // Determine the target directory for this registry's files.
        // Guard: registry names must not contain path traversal components.
        if reg.name.contains("..") || reg.name.contains('/') || reg.name.contains('\\') {
            tracing::warn!(registry = %reg.name, "skipping registry with suspicious name");
            continue;
        }
        let target_dir = if reg.name == "default" {
            skills_dir.clone()
        } else {
            let ns_dir = skills_dir.join(&reg.name);
            if !ns_dir.exists() {
                std::fs::create_dir_all(&ns_dir)?;
            }
            ns_dir
        };

        let base_url = registry_base_url(&reg.url);
        let mut applied = 0u32;

        for (filename, remote_hash) in &manifest.packs.skills.files {
            // Path traversal guard: reject filenames containing ".." or absolute paths.
            // A malicious manifest could use "../../../etc/cron.d/evil" to escape the
            // skills directory.
            if filename.contains("..") || Path::new(filename).is_absolute() {
                tracing::warn!(
                    registry = %reg.name,
                    filename,
                    "skipping manifest entry with suspicious path"
                );
                continue;
            }

            // Cross-registry conflict: key on the resolved file path so that
            // different namespaced registries writing to different directories
            // don't falsely collide on the same bare filename.
            let resolved_key = target_dir.join(filename).to_string_lossy().to_string();
            if let Some(owner) = claimed_files.get(&resolved_key)
                && *owner != reg.name
            {
                continue;
            }
            claimed_files.insert(resolved_key, reg.name.clone());

            let local_file = target_dir.join(filename);
            if local_file.exists() {
                let current_hash = file_sha256(&local_file).unwrap_or_default();
                if current_hash == *remote_hash {
                    continue; // Already up to date.
                }
            }

            // Fetch and write the file.
            match fetch_file(
                &client,
                &base_url,
                &format!("{}{}", manifest.packs.skills.path, filename),
            )
            .await
            {
                Ok(content) => {
                    std::fs::write(&local_file, &content)?;
                    applied += 1;
                }
                Err(e) => {
                    println!(
                        "    {WARN} [{name}] Failed to fetch {filename}: {e}",
                        name = reg.name
                    );
                }
            }
        }

        if applied > 0 {
            any_changed = true;
            println!(
                "    {OK} [{name}] Applied {applied} skill update(s) (v{ver})",
                name = reg.name,
                ver = manifest.version
            );
        } else {
            println!(
                "    {OK} [{name}] All skills are up to date",
                name = reg.name
            );
        }
    }

    // Save updated state — record file hashes so the next run can skip unchanged files.
    // Without persisting `installed_content.skills`, the multi-registry path would
    // re-download every file on every run because it couldn't prove they're up-to-date.
    {
        let mut state = UpdateState::load();
        state.last_check = now_iso();
        if any_changed {
            // Build a merged file-hash map across all registries.
            let mut file_hashes: HashMap<String, String> = state
                .installed_content
                .skills
                .as_ref()
                .map(|s| s.files.clone())
                .unwrap_or_default();
            // Walk skills_dir to capture current on-disk hashes.
            if let Ok(entries) = std::fs::read_dir(&skills_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                            if let Ok(hash) = file_sha256(&path) {
                                file_hashes.insert(name.to_string(), hash);
                            }
                        }
                    }
                }
            }
            // Use the highest manifest version across registries.
            let max_version = sorted
                .iter()
                .filter(|r| r.enabled)
                .map(|r| r.name.as_str())
                .next()
                .unwrap_or("0.0.0");
            let _ = max_version; // We don't have per-registry versions cached; use "multi".
            state.installed_content.skills = Some(SkillsRecord {
                version: "multi".into(),
                files: file_hashes,
                installed_at: now_iso(),
            });
        }
        state
            .save()
            .inspect_err(
                |e| tracing::warn!(error = %e, "failed to save update state after multi-registry sync"),
            )
            .ok();
    }

    Ok(any_changed)
}

fn skills_local_dir(config_path: &str) -> PathBuf {
    if let Ok(content) = std::fs::read_to_string(config_path)
        && let Ok(config) = content.parse::<toml::Value>()
        && let Some(path) = config
            .get("skills")
            .and_then(|s| s.get("skills_dir"))
            .and_then(|v| v.as_str())
    {
        return PathBuf::from(path);
    }
    ironclad_home().join("skills")
}

// ── Public CLI entry points ──────────────────────────────────

pub async fn cmd_update_check(
    channel: &str,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, _, GREEN, _, _, _, RESET, MONO) = colors();
    let (OK, _, WARN, _, _) = icons();

    heading("Update Check");
    let current = env!("CARGO_PKG_VERSION");
    let client = http_client()?;

    // Binary
    println!("\n  {BOLD}Binary{RESET}");
    println!("    Current: {MONO}v{current}{RESET}");
    println!("    Channel: {DIM}{channel}{RESET}");

    match check_binary_version(&client).await? {
        Some(latest) => {
            if is_newer(&latest, current) {
                println!("    Latest:  {GREEN}v{latest}{RESET} (update available)");
            } else {
                println!("    {OK} Up to date (v{current})");
            }
        }
        None => println!("    {WARN} Could not check crates.io"),
    }

    // Content packs — resolve all configured registries (multi-registry aware).
    let registries: Vec<ironclad_core::config::RegistrySource> = if let Some(url) =
        registry_url_override
    {
        // CLI override → single registry.
        vec![ironclad_core::config::RegistrySource {
            name: "cli-override".into(),
            url: url.to_string(),
            priority: 100,
            enabled: true,
        }]
    } else {
        std::fs::read_to_string(config_path)
            .ok()
            .and_then(|raw| {
                let table: toml::Value = toml::from_str(&raw).ok()?;
                let update_val = table.get("update")?.clone();
                let update_cfg: ironclad_core::config::UpdateConfig = update_val.try_into().ok()?;
                Some(update_cfg.resolve_registries())
            })
            .unwrap_or_else(|| {
                // Fallback: legacy single-URL resolution.
                let url = resolve_registry_url(None, config_path);
                vec![ironclad_core::config::RegistrySource {
                    name: "default".into(),
                    url,
                    priority: 50,
                    enabled: true,
                }]
            })
    };

    let enabled: Vec<_> = registries.iter().filter(|r| r.enabled).collect();

    println!("\n  {BOLD}Content Packs{RESET}");
    if enabled.len() == 1 {
        println!("    Registry: {DIM}{}{RESET}", enabled[0].url);
    } else {
        for reg in &enabled {
            println!("    Registry: {DIM}{}{RESET} ({})", reg.url, reg.name);
        }
    }

    // Check the primary (first enabled) registry for providers + skills status.
    let primary_url = enabled
        .first()
        .map(|r| r.url.as_str())
        .unwrap_or(DEFAULT_REGISTRY_URL);

    match fetch_manifest(&client, primary_url).await {
        Ok(manifest) => {
            let state = UpdateState::load();
            println!("    Pack version: {MONO}v{}{RESET}", manifest.version);

            // Providers
            let providers_path = providers_local_path(config_path);
            if providers_path.exists() {
                let local_hash = file_sha256(&providers_path).unwrap_or_default();
                if local_hash == manifest.packs.providers.sha256 {
                    println!("    {OK} Providers: up to date");
                } else {
                    println!("    {GREEN}\u{25b6}{RESET} Providers: update available");
                }
            } else {
                println!("    {GREEN}+{RESET} Providers: new (not yet installed locally)");
            }

            // Skills
            let skills_dir = skills_local_dir(config_path);
            let mut skills_new = 0u32;
            let mut skills_changed = 0u32;
            let mut skills_ok = 0u32;
            for (filename, remote_hash) in &manifest.packs.skills.files {
                let local_file = skills_dir.join(filename);
                if !local_file.exists() {
                    skills_new += 1;
                } else {
                    let local_hash = file_sha256(&local_file).unwrap_or_default();
                    if local_hash == *remote_hash {
                        skills_ok += 1;
                    } else {
                        skills_changed += 1;
                    }
                }
            }

            if skills_new == 0 && skills_changed == 0 {
                println!("    {OK} Skills: up to date ({skills_ok} files)");
            } else {
                println!(
                    "    {GREEN}\u{25b6}{RESET} Skills: {skills_new} new, {skills_changed} changed, {skills_ok} current"
                );
            }

            // Check additional non-default registries for reachability.
            for reg in enabled.iter().skip(1) {
                match fetch_manifest(&client, &reg.url).await {
                    Ok(m) => println!("    {OK} {}: reachable (v{})", reg.name, m.version),
                    Err(e) => println!("    {WARN} {}: unreachable ({e})", reg.name),
                }
            }

            if let Some(ref providers) = state.installed_content.providers {
                println!(
                    "\n    {DIM}Last content update: {}{RESET}",
                    providers.installed_at
                );
            }
        }
        Err(e) => {
            println!("    {WARN} Could not reach registry: {e}");
        }
    }

    println!();
    Ok(())
}

pub async fn cmd_update_all(
    channel: &str,
    yes: bool,
    no_restart: bool,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, BOLD, _, _, _, _, _, RESET, _) = colors();
    let (OK, _, WARN, DETAIL, _) = icons();
    heading("Ironclad Update");

    // ── Liability Waiver ──────────────────────────────────────────
    println!();
    println!("    {BOLD}IMPORTANT — PLEASE READ{RESET}");
    println!();
    println!("    Ironclad is an autonomous AI agent that can execute actions,");
    println!("    interact with external services, and manage digital assets");
    println!("    including cryptocurrency wallets and on-chain transactions.");
    println!();
    println!("    THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND.");
    println!("    The developers and contributors bear {BOLD}no responsibility{RESET} for:");
    println!();
    println!("      - Actions taken by the agent, whether intended or unintended");
    println!("      - Loss of funds, income, cryptocurrency, or other digital assets");
    println!("      - Security vulnerabilities, compromises, or unauthorized access");
    println!("      - Damages arising from the agent's use, misuse, or malfunction");
    println!("      - Any financial, legal, or operational consequences whatsoever");
    println!();
    println!("    By proceeding, you acknowledge that you use Ironclad entirely");
    println!("    at your own risk and accept full responsibility for its operation.");
    println!();
    if !yes && !confirm_action("I understand and accept these terms", true) {
        println!("\n    Update cancelled.\n");
        return Ok(());
    }

    let binary_updated = apply_binary_update(yes, "download").await?;

    let registry_url = resolve_registry_url(registry_url_override, config_path);
    apply_providers_update(yes, &registry_url, config_path).await?;
    apply_multi_registry_skills_update(yes, registry_url_override, config_path).await?;
    run_oauth_storage_maintenance();
    run_mechanic_checks_maintenance(config_path);
    if let Err(e) = apply_removed_legacy_config_migration(config_path) {
        println!("    {WARN} Legacy config migration skipped: {e}");
    }

    // ── Post-upgrade security config migration ─────────────────────
    // Detect pre-RBAC configs (no [security] section) and warn about
    // the breaking change: empty allow-lists now deny all messages.
    if let Err(e) = apply_security_config_migration(config_path) {
        println!("    {WARN} Security config migration skipped: {e}");
    }

    // Restart the daemon if a binary update was applied and --no-restart was not passed.
    if binary_updated && !no_restart && crate::daemon::is_installed() {
        println!("\n    Restarting daemon to apply update...");
        match crate::daemon::restart_daemon() {
            Ok(()) => println!("    {OK} Daemon restarted"),
            Err(e) => {
                println!("    {WARN} Could not restart daemon: {e}");
                println!("    {DETAIL} Run `ironclad daemon restart` manually.");
            }
        }
    } else if binary_updated && no_restart {
        println!("\n    {DETAIL} Skipping daemon restart (--no-restart).");
        println!("    {DETAIL} Run `ironclad daemon restart` to apply the update.");
    }

    println!("\n  {BOLD}Update complete.{RESET}\n");
    Ok(())
}

pub async fn cmd_update_binary(
    _channel: &str,
    yes: bool,
    method: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    heading("Ironclad Binary Update");
    apply_binary_update(yes, method).await?;
    run_oauth_storage_maintenance();
    let config_path = ironclad_core::config::resolve_config_path(None)
        .unwrap_or_else(|| home_dir().join(".ironclad").join("ironclad.toml"));
    run_mechanic_checks_maintenance(&config_path.to_string_lossy());
    println!();
    Ok(())
}

pub async fn cmd_update_providers(
    yes: bool,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    heading("Provider Config Update");
    let registry_url = resolve_registry_url(registry_url_override, config_path);
    apply_providers_update(yes, &registry_url, config_path).await?;
    run_oauth_storage_maintenance();
    run_mechanic_checks_maintenance(config_path);
    println!();
    Ok(())
}

pub async fn cmd_update_skills(
    yes: bool,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    heading("Skills Update");
    apply_multi_registry_skills_update(yes, registry_url_override, config_path).await?;
    run_oauth_storage_maintenance();
    run_mechanic_checks_maintenance(config_path);
    println!();
    Ok(())
}

// ── Security config migration ────────────────────────────────

/// Detect pre-RBAC config files (missing `[security]` section) and auto-append
/// the section with explicit defaults. Also prints a breaking-change warning
/// about the new deny-by-default behavior for empty channel allow-lists.
fn apply_security_config_migration(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let path = Path::new(config_path);
    if !path.exists() {
        return Ok(());
    }

    let raw = std::fs::read_to_string(path)?;
    // Normalize line endings for reliable section detection.
    let normalized = raw.replace("\r\n", "\n").replace('\r', "\n");

    // Check if [security] section already exists (line-anchored, not substring).
    let has_security = normalized.lines().any(|line| line.trim() == "[security]");

    if has_security {
        return Ok(());
    }

    // ── Breaking change warning ──────────────────────────────
    let (_, BOLD, _, _, _, _, _, RESET, _) = super::colors();
    let (_, ERR, WARN, DETAIL, _) = super::icons();

    println!();
    println!("  {ERR} {BOLD}SECURITY MODEL CHANGE{RESET}");
    println!();
    println!(
        "    Empty channel allow-lists now {BOLD}DENY all messages{RESET} (previously allowed all)."
    );
    println!(
        "    This is a critical security fix — your agent was previously open to the internet."
    );
    println!();

    // Parse the config to show per-channel status.
    if let Ok(config) = ironclad_core::IroncladConfig::from_file(path) {
        let channels_status = describe_channel_allowlists(&config);
        if !channels_status.is_empty() {
            println!("    Your current configuration:");
            for line in &channels_status {
                println!("      {line}");
            }
            println!();
        }

        if config.channels.trusted_sender_ids.is_empty() {
            println!("    {WARN} trusted_sender_ids = [] (no Creator-level users configured)");
            println!();
        }
    }

    println!("    Run {BOLD}ironclad mechanic --repair{RESET} for guided security setup.");
    println!();

    // ── Auto-append [security] section with explicit defaults ─
    let security_section = r#"
# Security: Claim-based RBAC authority resolution.
# See `ironclad mechanic` for guided configuration.
[security]
deny_on_empty_allowlist = true  # empty allow-lists deny all messages (secure default)
allowlist_authority = "Peer"     # allow-listed senders get Peer authority
trusted_authority = "Creator"    # trusted_sender_ids get Creator authority
api_authority = "Creator"        # HTTP API callers get Creator authority
threat_caution_ceiling = "External"  # threat-flagged inputs are capped at External
"#;

    // Backup before modifying.
    let backup = path.with_extension("toml.bak");
    if !backup.exists() {
        std::fs::copy(path, &backup)?;
    }

    let mut content = normalized;
    content.push_str(security_section);

    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, &content)?;
    std::fs::rename(&tmp, path)?;

    println!("    {DETAIL} Added [security] section to {config_path} (backup: .toml.bak)");
    println!();

    Ok(())
}

/// Produce human-readable status lines for each configured channel's allow-list.
fn describe_channel_allowlists(config: &ironclad_core::IroncladConfig) -> Vec<String> {
    let mut lines = Vec::new();

    if let Some(ref tg) = config.channels.telegram {
        if tg.allowed_chat_ids.is_empty() {
            lines.push("Telegram: allowed_chat_ids = [] (was: open to all → now: deny all)".into());
        } else {
            lines.push(format!(
                "Telegram: {} chat ID(s) configured",
                tg.allowed_chat_ids.len()
            ));
        }
    }

    if let Some(ref dc) = config.channels.discord {
        if dc.allowed_guild_ids.is_empty() {
            lines.push("Discord: allowed_guild_ids = [] (was: open to all → now: deny all)".into());
        } else {
            lines.push(format!(
                "Discord: {} guild ID(s) configured",
                dc.allowed_guild_ids.len()
            ));
        }
    }

    if let Some(ref wa) = config.channels.whatsapp {
        if wa.allowed_numbers.is_empty() {
            lines.push("WhatsApp: allowed_numbers = [] (was: open to all → now: deny all)".into());
        } else {
            lines.push(format!(
                "WhatsApp: {} number(s) configured",
                wa.allowed_numbers.len()
            ));
        }
    }

    if let Some(ref sig) = config.channels.signal {
        if sig.allowed_numbers.is_empty() {
            lines.push("Signal: allowed_numbers = [] (was: open to all → now: deny all)".into());
        } else {
            lines.push(format!(
                "Signal: {} number(s) configured",
                sig.allowed_numbers.len()
            ));
        }
    }

    if !config.channels.email.allowed_senders.is_empty() {
        lines.push(format!(
            "Email: {} sender(s) configured",
            config.channels.email.allowed_senders.len()
        ));
    } else if config.channels.email.enabled {
        lines.push("Email: allowed_senders = [] (was: open to all → now: deny all)".into());
    }

    lines
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, extract::State, routing::get};
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};
    use tokio::net::TcpListener;

    #[derive(Clone)]
    struct MockRegistry {
        manifest: String,
        providers: String,
        skill_payload: String,
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: test-scoped environment mutation restored on Drop.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.old {
                // SAFETY: restoring previous process env value.
                unsafe { std::env::set_var(self.key, v) };
            } else {
                // SAFETY: restoring previous process env value.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    async fn start_mock_registry(
        providers: String,
        skill_draft: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let providers_hash = bytes_sha256(providers.as_bytes());
        let draft_hash = bytes_sha256(skill_draft.as_bytes());
        let manifest = serde_json::json!({
            "version": "0.8.0",
            "packs": {
                "providers": {
                    "sha256": providers_hash,
                    "path": "registry/providers.toml"
                },
                "skills": {
                    "sha256": null,
                    "path": "registry/skills/",
                    "files": {
                        "draft.md": draft_hash
                    }
                }
            }
        })
        .to_string();

        let state = MockRegistry {
            manifest,
            providers,
            skill_payload: skill_draft,
        };

        async fn manifest_h(State(st): State<MockRegistry>) -> Json<serde_json::Value> {
            Json(serde_json::from_str(&st.manifest).unwrap())
        }
        async fn providers_h(State(st): State<MockRegistry>) -> String {
            st.providers
        }
        async fn skill_h(State(st): State<MockRegistry>) -> String {
            st.skill_payload
        }

        let app = Router::new()
            .route("/manifest.json", get(manifest_h))
            .route("/registry/providers.toml", get(providers_h))
            .route("/registry/skills/draft.md", get(skill_h))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (
            format!("http://{}:{}/manifest.json", addr.ip(), addr.port()),
            handle,
        )
    }

    #[test]
    fn update_state_serde_roundtrip() {
        let state = UpdateState {
            binary_version: "0.2.0".into(),
            last_check: "2026-02-20T00:00:00Z".into(),
            registry_url: DEFAULT_REGISTRY_URL.into(),
            installed_content: InstalledContent {
                providers: Some(ContentRecord {
                    version: "0.2.0".into(),
                    sha256: "abc123".into(),
                    installed_at: "2026-02-20T00:00:00Z".into(),
                }),
                skills: Some(SkillsRecord {
                    version: "0.2.0".into(),
                    files: {
                        let mut m = HashMap::new();
                        m.insert("draft.md".into(), "hash1".into());
                        m.insert("rust.md".into(), "hash2".into());
                        m
                    },
                    installed_at: "2026-02-20T00:00:00Z".into(),
                }),
            },
        };

        let json = serde_json::to_string_pretty(&state).unwrap();
        let parsed: UpdateState = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.binary_version, "0.2.0");
        assert_eq!(
            parsed.installed_content.providers.as_ref().unwrap().sha256,
            "abc123"
        );
        assert_eq!(
            parsed
                .installed_content
                .skills
                .as_ref()
                .unwrap()
                .files
                .len(),
            2
        );
    }

    #[test]
    fn update_state_default_is_empty() {
        let state = UpdateState::default();
        assert_eq!(state.binary_version, "");
        assert!(state.installed_content.providers.is_none());
        assert!(state.installed_content.skills.is_none());
    }

    #[test]
    fn file_sha256_computes_correctly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        std::fs::write(&path, "hello world\n").unwrap();

        let hash = file_sha256(&path).unwrap();
        assert_eq!(hash.len(), 64);

        let expected = bytes_sha256(b"hello world\n");
        assert_eq!(hash, expected);
    }

    #[test]
    fn file_sha256_error_on_missing() {
        let result = file_sha256(Path::new("/nonexistent/file.txt"));
        assert!(result.is_err());
    }

    #[test]
    fn bytes_sha256_deterministic() {
        let h1 = bytes_sha256(b"test data");
        let h2 = bytes_sha256(b"test data");
        assert_eq!(h1, h2);
        assert_ne!(bytes_sha256(b"different"), h1);
    }

    #[test]
    fn modification_detection_unmodified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.toml");
        let content = "[providers.openai]\nurl = \"https://api.openai.com\"\n";
        std::fs::write(&path, content).unwrap();

        let installed_hash = bytes_sha256(content.as_bytes());
        let current_hash = file_sha256(&path).unwrap();
        assert_eq!(current_hash, installed_hash);
    }

    #[test]
    fn modification_detection_modified() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("providers.toml");
        let original = "[providers.openai]\nurl = \"https://api.openai.com\"\n";
        let modified = "[providers.openai]\nurl = \"https://custom.endpoint.com\"\n";

        let installed_hash = bytes_sha256(original.as_bytes());
        std::fs::write(&path, modified).unwrap();

        let current_hash = file_sha256(&path).unwrap();
        assert_ne!(current_hash, installed_hash);
    }

    #[test]
    fn manifest_parse() {
        let json = r#"{
            "version": "0.2.0",
            "packs": {
                "providers": { "sha256": "abc123", "path": "registry/providers.toml" },
                "skills": {
                    "sha256": null,
                    "path": "registry/skills/",
                    "files": {
                        "draft.md": "hash1",
                        "rust.md": "hash2"
                    }
                }
            }
        }"#;
        let manifest: RegistryManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.packs.providers.sha256, "abc123");
        assert_eq!(manifest.packs.skills.files.len(), 2);
        assert_eq!(manifest.packs.skills.files["draft.md"], "hash1");
    }

    #[test]
    fn diff_lines_identical() {
        let result = diff_lines("a\nb\nc", "a\nb\nc");
        assert!(result.iter().all(|l| matches!(l, DiffLine::Same(_))));
    }

    #[test]
    fn diff_lines_changed() {
        let result = diff_lines("a\nb\nc", "a\nB\nc");
        assert_eq!(result.len(), 4);
        assert_eq!(result[0], DiffLine::Same("a".into()));
        assert_eq!(result[1], DiffLine::Removed("b".into()));
        assert_eq!(result[2], DiffLine::Added("B".into()));
        assert_eq!(result[3], DiffLine::Same("c".into()));
    }

    #[test]
    fn diff_lines_added() {
        let result = diff_lines("a\nb", "a\nb\nc");
        assert_eq!(result.len(), 3);
        assert_eq!(result[2], DiffLine::Added("c".into()));
    }

    #[test]
    fn diff_lines_removed() {
        let result = diff_lines("a\nb\nc", "a\nb");
        assert_eq!(result.len(), 3);
        assert_eq!(result[2], DiffLine::Removed("c".into()));
    }

    #[test]
    fn diff_lines_empty_to_content() {
        let result = diff_lines("", "a\nb");
        assert!(result.iter().any(|l| matches!(l, DiffLine::Added(_))));
    }

    #[test]
    fn semver_parse_basic() {
        assert_eq!(parse_semver("1.2.3"), (1, 2, 3));
        assert_eq!(parse_semver("v0.1.0"), (0, 1, 0));
        assert_eq!(parse_semver("10.20.30"), (10, 20, 30));
    }

    #[test]
    fn is_newer_works() {
        assert!(is_newer("0.2.0", "0.1.0"));
        assert!(is_newer("1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
    }

    #[test]
    fn registry_base_url_strips_filename() {
        let url = "https://roboticus.ai/registry/manifest.json";
        assert_eq!(registry_base_url(url), "https://roboticus.ai/registry");
    }

    #[test]
    fn resolve_registry_url_cli_override() {
        let result = resolve_registry_url(
            Some("https://custom.registry/manifest.json"),
            "nonexistent.toml",
        );
        assert_eq!(result, "https://custom.registry/manifest.json");
    }

    #[test]
    fn resolve_registry_url_default() {
        let result = resolve_registry_url(None, "nonexistent.toml");
        assert_eq!(result, DEFAULT_REGISTRY_URL);
    }

    #[test]
    fn resolve_registry_url_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("ironclad.toml");
        std::fs::write(
            &config,
            "[update]\nregistry_url = \"https://my.registry/manifest.json\"\n",
        )
        .unwrap();

        let result = resolve_registry_url(None, config.to_str().unwrap());
        assert_eq!(result, "https://my.registry/manifest.json");
    }

    #[test]
    fn update_state_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("update_state.json");

        let state = UpdateState {
            binary_version: "0.3.0".into(),
            last_check: "2026-03-01T12:00:00Z".into(),
            registry_url: "https://example.com/manifest.json".into(),
            installed_content: InstalledContent::default(),
        };

        let json = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(&path, &json).unwrap();

        let loaded: UpdateState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.binary_version, "0.3.0");
        assert_eq!(loaded.registry_url, "https://example.com/manifest.json");
    }

    #[test]
    fn bytes_sha256_empty_input() {
        let hash = bytes_sha256(b"");
        assert_eq!(hash.len(), 64);
        assert_eq!(
            hash,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn parse_semver_partial_version() {
        assert_eq!(parse_semver("1"), (1, 0, 0));
        assert_eq!(parse_semver("1.2"), (1, 2, 0));
    }

    #[test]
    fn parse_semver_empty() {
        assert_eq!(parse_semver(""), (0, 0, 0));
    }

    #[test]
    fn parse_semver_with_v_prefix() {
        assert_eq!(parse_semver("v1.2.3"), (1, 2, 3));
    }

    #[test]
    fn parse_semver_ignores_build_and_prerelease_metadata() {
        assert_eq!(parse_semver("0.9.4+hotfix.1"), (0, 9, 4));
        assert_eq!(parse_semver("v1.2.3-rc.1"), (1, 2, 3));
    }

    #[test]
    fn is_newer_patch_bump() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn is_newer_same_version() {
        assert!(!is_newer("1.0.0", "1.0.0"));
    }

    #[test]
    fn platform_archive_name_supported() {
        let name = platform_archive_name("1.2.3");
        if let Some(n) = name {
            assert!(n.contains("ironclad-1.2.3-"));
        }
    }

    #[test]
    fn diff_lines_both_empty() {
        let result = diff_lines("", "");
        assert!(result.is_empty() || result.iter().all(|l| matches!(l, DiffLine::Same(_))));
    }

    #[test]
    fn diff_lines_content_to_empty() {
        let result = diff_lines("a\nb", "");
        assert!(result.iter().any(|l| matches!(l, DiffLine::Removed(_))));
    }

    #[test]
    fn registry_base_url_no_slash() {
        assert_eq!(registry_base_url("manifest.json"), "manifest.json");
    }

    #[test]
    fn registry_base_url_nested() {
        assert_eq!(
            registry_base_url("https://cdn.example.com/v1/registry/manifest.json"),
            "https://cdn.example.com/v1/registry"
        );
    }

    #[test]
    fn installed_content_default_is_empty() {
        let ic = InstalledContent::default();
        assert!(ic.skills.is_none());
        assert!(ic.providers.is_none());
    }

    #[test]
    fn parse_sha256sums_for_artifact_finds_exact_entry() {
        let sums = "\
abc123  ironclad-0.8.0-darwin-aarch64.tar.gz\n\
def456  ironclad-0.8.0-linux-x86_64.tar.gz\n";
        let hash = parse_sha256sums_for_artifact(sums, "ironclad-0.8.0-linux-x86_64.tar.gz");
        assert_eq!(hash.as_deref(), Some("def456"));
    }

    #[test]
    fn find_file_recursive_finds_nested_target() {
        let dir = tempfile::tempdir().unwrap();
        let nested = dir.path().join("a").join("b");
        std::fs::create_dir_all(&nested).unwrap();
        let target = nested.join("needle.txt");
        std::fs::write(&target, "x").unwrap();
        let found = find_file_recursive(dir.path(), "needle.txt").unwrap();
        assert_eq!(found.as_deref(), Some(target.as_path()));
    }

    #[test]
    fn local_path_helpers_fallback_when_config_missing() {
        let p = providers_local_path("/no/such/file.toml");
        let s = skills_local_dir("/no/such/file.toml");
        assert!(p.ends_with("providers.toml"));
        assert!(s.ends_with("skills"));
    }

    #[test]
    fn parse_sha256sums_for_artifact_returns_none_when_missing() {
        let sums = "abc123  file-a.tar.gz\n";
        assert!(parse_sha256sums_for_artifact(sums, "file-b.tar.gz").is_none());
    }

    #[test]
    fn select_release_for_download_prefers_exact_tag() {
        let archive = platform_archive_name("0.9.4").unwrap();
        let releases = vec![
            GitHubRelease {
                tag_name: "v0.9.4+hotfix.1".into(),
                draft: false,
                prerelease: false,
                published_at: Some("2026-03-05T11:36:51Z".into()),
                assets: vec![
                    GitHubAsset {
                        name: "SHA256SUMS.txt".into(),
                    },
                    GitHubAsset {
                        name: format!(
                            "ironclad-0.9.4+hotfix.1-{}",
                            &archive["ironclad-0.9.4-".len()..]
                        ),
                    },
                ],
            },
            GitHubRelease {
                tag_name: "v0.9.4".into(),
                draft: false,
                prerelease: false,
                published_at: Some("2026-03-05T10:00:00Z".into()),
                assets: vec![
                    GitHubAsset {
                        name: "SHA256SUMS.txt".into(),
                    },
                    GitHubAsset {
                        name: archive.clone(),
                    },
                ],
            },
        ];

        let selected = select_release_for_download(&releases, "0.9.4");
        assert_eq!(
            selected.as_ref().map(|(tag, _)| tag.as_str()),
            Some("v0.9.4")
        );
    }

    #[test]
    fn select_release_for_download_falls_back_to_hotfix_tag() {
        let archive = platform_archive_name("0.9.4").unwrap();
        let suffix = &archive["ironclad-0.9.4-".len()..];
        let releases = vec![
            GitHubRelease {
                tag_name: "v0.9.4".into(),
                draft: false,
                prerelease: false,
                published_at: Some("2026-03-05T10:00:00Z".into()),
                assets: vec![GitHubAsset {
                    name: "PROVENANCE.json".into(),
                }],
            },
            GitHubRelease {
                tag_name: "v0.9.4+hotfix.2".into(),
                draft: false,
                prerelease: false,
                published_at: Some("2026-03-05T12:00:00Z".into()),
                assets: vec![
                    GitHubAsset {
                        name: "SHA256SUMS.txt".into(),
                    },
                    GitHubAsset {
                        name: format!("ironclad-0.9.4+hotfix.2-{suffix}"),
                    },
                ],
            },
        ];

        let selected = select_release_for_download(&releases, "0.9.4");
        let expected_archive = format!("ironclad-0.9.4+hotfix.2-{suffix}");
        assert_eq!(
            selected.as_ref().map(|(tag, _)| tag.as_str()),
            Some("v0.9.4+hotfix.2")
        );
        assert_eq!(
            selected
                .as_ref()
                .map(|(_, archive_name)| archive_name.as_str()),
            Some(expected_archive.as_str())
        );
    }

    #[test]
    fn find_file_recursive_returns_none_when_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let found = find_file_recursive(dir.path(), "does-not-exist.txt").unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn apply_providers_update_fetches_and_writes_local_file() {
        let _lock = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let config_path = temp.path().join("ironclad.toml");
        let providers_path = temp.path().join("providers.toml");
        std::fs::write(
            &config_path,
            format!("providers_file = \"{}\"\n", providers_path.display()),
        )
        .unwrap();

        let providers = "[providers.openai]\nurl = \"https://api.openai.com\"\n".to_string();
        let (registry_url, handle) =
            start_mock_registry(providers.clone(), "# hello\nbody\n".to_string()).await;

        let changed = apply_providers_update(true, &registry_url, config_path.to_str().unwrap())
            .await
            .unwrap();
        assert!(changed);
        assert_eq!(std::fs::read_to_string(&providers_path).unwrap(), providers);

        let changed_second =
            apply_providers_update(true, &registry_url, config_path.to_str().unwrap())
                .await
                .unwrap();
        assert!(!changed_second);
        handle.abort();
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn apply_skills_update_installs_and_then_reports_up_to_date() {
        let _lock = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let skills_dir = temp.path().join("skills");
        let config_path = temp.path().join("ironclad.toml");
        std::fs::write(
            &config_path,
            format!("[skills]\nskills_dir = \"{}\"\n", skills_dir.display()),
        )
        .unwrap();

        let draft = "# draft\nfrom registry\n".to_string();
        let (registry_url, handle) = start_mock_registry(
            "[providers.openai]\nurl=\"https://api.openai.com\"\n".to_string(),
            draft.clone(),
        )
        .await;

        let changed = apply_skills_update(true, &registry_url, config_path.to_str().unwrap())
            .await
            .unwrap();
        assert!(changed);
        assert_eq!(
            std::fs::read_to_string(skills_dir.join("draft.md")).unwrap(),
            draft
        );

        let changed_second =
            apply_skills_update(true, &registry_url, config_path.to_str().unwrap())
                .await
                .unwrap();
        assert!(!changed_second);
        handle.abort();
    }

    // ── semver_gte tests ────────────────────────────────────────

    #[test]
    fn semver_gte_equal_versions() {
        assert!(semver_gte("1.0.0", "1.0.0"));
    }

    #[test]
    fn semver_gte_local_newer() {
        assert!(semver_gte("1.1.0", "1.0.0"));
        assert!(semver_gte("2.0.0", "1.9.9"));
        assert!(semver_gte("0.9.6", "0.9.5"));
    }

    #[test]
    fn semver_gte_local_older() {
        assert!(!semver_gte("1.0.0", "1.0.1"));
        assert!(!semver_gte("0.9.5", "0.9.6"));
        assert!(!semver_gte("0.8.9", "0.9.0"));
    }

    #[test]
    fn semver_gte_different_segment_counts() {
        assert!(semver_gte("1.0.0", "1.0"));
        assert!(semver_gte("1.0", "1.0.0"));
        assert!(!semver_gte("1.0", "1.0.1"));
    }

    #[test]
    fn semver_gte_strips_prerelease_and_build_metadata() {
        // Per semver spec: pre-release has LOWER precedence than its release.
        // 1.0.0-rc.1 < 1.0.0
        assert!(!semver_gte("1.0.0-rc.1", "1.0.0"));
        assert!(semver_gte("1.0.0", "1.0.0-rc.1"));
        // Build metadata: "1.0.0+hotfix.1" should compare as 1.0.0
        assert!(semver_gte("1.0.0+build.42", "1.0.0"));
        assert!(semver_gte("1.0.0", "1.0.0+build.42"));
        // Combined: pre-release + build metadata → still pre-release < release
        assert!(!semver_gte("1.0.0-rc.1+build.42", "1.0.0"));
        // v prefix with pre-release
        assert!(!semver_gte("v1.0.0-rc.1", "1.0.0"));
        assert!(!semver_gte("v0.9.5-beta.1", "0.9.6"));
        // Two pre-releases with same core version — both are pre-release, so equal core → true
        assert!(semver_gte("1.0.0-rc.2", "1.0.0-rc.1"));
    }

    // ── Multi-registry test ─────────────────────────────────────

    /// Helper to start a mock registry that serves skills under a given namespace.
    async fn start_namespaced_mock_registry(
        registry_name: &str,
        skill_filename: &str,
        skill_content: String,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let content_hash = bytes_sha256(skill_content.as_bytes());
        let manifest = serde_json::json!({
            "version": "1.0.0",
            "packs": {
                "providers": {
                    "sha256": "unused",
                    "path": "registry/providers.toml"
                },
                "skills": {
                    "sha256": null,
                    "path": format!("registry/{registry_name}/"),
                    "files": {
                        skill_filename: content_hash
                    }
                }
            }
        })
        .to_string();

        let skill_route = format!("/registry/{registry_name}/{skill_filename}");

        let state = MockRegistry {
            manifest,
            providers: String::new(),
            skill_payload: skill_content,
        };

        async fn manifest_h(State(st): State<MockRegistry>) -> Json<serde_json::Value> {
            Json(serde_json::from_str(&st.manifest).unwrap())
        }
        async fn providers_h(State(st): State<MockRegistry>) -> String {
            st.providers.clone()
        }
        async fn skill_h(State(st): State<MockRegistry>) -> String {
            st.skill_payload.clone()
        }

        let app = Router::new()
            .route("/manifest.json", get(manifest_h))
            .route("/registry/providers.toml", get(providers_h))
            .route(&skill_route, get(skill_h))
            .with_state(state);
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (
            format!("http://{}:{}/manifest.json", addr.ip(), addr.port()),
            handle,
        )
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn multi_registry_namespaces_non_default_skills() {
        let _lock = env_lock().lock().unwrap();
        let temp = tempfile::tempdir().unwrap();
        let _home_guard = EnvGuard::set("HOME", temp.path().to_str().unwrap());
        let skills_dir = temp.path().join("skills");
        let config_path = temp.path().join("ironclad.toml");

        let skill_content = "# community skill\nbody\n".to_string();
        let (registry_url, handle) =
            start_namespaced_mock_registry("community", "helper.md", skill_content.clone()).await;

        // Write a config file with a multi-registry setup.
        let config_toml = format!(
            r#"[skills]
skills_dir = "{}"

[update]
registry_url = "{}"

[[update.registries]]
name = "community"
url = "{}"
priority = 40
enabled = true
"#,
            skills_dir.display(),
            registry_url,
            registry_url,
        );
        std::fs::write(&config_path, &config_toml).unwrap();

        let changed = apply_multi_registry_skills_update(true, None, config_path.to_str().unwrap())
            .await
            .unwrap();

        assert!(changed);
        // Skill should be namespaced under community/ subdirectory.
        let namespaced_path = skills_dir.join("community").join("helper.md");
        assert!(
            namespaced_path.exists(),
            "expected skill at {}, files in skills_dir: {:?}",
            namespaced_path.display(),
            std::fs::read_dir(&skills_dir)
                .map(|rd| rd.flatten().map(|e| e.path()).collect::<Vec<_>>())
                .unwrap_or_default()
        );
        assert_eq!(
            std::fs::read_to_string(&namespaced_path).unwrap(),
            skill_content
        );

        handle.abort();
    }
}
