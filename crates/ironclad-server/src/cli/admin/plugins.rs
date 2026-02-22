use super::*;

// ── Plugin listing ────────────────────────────────────────────

pub async fn cmd_plugins_list(base_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let resp = reqwest::get(format!("{base_url}/api/plugins")).await?;
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

    println!("\n  {:<20} {:<10} {:<10} {:<10}", "Plugin", "Version", "Status", "Tools");
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
            name,
            version,
            status,
            tools
        );
    }
    println!();
    Ok(())
}

// ── Plugin info, install, uninstall, toggle ────────────────────

pub async fn cmd_plugin_info(base_url: &str, name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let resp = reqwest::get(format!("{base_url}/api/plugins")).await?;
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    let plugins: Vec<serde_json::Value> = body
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let plugin = plugins.iter().find(|p| {
        p.get("name").and_then(|v| v.as_str()) == Some(name)
    });

    match plugin {
        Some(p) => {
            println!("\n  {BOLD}Plugin: {name}{RESET}\n");
            if let Some(v) = p.get("version").and_then(|v| v.as_str()) {
                println!("  Version:     {v}");
            }
            if let Some(d) = p.get("description").and_then(|v| v.as_str()) {
                println!("  Description: {d}");
            }
            let enabled = p.get("enabled").and_then(|v| v.as_bool()).unwrap_or(false);
            println!("  Status:      {}", if enabled { format!("{GREEN}enabled{RESET}") } else { format!("{RED}disabled{RESET}") });
            if let Some(path) = p.get("manifest_path").and_then(|v| v.as_str()) {
                println!("  Manifest:    {path}");
            }
            if let Some(tools) = p.get("tools").and_then(|v| v.as_array()) {
                println!("  Tools:       {}", tools.len());
                for tool in tools {
                    if let Some(tn) = tool.get("name").and_then(|v| v.as_str()) {
                        println!("    - {tn}");
                    }
                }
            }
            println!();
        }
        None => {
            eprintln!("  Plugin not found: {name}");
        }
    }
    Ok(())
}

fn copy_dir_recursive(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    if !dst.exists() {
        std::fs::create_dir_all(dst)?;
    }
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dest_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&entry.path(), &dest_path)?;
        } else {
            std::fs::copy(entry.path(), &dest_path)?;
        }
    }
    Ok(())
}

pub fn cmd_plugin_install(source: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let source_path = std::path::Path::new(source);
    if !source_path.exists() {
        eprintln!("  Source not found: {source}");
        return Ok(());
    }

    let manifest_path = source_path.join("plugin.toml");
    if !manifest_path.exists() {
        eprintln!("  No plugin.toml found in {source}");
        return Ok(());
    }

    let manifest_content = std::fs::read_to_string(&manifest_path)?;
    let manifest: toml::Value = manifest_content.parse()?;
    let plugin_name = manifest.get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let plugins_dir = std::path::PathBuf::from(&home).join(".ironclad").join("plugins");
    let dest = plugins_dir.join(plugin_name);

    if dest.exists() {
        eprintln!("  Plugin already installed: {plugin_name}");
        eprintln!("  Uninstall first with: ironclad plugins uninstall {plugin_name}");
        return Ok(());
    }

    std::fs::create_dir_all(&dest)?;
    copy_dir_recursive(source_path, &dest)?;

    println!("  {OK} Installed plugin: {plugin_name}");
    println!("  Location: {}", dest.display());
    println!("  Restart the server to activate.\n");
    Ok(())
}

pub fn cmd_plugin_uninstall(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
    let plugin_dir = std::path::PathBuf::from(&home)
        .join(".ironclad")
        .join("plugins")
        .join(name);

    if !plugin_dir.exists() {
        eprintln!("  Plugin not found: {name}");
        return Ok(());
    }

    std::fs::remove_dir_all(&plugin_dir)?;
    println!("  {OK} Uninstalled plugin: {name}");
    println!("  Restart the server to apply.\n");
    Ok(())
}

pub async fn cmd_plugin_toggle(
    base_url: &str,
    name: &str,
    enable: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let action = if enable { "enable" } else { "disable" };
    let client = reqwest::Client::new();
    let resp = client
        .put(format!("{base_url}/api/plugins/{name}/toggle"))
        .json(&serde_json::json!({ "enabled": enable }))
        .send()
        .await?;

    if resp.status().is_success() {
        println!("  {OK} Plugin {name} {action}d");
    } else {
        eprintln!("  Failed to {action} plugin {name}: {}", resp.status());
    }
    Ok(())
}
