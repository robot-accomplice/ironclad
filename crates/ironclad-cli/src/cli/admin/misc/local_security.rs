fn resolve_security_audit_config_path(config_path: &str) -> std::path::PathBuf {
    if config_path == "ironclad.toml" {
        return ironclad_core::resolve_config_path(None)
            .unwrap_or_else(|| std::path::PathBuf::from("ironclad.toml"));
    }
    std::path::PathBuf::from(config_path)
}

pub fn cmd_security_audit(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Security Audit{RESET}\n");

    let mut pass_count = 0u32;
    let mut warn_count = 0u32;
    #[cfg_attr(not(unix), allow(unused_mut))]
    let mut fail_count = 0u32;

    // 1. Check config file permissions
    let resolved_config_path = resolve_security_audit_config_path(config_path);
    let config_file = resolved_config_path.as_path();
    if config_file.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::metadata(config_file)?;
            let mode = meta.permissions().mode();
            if mode & 0o077 != 0 {
                println!(
                    "  {RED}{ERR} FAIL{RESET} Config file is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {}", config_file.display());
                fail_count += 1;
            } else {
                println!("  {OK} Config file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Config file permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  {WARN} Config file not found: {}", config_file.display());
        warn_count += 1;
    }

    // 2. Check for API keys in config
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        let has_plaintext_key =
            content.contains("api_key") && !content.contains("${") && !content.contains("env(");
        if has_plaintext_key {
            println!("  {WARN} Plaintext API keys found in config");
            println!("         Recommendation: Use environment variables instead");
            warn_count += 1;
        } else {
            println!("  {OK} No plaintext API keys in config");
            pass_count += 1;
        }
    }

    // 3. Check bind address
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("bind = \"0.0.0.0\"") {
            println!("  {WARN} Server bound to 0.0.0.0 (all interfaces)");
            println!("         Recommendation: Bind to 127.0.0.1 unless external access is needed");
            warn_count += 1;
        } else {
            println!("  {OK} Server not bound to all interfaces");
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
                println!(
                    "  {RED}{ERR} FAIL{RESET} Wallet file is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {}", wallet_path.display());
                fail_count += 1;
            } else {
                println!("  {OK} Wallet file permissions (mode {:o})", mode & 0o777);
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Wallet permission check (non-Unix)");
            warn_count += 1;
        }
    } else {
        println!("  {DIM}  \u{2500}{RESET} No wallet file found (OK if not using wallet features)");
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
                println!(
                    "  {WARN} Database is world/group-readable (mode {:o})",
                    mode & 0o777
                );
                println!("         Fix: chmod 600 {}", db_path.display());
                warn_count += 1;
            } else {
                println!("  {OK} Database file permissions");
                pass_count += 1;
            }
        }
        #[cfg(not(unix))]
        {
            println!("  {WARN} Database permission check (non-Unix)");
            warn_count += 1;
        }
    }

    // 6. Check CORS settings
    if config_file.exists() {
        let content = std::fs::read_to_string(config_file)?;
        if content.contains("cors") && content.contains("\"*\"") {
            println!("  {WARN} CORS allows all origins (\"*\")");
            println!("         Recommendation: Restrict CORS to specific origins in production");
            warn_count += 1;
        } else {
            println!("  {OK} CORS configuration");
            pass_count += 1;
        }
    }

    // 7. Check PID file
    let pid_path = ironclad_core::home_dir()
        .join(".ironclad")
        .join("ironclad.pid");
    if pid_path.exists() {
        println!("  {OK} PID file exists");
        pass_count += 1;
    }

    // Summary
    println!();
    let total = pass_count + warn_count + fail_count;
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

