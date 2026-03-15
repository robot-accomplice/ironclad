use super::*;

use ironclad_plugin_sdk::manifest::PluginManifest;
use sha2::{Digest, Sha256};

// ── Install source detection ────────────────────────────────────

enum InstallSource {
    /// Local directory containing plugin.toml (dev mode)
    Directory(std::path::PathBuf),
    /// Local .ic.zip archive
    Archive(std::path::PathBuf),
    /// Catalog plugin name (fetched from registry)
    Catalog(String),
}

fn detect_source(source: &str) -> InstallSource {
    let path = std::path::Path::new(source);
    let has_path_sep = source.contains('/') || source.contains('\\');
    let is_zip = path.extension().and_then(|e| e.to_str()) == Some("zip");

    if is_zip {
        // Anything ending in .zip is treated as an archive path
        InstallSource::Archive(path.to_path_buf())
    } else if has_path_sep || path.exists() {
        // Contains path separators or exists on disk → filesystem directory
        InstallSource::Directory(path.to_path_buf())
    } else {
        // Bare name like "claude-code" → catalog lookup
        InstallSource::Catalog(source.to_string())
    }
}

// ── Plugin listing ────────────────────────────────────────────

pub async fn cmd_plugins_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = super::http_client()?
        .get(format!("{base_url}/api/plugins"))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await?;

    let plugins = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if plugins.is_empty() {
        println!("\n  No plugins installed.\n");
        return Ok(());
    }

    println!(
        "\n  {:<20} {:<10} {:<10} {:<10}",
        "Plugin", "Version", "Status", "Tools"
    );
    println!("  {}", "─".repeat(55));
    for p in &plugins {
        let name = p.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let version = p.get("version").and_then(|v| v.as_str()).unwrap_or("?");
        let status = p.get("status").and_then(|v| v.as_str()).unwrap_or("?");
        let tools = p
            .get("tools")
            .and_then(|v| v.as_array())
            .map(|a| a.len())
            .unwrap_or(0);
        println!(
            "  {:<20} {:<10} {:<10} {:<10}",
            name, version, status, tools
        );
    }
    println!();
    Ok(())
}

// ── Plugin info ─────────────────────────────────────────────

pub async fn cmd_plugin_info(base_url: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (_dim, bold, _accent, green, yellow, red, _cyan, reset, _mono) = colors();
    let (ok, _action, _warn, _detail, _err_icon) = icons();
    let resp = super::http_client()?
        .get(format!("{base_url}/api/plugins"))
        .send()
        .await?;
    let body: serde_json::Value = resp.json().await.unwrap_or_else(|e| {
        tracing::warn!("failed to parse plugin info response: {e}");
        serde_json::Value::default()
    });
    let plugins: Vec<serde_json::Value> = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let plugin = plugins
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(name));

    match plugin {
        Some(p) => {
            println!("\n  {bold}Plugin: {name}{reset}\n");
            if let Some(v) = p.get("version").and_then(|v| v.as_str()) {
                println!("  Version:     {v}");
            }
            if let Some(d) = p.get("description").and_then(|v| v.as_str()) {
                println!("  Description: {d}");
            }
            let status = p
                .get("status")
                .and_then(|v| v.as_str())
                .map(|s| s.to_ascii_lowercase())
                .or_else(|| {
                    p.get("enabled").and_then(|v| v.as_bool()).map(|b| {
                        if b {
                            "active".to_string()
                        } else {
                            "disabled".to_string()
                        }
                    })
                })
                .unwrap_or_else(|| "unknown".to_string());
            println!(
                "  Status:      {}",
                if status == "active" || status == "loaded" {
                    format!("{green}{status}{reset}")
                } else if status == "disabled" || status == "error" {
                    format!("{red}{status}{reset}")
                } else {
                    format!("{yellow}{status}{reset}")
                }
            );
            if let Some(path) = p.get("manifest_path").and_then(|v| v.as_str()) {
                println!("  Manifest:    {path}");
            }
            if let Some(tools) = p.get("tools").and_then(|v| v.as_array()) {
                println!("  Tools:       {}", tools.len());
                for tool in tools {
                    if let Some(tn) = tool.get("name").and_then(|v| v.as_str()) {
                        println!("    {ok} {tn}");
                    }
                }
            }
            println!();
        }
        None => {
            eprintln!("  Plugin not found: {name}");
            return Err(format!("plugin not found: {name}").into());
        }
    }
    Ok(())
}

// ── Shared helpers ──────────────────────────────────────────

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        if ty.is_symlink() {
            continue;
        }
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else if ty.is_file() {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

pub(crate) fn companion_skill_install_name(plugin_name: &str, skill_rel: &str) -> String {
    let skill_filename = std::path::Path::new(skill_rel)
        .file_name()
        .unwrap_or_default()
        .to_string_lossy();
    let hash = Sha256::digest(skill_rel.as_bytes());
    let short = hex::encode(&hash[..6]);
    format!("{plugin_name}--{short}--{skill_filename}")
}

fn check_requirements(manifest: &PluginManifest) -> bool {
    let (_dim, bold, _accent, green, yellow, red, cyan, reset, _mono) = colors();
    let (ok, action, warn, _detail, err_icon) = icons();

    if manifest.requirements.is_empty() {
        return true;
    }

    println!(
        "\n  {action} Checking requirements for {bold}{}{reset}...\n",
        manifest.name
    );
    let results = manifest.check_requirements();
    let mut has_missing_required = false;

    for (req, found) in &results {
        if *found {
            println!(
                "    {ok} {green}{}{reset} ({}) — found",
                req.name, req.command
            );
        } else if req.optional {
            println!(
                "    {warn} {yellow}{}{reset} ({}) — not found (optional)",
                req.name, req.command
            );
        } else {
            has_missing_required = true;
            println!(
                "    {err_icon} {red}{}{reset} ({}) — not found",
                req.name, req.command
            );
            if let Some(hint) = &req.install_hint {
                println!("      Install: {cyan}{hint}{reset}");
            }
        }
    }
    println!();

    if has_missing_required {
        eprintln!(
            "  {err_icon} Cannot install {}: missing required dependencies.",
            manifest.name
        );
        eprintln!("  Install the missing requirements above and try again.\n");
        return false;
    }
    true
}

fn check_companion_skills_exist(manifest: &PluginManifest, source_dir: &std::path::Path) -> bool {
    let (_dim, _bold, _accent, _green, _yellow, _red, _cyan, _reset, _mono) = colors();
    let (_ok, _action, _warn, _detail, err_icon) = icons();

    for skill_path in &manifest.companion_skills {
        let full = source_dir.join(skill_path);
        if !full.exists() {
            eprintln!("  {err_icon} Companion skill not found in bundle: {skill_path}");
            return false;
        }
    }
    true
}

fn check_not_installed(plugin_name: &str) -> Result<std::path::PathBuf, ()> {
    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let plugins_dir = ironclad_dir.join("plugins");
    let dest = plugins_dir.join(plugin_name);

    if dest.exists() {
        eprintln!("  Plugin already installed: {plugin_name}");
        eprintln!("  Uninstall first with: ironclad plugins uninstall {plugin_name}");
        return Err(());
    }
    Ok(dest)
}

fn deploy_companion_skills(
    manifest: &PluginManifest,
    source_dir: &std::path::Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let (ok, _action, _warn, _detail, _err_icon) = icons();
    if manifest.companion_skills.is_empty() {
        return Ok(());
    }

    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let skills_dir = ironclad_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let mut installed = Vec::new();
    for skill_rel in &manifest.companion_skills {
        let src_skill = source_dir.join(skill_rel);
        let installed_name = companion_skill_install_name(&manifest.name, skill_rel);
        let dest_skill = skills_dir.join(&installed_name);

        if let Err(e) = std::fs::copy(&src_skill, &dest_skill) {
            // best-effort: rollback cleanup on install failure
            for path in installed.iter().rev() {
                let _ = std::fs::remove_file(path);
            }
            return Err(Box::new(e));
        }
        installed.push(dest_skill);
        println!("  {ok} Installed companion skill: {installed_name}");
    }
    Ok(())
}

fn print_plugin_summary(manifest: &PluginManifest, source_label: &str) {
    let (_dim, bold, _accent, green, _yellow, _red, _cyan, reset, _mono) = colors();
    let (ok, _action, _warn, _detail, _err_icon) = icons();

    println!("\n  {ok} Installed plugin: {bold}{}{reset}", manifest.name);
    println!("  Version: {green}{}{reset}", manifest.version);
    if !manifest.description.is_empty() {
        println!("  {}", truncate_str(&manifest.description, 72));
    }
    println!("  Source:  {source_label}");
    println!("  Restart the server to activate.\n");
}

fn truncate_str(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.len() <= max {
        return s.to_string();
    }
    // Walk char boundaries to find the last safe index within budget
    let end = s
        .char_indices()
        .map(|(i, _)| i)
        .take(max)
        .last()
        .unwrap_or(0);
    format!("{}…", &s[..end])
}

fn prompt_yes_no(prompt: &str) -> bool {
    use std::io::Write;
    print!("  {prompt} [y/N] ");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

// ── Install: unified entry point ────────────────────────────

pub async fn cmd_plugin_install(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    match detect_source(source) {
        InstallSource::Directory(path) => install_from_directory(&path),
        InstallSource::Archive(path) => install_from_archive(&path),
        InstallSource::Catalog(name) => install_from_catalog(&name).await,
    }
}

// ── Install from local directory (dev mode) ─────────────────

fn install_from_directory(source_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let (_dim, _bold, _accent, _green, _yellow, _red, _cyan, _reset, _mono) = colors();
    let (_ok, _action, warn, _detail, err_icon) = icons();

    if !source_path.exists() {
        return Err(format!("source not found: {}", source_path.display()).into());
    }

    let manifest_path = source_path.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!("no plugin.toml found in {}", source_path.display()).into());
    }

    let manifest = PluginManifest::from_file(&manifest_path)
        .map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    // Vet the plugin before installing (same gate as pack and server startup)
    let report = manifest.vet(source_path);
    for w in &report.warnings {
        eprintln!("    {warn} {w}");
    }
    if !report.is_ok() {
        for e in &report.errors {
            eprintln!("    {err_icon} {e}");
        }
        eprintln!("\n  {err_icon} Plugin failed vetting. Fix errors above before installing.\n");
        return Err("plugin vetting failed".into());
    }

    if !check_requirements(&manifest) {
        return Err("missing required plugin dependencies".into());
    }
    if !check_companion_skills_exist(&manifest, source_path) {
        return Err("companion skill files missing from plugin bundle".into());
    }
    let dest = match check_not_installed(&manifest.name) {
        Ok(d) => d,
        Err(()) => return Err(format!("plugin '{}' already installed", manifest.name).into()),
    };

    std::fs::create_dir_all(&dest)?;
    if let Err(e) = copy_dir_recursive(source_path, &dest) {
        // best-effort: rollback cleanup on install failure
        let _ = std::fs::remove_dir_all(&dest);
        return Err(Box::new(e));
    }

    if let Err(e) = deploy_companion_skills(&manifest, source_path) {
        // best-effort: rollback cleanup on install failure
        let _ = std::fs::remove_dir_all(&dest);
        return Err(e);
    }
    print_plugin_summary(&manifest, &format!("directory: {}", source_path.display()));
    Ok(())
}

// ── Install from .ic.zip archive ────────────────────────────

fn install_from_archive(archive_path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    use ironclad_plugin_sdk::archive;

    let (_dim, bold, _accent, green, _yellow, _red, cyan, reset, _mono) = colors();
    let (ok, action, _warn, _detail, err_icon) = icons();

    if !archive_path.exists() {
        return Err(format!("archive not found: {}", archive_path.display()).into());
    }

    println!("\n  {action} Unpacking {}...", archive_path.display());

    // Unpack to staging area
    let staging_dir = ironclad_core::home_dir().join(".ironclad").join("staging");
    std::fs::create_dir_all(&staging_dir)?;

    let result = archive::unpack(archive_path, &staging_dir)
        .map_err(|e| format!("Failed to unpack archive: {e}"))?;

    println!(
        "  {ok} Unpacked {bold}{}{reset} v{green}{}{reset} ({} files)",
        result.manifest.name, result.manifest.version, result.file_count
    );
    println!("  {ok} SHA-256: {cyan}{}{reset}", &result.sha256[..16]);

    // Requirements check
    if !check_requirements(&result.manifest) {
        // best-effort: staging cleanup on early exit
        let _ = std::fs::remove_dir_all(&result.dest_dir);
        return Err("missing required plugin dependencies".into());
    }

    // Check not already installed
    let dest = match check_not_installed(&result.manifest.name) {
        Ok(d) => d,
        Err(()) => {
            // best-effort: staging cleanup on early exit
            let _ = std::fs::remove_dir_all(&result.dest_dir);
            return Err(format!("plugin '{}' already installed", result.manifest.name).into());
        }
    };

    // Prompt user
    if !prompt_yes_no(&format!(
        "Install {} v{}?",
        result.manifest.name, result.manifest.version
    )) {
        println!("  Cancelled.");
        let _ = std::fs::remove_dir_all(&result.dest_dir);
        return Ok(());
    }

    // Move from staging to plugins dir
    std::fs::create_dir_all(dest.parent().unwrap_or(&dest))?;
    std::fs::rename(&result.dest_dir, &dest).or_else(|_| {
        // rename fails across filesystems; fall back to copy + remove
        if let Err(e) = copy_dir_recursive(&result.dest_dir, &dest) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(e);
        }
        std::fs::remove_dir_all(&result.dest_dir)
    })?;

    if let Err(e) = deploy_companion_skills(&result.manifest, &dest) {
        // Roll back partial install on post-move failure.
        if let Err(clean_err) = std::fs::remove_dir_all(&dest) {
            eprintln!(
                "  {err_icon} Companion skill deployment failed and rollback also failed: {clean_err}"
            );
        }
        return Err(e);
    }
    print_plugin_summary(
        &result.manifest,
        &format!("archive: {}", archive_path.display()),
    );
    Ok(())
}

// ── Install from remote catalog ─────────────────────────────

async fn install_from_catalog(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::cli::update;
    use ironclad_plugin_sdk::archive;

    let (_dim, bold, _accent, green, _yellow, red, cyan, reset, _mono) = colors();
    let (ok, action, _warn, _detail, err_icon) = icons();

    println!("\n  {action} Searching catalog for {bold}{name}{reset}...");

    // Fetch registry manifest
    let config_path = ironclad_core::config::resolve_config_path(None);
    let config_str = config_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let registry_url = update::resolve_registry_url(None, &config_str);
    let client = super::http_client()?;
    let manifest = update::fetch_manifest(&client, &registry_url).await?;

    let catalog = manifest
        .packs
        .plugins
        .as_ref()
        .ok_or("No plugin catalog available in the registry")?;

    let entry = catalog
        .find(name)
        .ok_or_else(|| format!("Plugin '{name}' not found in catalog"))?;

    println!(
        "  {ok} Found: {bold}{}{reset} v{green}{}{reset}",
        entry.name, entry.version
    );
    println!("  {}", truncate_str(&entry.description, 72));
    println!("  Author: {}", entry.author);
    println!("  Tier:   {}", entry.tier);

    // Check not already installed
    if check_not_installed(&entry.name).is_err() {
        return Err(format!("plugin '{}' already installed", entry.name).into());
    }

    // Prompt before download
    if !prompt_yes_no(&format!(
        "Download and install {} v{}?",
        entry.name, entry.version
    )) {
        println!("  Cancelled.");
        return Ok(());
    }

    // Download archive
    let base_url = update::registry_base_url(&registry_url);
    let archive_url = format!("{base_url}/{}", entry.path);

    println!("  {action} Downloading from {cyan}{archive_url}{reset}...");

    let client = super::http_client()?;
    let resp = client.get(&archive_url).send().await?;

    if !resp.status().is_success() {
        return Err(format!("download failed: HTTP {}", resp.status()).into());
    }

    let bytes = resp.bytes().await?;
    println!("  {ok} Downloaded {} bytes", bytes.len());

    // Verify checksum against catalog
    println!("  {action} Verifying SHA-256...");
    archive::verify_bytes_checksum(&bytes, &entry.sha256)
        .map_err(|e| format!("Checksum verification failed: {e}"))?;
    println!(
        "  {ok} Checksum verified: {cyan}{}{reset}",
        &entry.sha256[..16]
    );

    // Unpack to staging
    let staging_dir = ironclad_core::home_dir().join(".ironclad").join("staging");
    std::fs::create_dir_all(&staging_dir)?;

    let result = archive::unpack_bytes(&bytes, &staging_dir, entry.sha256.clone())
        .map_err(|e| format!("Failed to unpack archive: {e}"))?;

    // Identity check: manifest name must match catalog entry name
    if result.manifest.name != entry.name {
        let _ = std::fs::remove_dir_all(&result.dest_dir);
        return Err(format!(
            "identity mismatch: catalog says '{}' but archive contains '{}'",
            entry.name, result.manifest.name
        )
        .into());
    }

    // Re-check "already installed" against manifest name (authoritative identity)
    if check_not_installed(&result.manifest.name).is_err() {
        let _ = std::fs::remove_dir_all(&result.dest_dir);
        return Err(format!("plugin '{}' already installed", result.manifest.name).into());
    }

    // Requirements check
    if !check_requirements(&result.manifest) {
        let _ = std::fs::remove_dir_all(&result.dest_dir);
        return Err("missing required plugin dependencies".into());
    }

    // Move from staging to plugins dir
    let dest = ironclad_core::home_dir()
        .join(".ironclad")
        .join("plugins")
        .join(&result.manifest.name);
    std::fs::create_dir_all(dest.parent().unwrap_or(&dest))?;
    std::fs::rename(&result.dest_dir, &dest).or_else(|_| {
        if let Err(e) = copy_dir_recursive(&result.dest_dir, &dest) {
            let _ = std::fs::remove_dir_all(&dest);
            return Err(e);
        }
        std::fs::remove_dir_all(&result.dest_dir)
    })?;

    if let Err(e) = deploy_companion_skills(&result.manifest, &dest) {
        // Roll back partial install on post-move failure.
        if let Err(clean_err) = std::fs::remove_dir_all(&dest) {
            eprintln!(
                "  {err_icon} Companion skill deployment failed and rollback also failed: {clean_err}"
            );
        }
        return Err(e);
    }
    print_plugin_summary(&result.manifest, &format!("catalog: {name}"));
    Ok(())
}

// ── Uninstall ───────────────────────────────────────────────

pub fn cmd_plugin_uninstall(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (_dim, _bold, _accent, _green, _yellow, _red, _cyan, _reset, _mono) = colors();
    let (ok, _action, warn, _detail, _err_icon) = icons();
    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");
    let plugin_dir = ironclad_dir.join("plugins").join(name);

    if !plugin_dir.exists() {
        eprintln!("  Plugin not found: {name}");
        return Err(format!("plugin not found: {name}").into());
    }

    // Remove companion skills if the manifest declares them
    let manifest_path = plugin_dir.join("plugin.toml");
    if manifest_path.exists()
        && let Ok(manifest) = PluginManifest::from_file(&manifest_path)
    {
        let skills_dir = ironclad_dir.join("skills");
        for skill_rel in &manifest.companion_skills {
            let installed_name = companion_skill_install_name(name, skill_rel);
            let skill_path = skills_dir.join(&installed_name);
            if skill_path.exists() {
                if let Err(e) = std::fs::remove_file(&skill_path) {
                    eprintln!("  {warn} Could not remove companion skill {installed_name}: {e}",);
                } else {
                    println!("  {ok} Removed companion skill: {installed_name}");
                }
            } else {
                // Backward compat: legacy flat naming — only remove if content matches
                let legacy_name = std::path::Path::new(skill_rel)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let old_prefixed_name = format!("{name}--{legacy_name}");
                let legacy_path = skills_dir.join(&legacy_name);
                let old_prefixed_path = skills_dir.join(&old_prefixed_name);
                let source_path = plugin_dir.join(skill_rel);
                let same_content = std::fs::read(&legacy_path)
                    .ok()
                    .zip(std::fs::read(&source_path).ok())
                    .map(|(a, b)| a == b)
                    .unwrap_or(false);
                if same_content {
                    if let Err(e) = std::fs::remove_file(&legacy_path) {
                        eprintln!(
                            "  {warn} Could not remove legacy companion skill {legacy_name}: {e}",
                        );
                    } else {
                        println!("  {ok} Removed legacy companion skill: {legacy_name}");
                    }
                }
                let old_prefixed_same_content = std::fs::read(&old_prefixed_path)
                    .ok()
                    .zip(std::fs::read(&source_path).ok())
                    .map(|(a, b)| a == b)
                    .unwrap_or(false);
                if old_prefixed_same_content {
                    if let Err(e) = std::fs::remove_file(&old_prefixed_path) {
                        eprintln!(
                            "  {warn} Could not remove legacy companion skill {old_prefixed_name}: {e}",
                        );
                    } else {
                        println!("  {ok} Removed legacy companion skill: {old_prefixed_name}");
                    }
                }
            }
        }
    }

    // Remove companion skills if the manifest declares them
    let manifest_path = plugin_dir.join("plugin.toml");
    if manifest_path.exists()
        && let Ok(manifest) = PluginManifest::from_file(&manifest_path)
    {
        let skills_dir = ironclad_dir.join("skills");
        for skill_rel in &manifest.companion_skills {
            let installed_name = companion_skill_install_name(name, skill_rel);
            let skill_path = skills_dir.join(&installed_name);
            if skill_path.exists() {
                if let Err(e) = std::fs::remove_file(&skill_path) {
                    eprintln!("  {warn} Could not remove companion skill {installed_name}: {e}",);
                } else {
                    println!("  {ok} Removed companion skill: {installed_name}");
                }
            } else {
                // Backward compat: legacy flat naming — only remove if content matches
                let legacy_name = std::path::Path::new(skill_rel)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let old_prefixed_name = format!("{name}--{legacy_name}");
                let legacy_path = skills_dir.join(&legacy_name);
                let old_prefixed_path = skills_dir.join(&old_prefixed_name);
                let source_path = plugin_dir.join(skill_rel);
                let same_content = std::fs::read(&legacy_path)
                    .ok()
                    .zip(std::fs::read(&source_path).ok())
                    .map(|(a, b)| a == b)
                    .unwrap_or(false);
                if same_content {
                    if let Err(e) = std::fs::remove_file(&legacy_path) {
                        eprintln!(
                            "  {warn} Could not remove legacy companion skill {legacy_name}: {e}",
                        );
                    } else {
                        println!("  {ok} Removed legacy companion skill: {legacy_name}");
                    }
                }
                let old_prefixed_same_content = std::fs::read(&old_prefixed_path)
                    .ok()
                    .zip(std::fs::read(&source_path).ok())
                    .map(|(a, b)| a == b)
                    .unwrap_or(false);
                if old_prefixed_same_content {
                    if let Err(e) = std::fs::remove_file(&old_prefixed_path) {
                        eprintln!(
                            "  {warn} Could not remove legacy companion skill {old_prefixed_name}: {e}",
                        );
                    } else {
                        println!("  {ok} Removed legacy companion skill: {old_prefixed_name}");
                    }
                }
            }
        }
    }

    std::fs::remove_dir_all(&plugin_dir)?;
    println!("  {ok} Uninstalled plugin: {name}");
    println!("  Restart the server to apply.\n");
    Ok(())
}

// ── Toggle enable/disable ───────────────────────────────────

pub async fn cmd_plugin_toggle(
    base_url: &str,
    name: &str,
    enable: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (ok, _action, _warn, _detail, _err_icon) = icons();
    let action = if enable { "enable" } else { "disable" };
    let client = super::http_client()?;
    let resp = client
        .put(format!("{base_url}/api/plugins/{name}/toggle"))
        .json(&serde_json::json!({ "enabled": enable }))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("  {ok} Plugin {name} {action}d");
    } else {
        eprintln!("  Failed to {action} plugin {name}: {}", resp.status());
        return Err(format!("failed to {action} plugin {name}: HTTP {}", resp.status()).into());
    }
    Ok(())
}

// ── Search remote catalog ───────────────────────────────────

pub async fn cmd_plugin_search(query: &str) -> Result<(), Box<dyn std::error::Error>> {
    use crate::cli::update;

    let (_dim, bold, _accent, green, yellow, _red, cyan, reset, _mono) = colors();
    let (ok, action, _warn, _detail, _err_icon) = icons();

    println!("\n  {action} Searching plugin catalog...\n");

    let config_path = ironclad_core::config::resolve_config_path(None);
    let config_str = config_path
        .as_ref()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    let registry_url = update::resolve_registry_url(None, &config_str);
    let client = super::http_client()?;
    let manifest = update::fetch_manifest(&client, &registry_url).await?;

    let catalog = manifest
        .packs
        .plugins
        .as_ref()
        .ok_or("No plugin catalog available in the registry")?;

    let results = catalog.search(query);

    if results.is_empty() {
        println!("  No plugins found matching \"{query}\".\n");
        return Ok(());
    }

    println!(
        "  {:<20} {:<10} {:<12} {}",
        "Name", "Version", "Tier", "Description"
    );
    println!("  {}", "─".repeat(70));
    for entry in &results {
        let tier_display = match entry.tier.as_str() {
            "official" => format!("{green}official{reset}"),
            "community" => format!("{yellow}community{reset}"),
            _ => entry.tier.clone(),
        };
        println!(
            "  {:<20} {:<10} {:<12} {}",
            entry.name,
            entry.version,
            tier_display,
            truncate_str(&entry.description, 40)
        );
    }
    println!(
        "\n  {ok} {} plugin(s) found. Install with: {cyan}ironclad plugins install <name>{reset}\n",
        results.len()
    );
    Ok(())
}

// ── Pack a plugin directory into .ic.zip ────────────────────

pub fn cmd_plugin_pack(dir: &str, output: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    use ironclad_plugin_sdk::archive;

    let (_dim, bold, _accent, green, _yellow, _red, cyan, reset, _mono) = colors();
    let (ok, action, _warn, _detail, err_icon) = icons();

    let source_path = std::path::Path::new(dir);
    if !source_path.exists() {
        return Err(format!("source directory not found: {dir}").into());
    }

    let manifest_path = source_path.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(format!("no plugin.toml found in {dir}").into());
    }

    // Vet the plugin before packing
    let manifest = PluginManifest::from_file(&manifest_path)
        .map_err(|e| format!("Invalid plugin.toml: {e}"))?;

    println!(
        "\n  {action} Vetting {bold}{}{reset} v{green}{}{reset}...\n",
        manifest.name, manifest.version
    );

    let report = manifest.vet(source_path);
    let has_problems = !report.errors.is_empty() || !report.warnings.is_empty();
    if has_problems {
        for err in &report.errors {
            eprintln!("    {err_icon} {err}");
        }
        let (_ok2, _action2, warn2, _detail2, _err2) = icons();
        for w in &report.warnings {
            eprintln!("    {warn2} {w}");
        }
        if !report.errors.is_empty() {
            eprintln!(
                "\n  {err_icon} Plugin failed vetting. Fix the errors above before packing.\n"
            );
            return Err("plugin vetting failed".into());
        }
        println!();
    }

    let output_dir = output
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    println!("  {action} Packing archive...");

    let result = archive::pack(source_path, &output_dir)
        .map_err(|e| format!("Failed to pack archive: {e}"))?;

    println!(
        "  {ok} Created: {bold}{}{reset}",
        result.archive_path.display()
    );
    println!("  SHA-256:  {cyan}{}{reset}", result.sha256);
    println!("  Files:    {}", result.file_count);
    println!(
        "  Size:     {} bytes (uncompressed)\n",
        result.uncompressed_bytes
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::companion_skill_install_name;

    #[test]
    fn companion_skill_install_name_distinguishes_paths_with_same_basename() {
        let a = companion_skill_install_name("plugin-a", "skills/core/readme.md");
        let b = companion_skill_install_name("plugin-a", "skills/extra/readme.md");
        assert_ne!(a, b);
    }
}
