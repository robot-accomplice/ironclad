fn run_mechanic_text_local_preflight(
    ironclad_dir: &Path,
    repair: bool,
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let ironclad_dir = ironclad_dir.to_path_buf();

    // Check directories
    let dirs = [
        ironclad_dir.clone(),
        ironclad_dir.join("workspace"),
        ironclad_dir.join("skills"),
        ironclad_dir.join("plugins"),
        ironclad_dir.join("logs"),
    ];

    for dir in &dirs {
        if dir.exists() {
            println!("  {OK} Directory exists: {}", dir.display());
        } else if repair {
            std::fs::create_dir_all(dir)?;
            println!("  {ACTION} Created directory: {}", dir.display());
            *fixed += 1;
        } else {
            println!(
                "  {WARN} Missing directory: {} (use --repair to create)",
                dir.display()
            );
        }
    }

    // Check config file
    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    if config_path.exists() || alt_config.exists() {
        println!("  {OK} Configuration file found");
    } else if repair {
        let default_config = format!(
            concat!(
                "[agent]\n",
                "name = \"Ironclad\"\n",
                "id = \"ironclad-dev\"\n\n",
                "[server]\n",
                "port = 18789\n",
                "bind = \"127.0.0.1\"\n\n",
                "[database]\n",
                "path = \"{}/state.db\"\n\n",
                "[models]\n",
                "primary = \"ollama/qwen3:8b\"\n",
                "fallbacks = [\"openai/gpt-4o\"]\n\n",
                "# Provider-specific settings are auto-merged from bundled defaults.\n",
                "# Override any provider below; new providers work the same way.\n",
                "# [providers.ollama]\n",
                "# url = \"http://localhost:11434\"\n",
                "# tier = \"T1\"\n",
                "# format = \"openai\"\n",
                "# is_local = true\n",
            ),
            ironclad_dir.display()
        );
        std::fs::write(&alt_config, default_config)?;
        println!(
            "  {ACTION} Created default config: {}",
            alt_config.display()
        );
        *fixed += 1;
    } else {
        println!("  {WARN} No config file found (use --repair or `ironclad init`)");
    }

    let effective_config_path = if config_path.exists() {
        Some(Path::new(config_path))
    } else if alt_config.exists() {
        Some(alt_config.as_path())
    } else {
        None
    };
    if let Some(cfg_path) = effective_config_path {
        match migrate_removed_legacy_config_if_needed(cfg_path, repair)? {
            Some(report) => {
                println!("  {ACTION} Migrated removed legacy config settings");
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
                    println!(
                        "    {DETAIL} Removed deprecated circuit_breaker.credit_cooldown_seconds"
                    );
                }
                *fixed += 1;
            }
            None if repair => {
                println!("  {OK} Config compatibility migration not needed");
            }
            None => {}
        }
    }

    // Check file permissions (Unix)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let sensitive_files = [
            ironclad_dir.join("wallet.json"),
            ironclad_dir.join("state.db"),
        ];

        for file in &sensitive_files {
            if file.exists() {
                let meta = std::fs::metadata(file)?;
                let mode = meta.permissions().mode() & 0o777;
                if mode & 0o077 != 0 {
                    if repair {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o600);
                        std::fs::set_permissions(file, perms)?;
                        println!("  {ACTION} Set permissions 600 on {}", file.display());
                        *fixed += 1;
                    } else {
                        println!(
                            "  {WARN} {} has loose permissions ({:o}) - use --repair",
                            file.display(),
                            mode
                        );
                    }
                } else {
                    println!("  {OK} {} permissions OK ({:o})", file.display(), mode);
                }
            }
        }
    }

    let oauth_health = check_and_repair_oauth_storage(repair);
    if oauth_health.needs_attention() {
        if repair && oauth_health.repaired {
            println!("  {ACTION} Repaired OAuth token storage migration drift");
            if oauth_health.migrated_entries > 0 {
                println!(
                    "    {DETAIL} Migrated {} OAuth token entr{} to keystore.",
                    oauth_health.migrated_entries,
                    if oauth_health.migrated_entries == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                );
            }
            *fixed += 1;
        } else {
            println!("  {WARN} OAuth token storage needs migration/repair");
            if oauth_health.legacy_plaintext_exists {
                println!("    {DETAIL} Legacy plaintext token file is still present.");
            }
            if !oauth_health.keystore_available {
                println!("    {DETAIL} Keystore is unavailable; migration cannot proceed.");
            }
            if oauth_health.malformed_keystore_entries > 0 {
                println!(
                    "    {DETAIL} Found {} malformed OAuth keystore entr{}.",
                    oauth_health.malformed_keystore_entries,
                    if oauth_health.malformed_keystore_entries == 1 {
                        "y"
                    } else {
                        "ies"
                    }
                );
            }
            if oauth_health.legacy_parse_failed {
                println!(
                    "    {DETAIL} Legacy OAuth file is unreadable; manual cleanup may be required."
                );
            }
            println!("    {DETAIL} Run `ironclad mechanic --repair` to attempt automatic repair.");
        }
    } else {
        println!("  {OK} OAuth token storage healthy");
    }

    // Check Go toolchain
    let mut go_bin = which_binary("go");
    match go_bin.as_ref() {
        Some(path) => {
            let ver = std::process::Command::new(path)
                .arg("version")
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let ver = ver.trim().strip_prefix("go version ").unwrap_or(ver.trim());
            println!("  {OK} Go toolchain: {ver} ({path})");
        }
        None => {
            println!("  {RED}{ERR}{RESET} Go not found (required for gosh plugin engine)");
            #[cfg(target_os = "windows")]
            println!(
                "         Install from https://go.dev/dl/ or: winget install -e --id GoLang.Go"
            );
            #[cfg(target_os = "macos")]
            println!("         Install from https://go.dev/dl/ or: brew install go");
            #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
            println!("         Install from https://go.dev/dl/ (or your distro package manager)");

            if repair && prompt_yes_no("  Attempt automatic Go installation now?") {
                #[cfg(target_os = "windows")]
                let install_result = std::process::Command::new("winget")
                    .args([
                        "install",
                        "-e",
                        "--id",
                        "GoLang.Go",
                        "--accept-package-agreements",
                        "--accept-source-agreements",
                    ])
                    .status();

                #[cfg(target_os = "macos")]
                let install_result = std::process::Command::new("brew")
                    .args(["install", "go"])
                    .status();

                #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
                let install_result = {
                    if which_binary("apt-get").is_some() {
                        std::process::Command::new("sudo")
                            .args(["apt-get", "install", "-y", "golang-go"])
                            .status()
                    } else if which_binary("dnf").is_some() {
                        std::process::Command::new("sudo")
                            .args(["dnf", "install", "-y", "golang"])
                            .status()
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "no supported package manager found",
                        ))
                    }
                };

                match install_result {
                    Ok(status) if status.success() => {
                        go_bin = which_binary("go");
                        if let Some(path) = go_bin.as_ref() {
                            println!("  {ACTION} Go installed: {path}");
                            *fixed += 1;
                        } else {
                            println!(
                                "  {WARN} Go install may have succeeded, but `go` is not on PATH yet. Open a new shell and re-run `ironclad mechanic --repair`."
                            );
                        }
                    }
                    _ => {
                        println!("  {RED}{ERR}{RESET} Automatic Go install failed.");
                    }
                }
            }
        }
    }

    // Report Go bin PATH status explicitly so users can see why gosh may not resolve.
    let go_bin_dirs = go_bin_candidates();
    if go_bin_dirs.is_empty() {
        println!("  {WARN} Go bin path status: no candidate bin directory found");
    } else {
        for dir in &go_bin_dirs {
            if dir.exists() {
                if path_contains_dir(dir) {
                    println!("  {OK} Go bin path status: on PATH ({})", dir.display());
                } else {
                    println!(
                        "  {WARN} Go bin path status: missing from PATH ({})",
                        dir.display()
                    );
                }
            } else {
                println!(
                    "  {WARN} Go bin path status: candidate directory not found ({})",
                    dir.display()
                );
            }
        }
    }

    // Check gosh scripting engine
    match which_binary("gosh") {
        Some(path) => {
            println!("  {OK} gosh scripting engine: {path}");
        }
        None if repair => {
            if go_bin.is_some() {
                if prompt_yes_no("  Install gosh now via `go install`?") {
                    println!("  {ACTION} Installing gosh...");
                    let result = if let Some(go_path) = go_bin.as_deref() {
                        std::process::Command::new(go_path)
                            .args(["install", "github.com/drewwalton19216801/gosh@latest"])
                            .status()
                    } else {
                        Err(std::io::Error::new(
                            std::io::ErrorKind::NotFound,
                            "go binary not found",
                        ))
                    };
                    match result {
                        Ok(s) if s.success() => {
                            if let Some(path) = which_binary("gosh") {
                                println!("  {ACTION} gosh installed: {path}");
                                *fixed += 1;
                            } else if let Some(gosh_path) = find_gosh_in_go_bins() {
                                println!(
                                    "  {WARN} gosh installed at {} but not on PATH.",
                                    gosh_path.display()
                                );
                                if let Some(go_bin_dir) = gosh_path.parent() {
                                    if prompt_yes_no(&format!(
                                        "  Add {} to your PATH now?",
                                        go_bin_dir.display()
                                    )) {
                                        #[cfg(windows)]
                                        {
                                            match add_dir_to_user_path_windows(go_bin_dir) {
                                                Ok(()) => {
                                                    println!(
                                                        "  {ACTION} Added {} to user PATH",
                                                        go_bin_dir.display()
                                                    );
                                                    println!(
                                                        "         Open a new shell and re-run `ironclad mechanic --repair` to verify."
                                                    );
                                                    *fixed += 1;
                                                }
                                                Err(e) => {
                                                    println!(
                                                        "  {RED}{ERR}{RESET} Failed to update PATH automatically: {e}"
                                                    );
                                                    println!(
                                                        "         Add this directory manually: {}",
                                                        go_bin_dir.display()
                                                    );
                                                }
                                            }
                                        }
                                        #[cfg(not(windows))]
                                        {
                                            println!(
                                                "  {WARN} Automatic PATH updates are only implemented on Windows."
                                            );
                                            println!(
                                                "         Add this directory manually: {}",
                                                go_bin_dir.display()
                                            );
                                        }
                                    } else {
                                        println!("  {WARN} PATH update skipped by user.");
                                    }
                                }
                            } else {
                                println!(
                                    "  {WARN} go install succeeded but `gosh` is not on PATH."
                                );
                                println!(
                                    "         Add your Go bin directory to PATH, then re-run mechanic."
                                );
                                #[cfg(target_os = "windows")]
                                println!("         Typical path: %USERPROFILE%\\go\\bin");
                                #[cfg(not(target_os = "windows"))]
                                println!("         Typical path: $HOME/go/bin");
                            }
                        }
                        _ => {
                            println!("  {RED}{ERR}{RESET} Failed to install gosh. Try manually:");
                            println!(
                                "         go install github.com/drewwalton19216801/gosh@latest"
                            );
                        }
                    }
                } else {
                    println!("  {WARN} gosh not installed (skipped by user)");
                }
            } else {
                println!("  {WARN} gosh not found (Go is required first)");
            }
        }
        None => {
            println!(
                "  {WARN} gosh not found (use --repair to install, or: go install github.com/drewwalton19216801/gosh@latest)"
            );
            if let Some(gosh_path) = find_gosh_in_go_bins() {
                println!(
                    "         Found gosh at {} but that directory is not on PATH.",
                    gosh_path.display()
                );
                if let Some(dir) = gosh_path.parent()
                    && !path_contains_dir(dir)
                {
                    #[cfg(target_os = "windows")]
                    println!("         Run `ironclad mechanic --repair` to add it with approval.");
                }
            }
        }
    }

    let skills_cleanup = cleanup_internalized_skill_artifacts(
        &ironclad_dir.join("state.db"),
        &ironclad_dir.join("skills"),
        repair,
    );
    if !skills_cleanup.stale_db_skills.is_empty()
        || !skills_cleanup.stale_files.is_empty()
        || !skills_cleanup.stale_dirs.is_empty()
    {
        println!(
            "  {WARN} Internalized skills still present as external artifacts (DB/filesystem drift)"
        );
        if !skills_cleanup.stale_db_skills.is_empty() {
            println!(
                "    {DETAIL} Stale DB rows: {}",
                skills_cleanup.stale_db_skills.join(", ")
            );
        }
        let stale_paths: Vec<String> = skills_cleanup
            .stale_files
            .iter()
            .chain(skills_cleanup.stale_dirs.iter())
            .map(|p| p.display().to_string())
            .collect();
        if !stale_paths.is_empty() {
            println!(
                "    {DETAIL} Stale skill artifacts: {}",
                stale_paths.join(", ")
            );
        }
        if repair {
            let removed_count =
                skills_cleanup.removed_db_skills.len() + skills_cleanup.removed_paths.len();
            if removed_count > 0 {
                println!(
                    "  {ACTION} Cleaned {removed_count} internalized-skill artifact{}",
                    if removed_count == 1 { "" } else { "s" }
                );
                *fixed += removed_count as u32;
            }
        } else {
            println!("    {DETAIL} Run `ironclad mechanic --repair` to remove stale artifacts.");
        }
    }

    let capability_skill_parity = evaluate_capability_skill_parity(&ironclad_dir.join("state.db"));
    if !capability_skill_parity.missing_in_registry.is_empty() {
        println!("  {ERR} Capability-to-skill parity gap in builtin registry");
        println!(
            "    {DETAIL} Missing mappings: {}",
            capability_skill_parity.missing_in_registry.join("; ")
        );
        println!(
            "    {DETAIL} Add missing skills to registry/builtin-skills.json before shipping."
        );
    }
    if capability_skill_parity.missing_in_registry.is_empty() {
        println!("  {OK} Capability-to-skill parity checks passed");
    }

    Ok(())
}
