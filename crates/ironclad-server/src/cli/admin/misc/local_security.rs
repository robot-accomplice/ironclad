fn resolve_security_audit_config_path(config_path: &str) -> std::path::PathBuf {
    if config_path == "ironclad.toml" {
        return ironclad_core::resolve_config_path(None)
            .unwrap_or_else(|| std::path::PathBuf::from("ironclad.toml"));
    }
    std::path::PathBuf::from(config_path)
}

pub fn cmd_security_audit(config_path: &str, json: bool) -> Result<(), Box<dyn std::error::Error>> {
    let mut findings: Vec<serde_json::Value> = Vec::new();
    let mut pass_count = 0u32;
    let mut warn_count = 0u32;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut fail_count = 0u32;

    let resolved_config_path = resolve_security_audit_config_path(config_path);
    let config_file = resolved_config_path.as_path();

    // 1. Check config file permissions
    if config_file.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(config_file)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                findings.push(serde_json::json!({
                    "check": "config_permissions",
                    "status": "fail",
                    "detail": format!("Config file is world/group-readable (mode {:o})", mode & 0o777),
                    "fix": format!("chmod 600 {}", config_file.display()),
                }));
                fail_count += 1;
            } else {
                findings.push(serde_json::json!({
                    "check": "config_permissions",
                    "status": "pass",
                    "detail": format!("mode {:o}", mode & 0o777),
                }));
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            findings.push(serde_json::json!({
                "check": "config_permissions",
                "status": "warn",
                "detail": "non-Unix platform",
            }));
            warn_count += 1;
        }
    } else {
        findings.push(serde_json::json!({
            "check": "config_permissions",
            "status": "warn",
            "detail": format!("Config file not found: {}", config_file.display()),
        }));
        warn_count += 1;
    }

    // 2. Check for API keys in config
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        let has_plaintext_key =
            content.contains("api_key") && !content.contains("${") && !content.contains("env(");
        if has_plaintext_key {
            findings.push(serde_json::json!({
                "check": "plaintext_api_keys",
                "status": "warn",
                "detail": "Plaintext API keys found in config",
                "fix": "Use environment variables instead",
            }));
            warn_count += 1;
        } else {
            findings.push(serde_json::json!({
                "check": "plaintext_api_keys",
                "status": "pass",
            }));
            pass_count += 1;
        }
    }

    // 3. Check bind address
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("bind = \"0.0.0.0\"") {
            findings.push(serde_json::json!({
                "check": "bind_address",
                "status": "warn",
                "detail": "Server bound to 0.0.0.0 (all interfaces)",
                "fix": "Bind to 127.0.0.1 unless external access is needed",
            }));
            warn_count += 1;
        } else {
            findings.push(serde_json::json!({
                "check": "bind_address",
                "status": "pass",
            }));
            pass_count += 1;
        }
    }

    // 4. Check wallet file permissions
    let wallet_path = ironclad_core::home_dir()
        .join(".ironclad")
        .join("wallet.json");
    if wallet_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&wallet_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                findings.push(serde_json::json!({
                    "check": "wallet_permissions",
                    "status": "fail",
                    "detail": format!("Wallet file is world/group-readable (mode {:o})", mode & 0o777),
                    "fix": format!("chmod 600 {}", wallet_path.display()),
                }));
                fail_count += 1;
            } else {
                findings.push(serde_json::json!({
                    "check": "wallet_permissions",
                    "status": "pass",
                    "detail": format!("mode {:o}", mode & 0o777),
                }));
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            findings.push(serde_json::json!({
                "check": "wallet_permissions",
                "status": "warn",
                "detail": "non-Unix platform",
            }));
            warn_count += 1;
        }
    }

    // 5. Check database file permissions
    let db_path = ironclad_core::home_dir().join(".ironclad").join("state.db");
    if db_path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(&db_path)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                findings.push(serde_json::json!({
                    "check": "database_permissions",
                    "status": "warn",
                    "detail": format!("Database is world/group-readable (mode {:o})", mode & 0o777),
                    "fix": format!("chmod 600 {}", db_path.display()),
                }));
                warn_count += 1;
            } else {
                findings.push(serde_json::json!({
                    "check": "database_permissions",
                    "status": "pass",
                }));
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            findings.push(serde_json::json!({
                "check": "database_permissions",
                "status": "warn",
                "detail": "non-Unix platform",
            }));
            warn_count += 1;
        }
    }

    // 6. Check CORS settings
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("cors") && content.contains("\"*\"") {
            findings.push(serde_json::json!({
                "check": "cors",
                "status": "warn",
                "detail": "CORS allows all origins (\"*\")",
                "fix": "Restrict CORS to specific origins in production",
            }));
            warn_count += 1;
        } else {
            findings.push(serde_json::json!({
                "check": "cors",
                "status": "pass",
            }));
            pass_count += 1;
        }
    }

    // 7. Check PID file
    let pid_path = ironclad_core::home_dir()
        .join(".ironclad")
        .join("ironclad.pid");
    if pid_path.exists() {
        findings.push(serde_json::json!({
            "check": "pid_file",
            "status": "pass",
        }));
        pass_count += 1;
    }

    let total = pass_count + warn_count + fail_count;

    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "pass": pass_count,
                "warn": warn_count,
                "fail": fail_count,
                "total": total,
                "findings": findings,
            }))?
        );
        return Ok(());
    }

    // Human-readable output
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Security Audit{RESET}\n");

    for f in &findings {
        let check = f["check"].as_str().unwrap_or("");
        let status = f["status"].as_str().unwrap_or("");
        let detail = f["detail"].as_str().unwrap_or("");
        let fix = f["fix"].as_str().unwrap_or("");
        match status {
            "fail" => {
                println!("  {RED}{ERR} FAIL{RESET} {detail}");
                if !fix.is_empty() {
                    println!("         Fix: {fix}");
                }
            }
            "warn" => {
                println!("  {WARN} {detail}");
                if !fix.is_empty() {
                    println!("         Recommendation: {fix}");
                }
            }
            "pass" => {
                let label = check.replace('_', " ");
                if detail.is_empty() {
                    println!("  {OK} {label}");
                } else {
                    println!("  {OK} {label} ({detail})");
                }
            }
            _ => {}
        }
    }

    println!();
    if fail_count > 0 {
        println!(
            "  {RED}{ERR}{RESET} {fail_count} failure(s), {warn_count} warning(s), {pass_count} passed out of {total} checks"
        );
    } else if warn_count > 0 {
        println!("  {WARN} {warn_count} warning(s), {pass_count} passed out of {total} checks");
    } else {
        println!("  {OK} All {total} checks passed");
    }
    println!();

    Ok(())
}
