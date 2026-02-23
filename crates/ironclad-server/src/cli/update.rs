use std::collections::HashMap;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{colors, heading, icons};
use crate::cli::{CRT_DRAW_MS, theme};

const DEFAULT_REGISTRY_URL: &str = "https://registry.roboticus.ai/manifest.json";
const CRATES_IO_API: &str = "https://crates.io/api/v1/crates/ironclad-server";
const CRATE_NAME: &str = "ironclad-server";

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
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(_) => Self::default(),
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
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home)
        .join(".ironclad")
        .join("update_state.json")
}

fn ironclad_home() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    PathBuf::from(home).join(".ironclad")
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

fn resolve_registry_url(cli_override: Option<&str>, config_path: &str) -> String {
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

fn registry_base_url(manifest_url: &str) -> String {
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

// ── Version comparison ───────────────────────────────────────

fn parse_semver(v: &str) -> (u32, u32, u32) {
    let v = v.trim_start_matches('v');
    let parts: Vec<&str> = v.split('.').collect();
    let major = parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minor = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let patch = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    (major, minor, patch)
}

fn is_newer(remote: &str, local: &str) -> bool {
    parse_semver(remote) > parse_semver(local)
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

async fn check_binary_version(
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

async fn apply_binary_update(yes: bool) -> Result<bool, Box<dyn std::error::Error>> {
    let (DIM, BOLD, _, GREEN, _, _, _, RESET, MONO) = colors();
    let (OK, _, WARN, _, ERR) = icons();
    let current = env!("CARGO_PKG_VERSION");
    let client = http_client()?;

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

    if !yes && !confirm_action("Proceed with binary update?", true) {
        println!("    Skipped.");
        return Ok(false);
    }

    println!("    Installing v{latest} via cargo install...");
    println!("    {DIM}This may take a few minutes.{RESET}");

    let status = std::process::Command::new("cargo")
        .args(["install", CRATE_NAME, "--locked"])
        .status();

    match status {
        Ok(s) if s.success() => {
            println!("    {OK} Binary updated to v{latest}");
            let mut state = UpdateState::load();
            state.binary_version = latest;
            state.last_check = now_iso();
            state.save().ok();
            Ok(true)
        }
        Ok(s) => {
            println!(
                "    {ERR} cargo install exited with code {}",
                s.code().unwrap_or(-1)
            );
            Ok(false)
        }
        Err(e) => {
            println!("    {ERR} Failed to run cargo install: {e}");
            println!("    {DIM}Ensure cargo is in your PATH{RESET}");
            Ok(false)
        }
    }
}

// ── Content update (providers + skills) ──────────────────────

async fn fetch_manifest(
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
    state.save().ok();

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
    state.save().ok();

    println!();
    println!(
        "    {OK} Applied {applied} skill update(s) (v{})",
        manifest.version
    );
    Ok(true)
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

    // Content packs
    let registry_url = resolve_registry_url(registry_url_override, config_path);

    println!("\n  {BOLD}Content Packs{RESET}");
    println!("    Registry: {DIM}{registry_url}{RESET}");

    match fetch_manifest(&client, &registry_url).await {
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
    _no_restart: bool,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let (_, BOLD, _, _, _, _, _, RESET, _) = colors();
    heading("Ironclad Update");

    apply_binary_update(yes).await?;

    let registry_url = resolve_registry_url(registry_url_override, config_path);
    apply_providers_update(yes, &registry_url, config_path).await?;
    apply_skills_update(yes, &registry_url, config_path).await?;

    println!("\n  {BOLD}Update complete.{RESET}\n");
    Ok(())
}

pub async fn cmd_update_binary(
    _channel: &str,
    yes: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    heading("Ironclad Binary Update");
    apply_binary_update(yes).await?;
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
    println!();
    Ok(())
}

pub async fn cmd_update_skills(
    yes: bool,
    registry_url_override: Option<&str>,
    config_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    heading("Skills Update");
    let registry_url = resolve_registry_url(registry_url_override, config_path);
    apply_skills_update(yes, &registry_url, config_path).await?;
    println!();
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

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
                        m.insert("hello.md".into(), "hash1".into());
                        m.insert("plan.md".into(), "hash2".into());
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
                        "hello.md": "hash1",
                        "summarize.md": "hash2"
                    }
                }
            }
        }"#;
        let manifest: RegistryManifest = serde_json::from_str(json).unwrap();
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.packs.providers.sha256, "abc123");
        assert_eq!(manifest.packs.skills.files.len(), 2);
        assert_eq!(manifest.packs.skills.files["hello.md"], "hash1");
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
        let url = "https://registry.roboticus.ai/manifest.json";
        assert_eq!(registry_base_url(url), "https://registry.roboticus.ai");
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
    fn is_newer_patch_bump() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn is_newer_same_version() {
        assert!(!is_newer("1.0.0", "1.0.0"));
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
}
