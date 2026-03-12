fn run_mechanic_text_security_and_finalize(
    ironclad_dir: &Path,
    repair: bool,
    fixed: &mut u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    // ── Security configuration audit ─────────────────────────────
    println!("\n  {BOLD}Security Configuration{RESET}\n");
    {
        let cfg_path = if config_path.exists() {
            config_path.to_path_buf()
        } else if alt_config.exists() {
            alt_config.clone()
        } else {
            PathBuf::new()
        };

        if cfg_path.as_os_str().is_empty() {
            println!("  {WARN} No config file found — cannot audit security settings");
        } else if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
            if let Ok(cfg) = toml::from_str::<ironclad_core::IroncladConfig>(&raw) {
                let has_security_section = has_toml_section(&raw, "[security]");

                // Check 1: Missing [security] section
                if !has_security_section {
                    println!("  {WARN} No [security] section in config (running on defaults)");
                    println!(
                        "    {DETAIL} Run `ironclad mechanic --repair` for guided security setup."
                    );
                }

                // Check 2: No trusted senders + channels enabled
                let has_channels = cfg.channels.telegram.as_ref().is_some_and(|t| t.enabled)
                    || cfg.channels.whatsapp.as_ref().is_some_and(|w| w.enabled)
                    || cfg.channels.discord.as_ref().is_some_and(|d| d.enabled)
                    || cfg.channels.signal.as_ref().is_some_and(|s| s.enabled)
                    || cfg.channels.email.enabled;

                if cfg.channels.trusted_sender_ids.is_empty() && has_channels {
                    println!(
                        "  {RED}{ERR}{RESET} No trusted senders configured — no user can reach Creator authority."
                    );
                    println!(
                        "    {DETAIL} Caution+ tools (filesystem, scripts, delegation) are inaccessible."
                    );
                    println!("    {DETAIL} Add sender IDs to trusted_sender_ids in [channels].");
                }

                // Per-channel allow-list checks
                let channel_checks: Vec<(&str, bool, bool)> = vec![
                    (
                        "Telegram",
                        cfg.channels.telegram.as_ref().is_some_and(|t| t.enabled),
                        cfg.channels
                            .telegram
                            .as_ref()
                            .map(|t| t.allowed_chat_ids.is_empty())
                            .unwrap_or(true),
                    ),
                    (
                        "Discord",
                        cfg.channels.discord.as_ref().is_some_and(|d| d.enabled),
                        cfg.channels
                            .discord
                            .as_ref()
                            .map(|d| d.allowed_guild_ids.is_empty())
                            .unwrap_or(true),
                    ),
                    (
                        "WhatsApp",
                        cfg.channels.whatsapp.as_ref().is_some_and(|w| w.enabled),
                        cfg.channels
                            .whatsapp
                            .as_ref()
                            .map(|w| w.allowed_numbers.is_empty())
                            .unwrap_or(true),
                    ),
                    (
                        "Signal",
                        cfg.channels.signal.as_ref().is_some_and(|s| s.enabled),
                        cfg.channels
                            .signal
                            .as_ref()
                            .map(|s| s.allowed_numbers.is_empty())
                            .unwrap_or(true),
                    ),
                    (
                        "Email",
                        cfg.channels.email.enabled,
                        cfg.channels.email.allowed_senders.is_empty(),
                    ),
                ];

                let mut any_security_issue = false;
                for (name, enabled, empty_list) in &channel_checks {
                    if *enabled && *empty_list {
                        any_security_issue = true;
                        if cfg.security.deny_on_empty_allowlist {
                            println!(
                                "  {RED}{ERR}{RESET} {name}: no allow-list — all messages will be rejected"
                            );
                        } else {
                            println!(
                                "  {RED}{ERR}{RESET} {name}: open to the entire internet (empty allow-list + deny_on_empty = false)"
                            );
                        }
                    } else if *enabled {
                        println!("  {OK} {name}: allow-list configured");
                    }
                }

                // Summary line for clean configs
                if !any_security_issue
                    && !cfg.channels.trusted_sender_ids.is_empty()
                    && has_security_section
                {
                    println!("  {OK} Security configuration looks healthy");
                    println!(
                        "    {DETAIL} deny_on_empty_allowlist: {}",
                        cfg.security.deny_on_empty_allowlist
                    );
                    println!(
                        "    {DETAIL} allowlist_authority: {:?}",
                        cfg.security.allowlist_authority
                    );
                    println!(
                        "    {DETAIL} trusted senders: {} configured",
                        cfg.channels.trusted_sender_ids.len()
                    );
                }

                // ── Interactive security repair interview ──────────
                if repair
                    && (any_security_issue
                        || cfg.channels.trusted_sender_ids.is_empty()
                        || !has_security_section)
                {
                    println!();
                    println!("  {ACCENT}=== Security Configuration Interview ==={RESET}");
                    println!();

                    // Collect user answers
                    let mut new_deny_on_empty = cfg.security.deny_on_empty_allowlist;
                    let mut new_trusted: Vec<String> = cfg.channels.trusted_sender_ids.clone();
                    let mut channel_updates: Vec<(String, Vec<String>)> = Vec::new();

                    // Q1: Should empty allow-lists deny or allow?
                    if any_security_issue {
                        println!(
                            "  {CYAN}Q1:{RESET} Should channels with no allow-list deny all messages? (secure-by-default)"
                        );
                        println!("      Yes = reject messages from unknown senders (recommended)");
                        println!(
                            "      No  = allow all messages when allow-list is empty (legacy behavior)"
                        );
                        if prompt_yes_no("      Deny on empty allow-list?") {
                            new_deny_on_empty = true;
                        } else {
                            new_deny_on_empty = false;
                            println!(
                                "    {WARN} Legacy open mode — any user on the internet can message your agent."
                            );
                        }
                        println!();
                    }

                    // Q2: Per-channel allow-lists for channels with empty lists
                    for (name, enabled, empty_list) in &channel_checks {
                        if *enabled && *empty_list && new_deny_on_empty {
                            println!(
                                "  {CYAN}Q:{RESET} Enter allowed IDs for {name} (comma-separated, or 'skip'):"
                            );
                            match *name {
                                "Telegram" => println!(
                                    "      (Find your chat ID by messaging @userinfobot on Telegram)"
                                ),
                                "Discord" => {
                                    println!("      (Right-click your server → Copy Server ID)")
                                }
                                _ => {}
                            }
                            let answer = prompt_line("      > ");
                            if !answer.is_empty() && answer.to_lowercase() != "skip" {
                                let ids: Vec<String> = answer
                                    .split(',')
                                    .map(|s| s.trim().to_string())
                                    .filter(|s| !s.is_empty())
                                    .collect();
                                if !ids.is_empty() {
                                    channel_updates.push((name.to_lowercase(), ids));
                                }
                            }
                            println!();
                        }
                    }

                    // Q3: Trusted sender IDs
                    if cfg.channels.trusted_sender_ids.is_empty() {
                        println!(
                            "  {CYAN}Q:{RESET} Which sender IDs should have {BOLD}Creator{RESET} (full) authority?"
                        );
                        println!(
                            "      These users can run scripts, modify files, delegate to subagents."
                        );
                        println!(
                            "      (Telegram: message @userinfobot to find your numeric user ID)"
                        );
                        println!("      Enter IDs (comma-separated, or 'skip'):");
                        let answer = prompt_line("      > ");
                        if !answer.is_empty() && answer.to_lowercase() != "skip" {
                            new_trusted = answer
                                .split(',')
                                .map(|s| s.trim().to_string())
                                .filter(|s| !s.is_empty())
                                .collect();
                        }
                        println!();
                    }

                    // Q4: Allowlist authority level
                    let mut new_allowlist_auth = cfg.security.allowlist_authority;
                    if !channel_updates.is_empty() || any_security_issue {
                        println!(
                            "  {CYAN}Q:{RESET} Should allow-listed users (not in trusted list) be able to use filesystem tools?"
                        );
                        println!(
                            "      Yes = Peer authority (read/write files, but NOT run scripts)"
                        );
                        println!("      No  = External authority (safe tools only)");
                        if prompt_yes_no("      Grant Peer authority to allow-listed users?") {
                            new_allowlist_auth = ironclad_core::InputAuthority::Peer;
                        } else {
                            new_allowlist_auth = ironclad_core::InputAuthority::External;
                        }
                        println!();
                    }

                    // Write the config changes using TOML merge
                    // Build the security section
                    // Build the [security] TOML section.
                    // Use serde_json round-trip to get the exact variant
                    // names that toml deserialization expects, avoiding
                    // fragile Debug formatting.
                    let auth_str = |a: ironclad_core::InputAuthority| -> &'static str {
                        match a {
                            ironclad_core::InputAuthority::External => "External",
                            ironclad_core::InputAuthority::Peer => "Peer",
                            ironclad_core::InputAuthority::SelfGenerated => "SelfGenerated",
                            ironclad_core::InputAuthority::Creator => "Creator",
                        }
                    };
                    let security_toml = format!(
                        "\n[security]\ndeny_on_empty_allowlist = {}\nallowlist_authority = \"{}\"\ntrusted_authority = \"{}\"\napi_authority = \"{}\"\nthreat_caution_ceiling = \"{}\"\n",
                        new_deny_on_empty,
                        auth_str(new_allowlist_auth),
                        auth_str(cfg.security.trusted_authority),
                        auth_str(cfg.security.api_authority),
                        auth_str(cfg.security.threat_caution_ceiling),
                    );

                    // Write updates to config file
                    // Use backup + append/merge pattern
                    let backup_path = cfg_path.with_extension("toml.bak");
                    if let Err(e) = std::fs::copy(&cfg_path, &backup_path) {
                        println!("  {WARN} Could not create config backup: {e}");
                    } else {
                        println!("  {OK} Backed up config to {}", backup_path.display());
                    }

                    // Normalize to \n for safe manipulation. The final write
                    // uses \n consistently (POSIX standard for config files).
                    let mut content = raw.replace("\r\n", "\n").replace('\r', "\n");

                    // Replace or append [security] section.
                    // Use line-start anchoring to avoid matching "[security]"
                    // inside comments or string values.
                    if has_security_section {
                        // Find a line whose trimmed content is exactly "[security]"
                        let mut byte_offset = 0usize;
                        let mut section_start: Option<usize> = None;
                        for line in content.split('\n') {
                            if line.trim() == "[security]" {
                                section_start = Some(byte_offset);
                                break;
                            }
                            byte_offset += line.len() + 1; // +1 for the '\n'
                        }

                        if let Some(start) = section_start {
                            let after_header = start + "[security]".len();
                            let rest = &content[after_header..];
                            let end = rest
                                .find("\n[")
                                .map(|i| after_header + i)
                                .unwrap_or(content.len());
                            content = format!("{}{}", &content[..start], &content[end..]);
                        }
                    }
                    content.push_str(&security_toml);

                    // Update trusted_sender_ids in [channels]
                    if new_trusted != cfg.channels.trusted_sender_ids && !new_trusted.is_empty() {
                        let formatted: Vec<String> =
                            new_trusted.iter().map(|s| format!("\"{}\"", s)).collect();
                        let new_line = format!("trusted_sender_ids = [{}]", formatted.join(", "));
                        if content.contains("trusted_sender_ids") {
                            // Replace existing line
                            let lines: Vec<&str> = content.lines().collect();
                            let new_lines: Vec<String> = lines
                                .iter()
                                .map(|line| {
                                    if line.trim().starts_with("trusted_sender_ids") {
                                        new_line.clone()
                                    } else {
                                        line.to_string()
                                    }
                                })
                                .collect();
                            content = new_lines.join("\n");
                            if !content.ends_with('\n') {
                                content.push('\n');
                            }
                        } else if let Some(pos) = content.find("[channels]") {
                            // Insert after [channels] line
                            let insert_pos = content[pos..]
                                .find('\n')
                                .map(|i| pos + i + 1)
                                .unwrap_or(content.len());
                            content.insert_str(insert_pos, &format!("{}\n", new_line));
                        }
                    }

                    // Update per-channel allow-lists
                    for (channel, ids) in &channel_updates {
                        let field_name = match channel.as_str() {
                            "telegram" => "allowed_chat_ids",
                            "discord" => "allowed_guild_ids",
                            "whatsapp" | "signal" => "allowed_numbers",
                            "email" => "allowed_senders",
                            _ => continue,
                        };
                        let formatted: Vec<String> =
                            ids.iter().map(|s| format!("\"{}\"", s)).collect();
                        let new_line = format!("{} = [{}]", field_name, formatted.join(", "));
                        if content.contains(field_name) {
                            let lines: Vec<&str> = content.lines().collect();
                            let new_lines: Vec<String> = lines
                                .iter()
                                .map(|line| {
                                    if line.trim().starts_with(field_name) {
                                        new_line.clone()
                                    } else {
                                        line.to_string()
                                    }
                                })
                                .collect();
                            content = new_lines.join("\n");
                            if !content.ends_with('\n') {
                                content.push('\n');
                            }
                        }
                    }

                    // Write the updated config
                    match std::fs::write(&cfg_path, &content) {
                        Ok(()) => {
                            println!(
                                "  {ACTION} Updated security configuration in {}",
                                cfg_path.display()
                            );
                            *fixed += 1;
                        }
                        Err(e) => {
                            println!("  {RED}{ERR}{RESET} Failed to write config: {e}");
                        }
                    }
                }
            } else {
                println!("  {WARN} Could not parse config file for security audit");
            }
        } else {
            println!("  {WARN} Could not read config file for security audit");
        }
    }

    // ── Sandbox configuration health check ─────────────────────────
    println!("\n  {BOLD}Sandbox Configuration{RESET}\n");
    {
        let cfg_path = if config_path.exists() {
            config_path.to_path_buf()
        } else if alt_config.exists() {
            alt_config.clone()
        } else {
            PathBuf::new()
        };

        if cfg_path.as_os_str().is_empty() {
            println!("  {WARN} No config file found — cannot audit sandbox settings");
        } else if let Ok(raw) = std::fs::read_to_string(&cfg_path) {
            if let Ok(cfg) = toml::from_str::<ironclad_core::IroncladConfig>(&raw) {
                let sk = &cfg.skills;
                let mut any_issue = false;

                // Check 1: Sandbox disabled entirely
                if !sk.sandbox_env {
                    any_issue = true;
                    println!(
                        "  {RED}{ERR}{RESET} Sandbox disabled — skill scripts run with full env access"
                    );
                    println!(
                        "    {DETAIL} Set sandbox_env = true in [skills] to isolate script execution."
                    );
                }

                // Check 2: Bare interpreter names (PATH hijacking risk)
                let bare_interpreters: Vec<&str> = sk
                    .allowed_interpreters
                    .iter()
                    .filter(|i| !std::path::Path::new(i.as_str()).is_absolute())
                    .map(|s| s.as_str())
                    .collect();
                if !bare_interpreters.is_empty() {
                    any_issue = true;
                    println!(
                        "  {WARN} {} interpreter{} use bare names (PATH hijacking risk)",
                        bare_interpreters.len(),
                        if bare_interpreters.len() == 1 { "" } else { "s" }
                    );
                    for name in &bare_interpreters {
                        println!("    {DETAIL} {name} — resolve to absolute path for safety");
                    }
                    println!(
                        "    {DETAIL} At runtime, the script runner resolves to absolute paths automatically."
                    );
                    println!(
                        "    {DETAIL} For defense-in-depth, set absolute paths in allowed_interpreters."
                    );
                }

                // Check 3: No memory limit configured
                if sk.script_max_memory_bytes.is_none() {
                    any_issue = true;
                    println!("  {WARN} No memory ceiling for skill scripts (script_max_memory_bytes = none)");
                    println!("    {DETAIL} A runaway script could exhaust system memory.");
                }

                // Check 4: Network access allowed
                if sk.network_allowed {
                    println!(
                        "  {YELLOW}ℹ{RESET}  Network access enabled for sandboxed scripts (network_allowed = true)"
                    );
                }

                // Check 5: Platform limitations
                #[cfg(target_os = "macos")]
                {
                    println!(
                        "  {YELLOW}ℹ{RESET}  macOS: RLIMIT_AS memory enforcement unavailable (virtual memory model)"
                    );
                    println!(
                        "  {YELLOW}ℹ{RESET}  macOS: network namespace isolation unavailable (no unshare)"
                    );
                }

                // Summary
                if !any_issue {
                    println!("  {OK} Sandbox configuration looks healthy");
                    println!(
                        "    {DETAIL} sandbox_env: {}",
                        if sk.sandbox_env { "enabled" } else { "disabled" }
                    );
                    println!(
                        "    {DETAIL} interpreters: {} configured",
                        sk.allowed_interpreters.len()
                    );
                    if let Some(mem) = sk.script_max_memory_bytes {
                        println!(
                            "    {DETAIL} memory ceiling: {} MiB",
                            mem / (1024 * 1024)
                        );
                    }
                    println!(
                        "    {DETAIL} network: {}",
                        if sk.network_allowed { "allowed" } else { "denied" }
                    );
                    println!(
                        "    {DETAIL} timeout: {}s",
                        sk.script_timeout_seconds
                    );
                }
            } else {
                println!("  {WARN} Could not parse config file for sandbox audit");
            }
        } else {
            println!("  {WARN} Could not read config file for sandbox audit");
        }
    }

    if repair {
        let state_db = ironclad_dir.join("state.db");
        match normalize_schema_safe(&state_db) {
            Ok(true) => {
                println!("  {ACTION} Applied safe schema normalization in state.db");
                *fixed += 1;
            }
            Ok(false) => {}
            Err(e) => {
                println!("  {WARN} Schema normalization skipped: {e}");
            }
        }
    }

    println!();
    if repair && *fixed > 0 {
        println!("  {ACTION} Auto-fixed {fixed} issue(s)");
    }
    println!();
    Ok(())
}
