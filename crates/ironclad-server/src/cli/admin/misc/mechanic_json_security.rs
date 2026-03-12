fn collect_mechanic_json_security_and_plugin_findings(
    ironclad_dir: &Path,
    repair: bool,
    findings: &mut Vec<MechanicFinding>,
    actions: &mut RepairActionSummary,
) -> Result<(), Box<dyn std::error::Error>> {
    let config_path = std::path::Path::new("ironclad.toml");
    let alt_config = ironclad_dir.join("ironclad.toml");
    let log_snapshot = recent_log_snapshot(&ironclad_dir.join("logs"), 350_000);
    if let Some(snapshot) = log_snapshot.as_deref() {
        let tg_404_count =
            count_occurrences(snapshot, "Telegram API error\",\"status\":\"404 Not Found");
        let tg_poll_err_count = count_occurrences(snapshot, "Telegram poll error, backing off 5s");
        if tg_404_count >= 3 || tg_poll_err_count >= 3 {
            findings.push(finding(
                "telegram-invalid-token-likely",
                "high",
                0.96,
                "Repeated Telegram 404/poll-backoff failures",
                "Log signatures strongly suggest an invalid or revoked Telegram bot token.",
                "Set a valid token and restart daemon.",
                vec![
                    "ironclad keystore set telegram_bot_token \"<TOKEN>\"".to_string(),
                    "ironclad daemon restart".to_string(),
                ],
                false,
                true,
            ));
        }
        let unknown_action_count = count_occurrences(snapshot, "unknown action: unknown");
        if unknown_action_count >= 3 {
            findings.push(finding(
                "cron-unknown-action-storm",
                "high",
                0.92,
                "Recurring cron unknown-action failures",
                "Scheduler repeatedly hit legacy/invalid cron action payloads.",
                "Recover paused jobs selectively after validation.",
                vec!["ironclad schedule recover --all --dry-run".to_string()],
                true,
                false,
            ));
        }
    }

    // ── Security configuration findings ─────────────────────────────
    // Try to load the config to analyze security posture.
    let security_config_path = if config_path.exists() {
        Some(config_path.to_path_buf())
    } else if alt_config.exists() {
        Some(alt_config.clone())
    } else {
        None
    };
    if let Some(ref cfg_path) = security_config_path
        && let Ok(raw) = std::fs::read_to_string(cfg_path)
        && let Ok(cfg) = toml::from_str::<ironclad_core::IroncladConfig>(&raw)
    {
        // Check 1: Missing [security] section (running on defaults)
        if !has_toml_section(&raw, "[security]") {
            findings.push(finding(
                        "security-missing-section",
                        "medium",
                        0.95,
                        "No [security] section in config",
                        "Running on default security settings. Run `ironclad mechanic --repair` for guided setup.",
                        "Add explicit [security] section with deny_on_empty_allowlist, allowlist_authority, etc.",
                        vec!["ironclad mechanic --repair".to_string()],
                        false,
                        true,
                    ));
        }

        // Check 2: No trusted senders + channels enabled
        let has_channels = cfg.channels.telegram.as_ref().is_some_and(|t| t.enabled)
            || cfg.channels.whatsapp.as_ref().is_some_and(|w| w.enabled)
            || cfg.channels.discord.as_ref().is_some_and(|d| d.enabled)
            || cfg.channels.signal.as_ref().is_some_and(|s| s.enabled)
            || cfg.channels.email.enabled;
        if cfg.channels.trusted_sender_ids.is_empty() && has_channels {
            findings.push(finding(
                "security-no-trusted-senders",
                "high",
                0.97,
                "No trusted senders configured",
                "trusted_sender_ids is empty — no user can reach Creator authority. \
                         Caution+ tools (filesystem, scripts, delegation) are inaccessible.",
                "Add sender IDs to trusted_sender_ids in [channels].",
                vec!["ironclad mechanic --repair".to_string()],
                false,
                true,
            ));
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

        for (name, enabled, empty_list) in &channel_checks {
            if *enabled && *empty_list {
                if cfg.security.deny_on_empty_allowlist {
                    // Check 3: deny_on_empty=true + empty list → all messages rejected
                    findings.push(finding(
                                "security-no-allowlist",
                                "high",
                                0.98,
                                format!("{name} has no allow-list — all messages will be rejected"),
                                format!(
                                    "{name} is enabled but has no allowed IDs and deny_on_empty_allowlist = true. \
                                     No one can send messages via this channel."
                                ),
                                format!("Add allowed IDs for {name} or disable the channel until an allow-list is configured."),
                                vec!["ironclad mechanic --repair".to_string()],
                                false,
                                true,
                            ));
                } else {
                    // Check 4: deny_on_empty=false + empty list → open to the world
                    findings.push(finding(
                                "security-open-to-world",
                                "critical",
                                0.99,
                                format!("{name} is open to the entire internet"),
                                format!(
                                    "{name} is enabled with an empty allow-list under a deprecated insecure configuration. \
                                     Runtime startup now rejects this state; migrate to an explicit allow-list or disable the channel."
                                ),
                                format!("Add allowed IDs for {name}, then rerun mechanic repair, or disable the channel."),
                                vec!["ironclad mechanic --repair".to_string()],
                                false,
                                true,
                            ));
                }
            }
        }
    }

    // ── Sandbox configuration findings ───────────────────────────────
    if let Some(ref cfg_path) = security_config_path
        && let Ok(raw) = std::fs::read_to_string(cfg_path)
        && let Ok(cfg) = toml::from_str::<ironclad_core::IroncladConfig>(&raw)
    {
        let sk = &cfg.skills;

        // Sandbox disabled entirely
        if !sk.sandbox_env {
            findings.push(finding(
                "sandbox-disabled",
                "high",
                0.99,
                "Sandbox disabled — skill scripts run with full environment access",
                "sandbox_env = false in [skills]. Scripts inherit the agent's full \
                 environment and filesystem access. This negates all sandbox protections.",
                "Set sandbox_env = true in [skills].",
                vec!["ironclad mechanic --repair".to_string()],
                false,
                true,
            ));
        }

        // Bare interpreter names (PATH hijacking risk)
        let bare_interpreters: Vec<&str> = sk
            .allowed_interpreters
            .iter()
            .filter(|i| !std::path::Path::new(i.as_str()).is_absolute())
            .map(|s| s.as_str())
            .collect();
        if !bare_interpreters.is_empty() {
            findings.push(finding(
                "sandbox-bare-interpreters",
                "medium",
                0.90,
                format!(
                    "{} interpreter(s) use bare names (PATH hijacking risk)",
                    bare_interpreters.len()
                ),
                format!(
                    "allowed_interpreters contains bare names: [{}]. A malicious PATH entry \
                     could shadow a legitimate interpreter. The script runner resolves to \
                     absolute paths at runtime, but config-level absolute paths provide \
                     defense-in-depth.",
                    bare_interpreters.join(", ")
                ),
                "Set absolute paths for allowed_interpreters in [skills] config.",
                vec![],
                false,
                false,
            ));
        }

        // No memory limit
        if sk.script_max_memory_bytes.is_none() {
            findings.push(finding(
                "sandbox-no-memory-limit",
                "medium",
                0.85,
                "No memory ceiling for skill scripts",
                "script_max_memory_bytes is not set — a runaway script could exhaust \
                 system memory. Default is 256 MiB on Linux (RLIMIT_AS).",
                "Set script_max_memory_bytes in [skills] config.",
                vec![],
                false,
                false,
            ));
        }
    }

    // Plugin health checks
    {
        use ironclad_plugin_sdk::manifest::PluginManifest;

        let plugins_dir = ironclad_dir.join("plugins");
        if plugins_dir.exists()
            && let Ok(entries) = std::fs::read_dir(&plugins_dir)
        {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let dir_name = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();
                let manifest_path = path.join("plugin.toml");

                if !manifest_path.exists() {
                    let mut f = finding(
                        "plugin-orphan-directory",
                        "medium",
                        0.95,
                        format!("Orphan plugin directory: {dir_name}"),
                        "Plugin directory exists but contains no valid plugin.toml. Likely an aborted install.",
                        "Remove orphan plugin directory.",
                        vec![format!("rm -rf \"{}\"", path.display())],
                        true,
                        false,
                    );
                    if repair && let Ok(()) = std::fs::remove_dir_all(&path) {
                        f.auto_repaired = true;
                    }
                    findings.push(f);
                    continue;
                }

                match PluginManifest::from_file(&manifest_path) {
                    Ok(manifest) => {
                        let report = manifest.vet(&path);
                        for e in &report.errors {
                            findings.push(finding(
                                    "plugin-vet-error",
                                    "high",
                                    0.95,
                                    format!("Plugin '{}': {e}", manifest.name),
                                    format!(
                                        "Plugin '{}' v{} has a blocking integrity error.",
                                        manifest.name, manifest.version
                                    ),
                                    "Reinstall the plugin or resolve the missing dependency.",
                                    vec![format!(
                                        "ironclad plugins uninstall {} && ironclad plugins install <source>",
                                        manifest.name
                                    )],
                                    false,
                                    true,
                                ));
                        }
                        for w in &report.warnings {
                            findings.push(finding(
                                "plugin-vet-warning",
                                "low",
                                0.90,
                                format!("Plugin '{}': {w}", manifest.name),
                                format!(
                                    "Plugin '{}' v{} has a non-blocking issue.",
                                    manifest.name, manifest.version
                                ),
                                "Review plugin configuration.",
                                vec![format!("ironclad plugins info {}", manifest.name)],
                                false,
                                false,
                            ));
                        }

                        // Repair: re-deploy missing companion skills
                        if repair {
                            let skills_dir = ironclad_dir.join("skills");
                            for skill_rel in &manifest.companion_skills {
                                let src = path.join(skill_rel);
                                let installed_name = super::plugins::companion_skill_install_name(
                                    &manifest.name,
                                    skill_rel,
                                );
                                let dest = skills_dir.join(&installed_name);
                                if src.exists() && !dest.exists() {
                                    std::fs::create_dir_all(&skills_dir).ok();
                                    if std::fs::copy(&src, &dest).is_ok() {
                                        findings.push(finding(
                                                "plugin-companion-skill-redeployed",
                                                "info",
                                                1.0,
                                                format!(
                                                    "Re-deployed companion skill: {installed_name}",
                                                ),
                                                format!(
                                                    "Plugin '{}' companion skill was missing from skills directory.",
                                                    manifest.name
                                                ),
                                                "Companion skill re-deployed from plugin bundle.",
                                                vec![],
                                                true,
                                                false,
                                            ));
                                    }
                                }
                            }
                        }
                    }
                    Err(_) => {
                        let mut f = finding(
                            "plugin-corrupt-manifest",
                            "medium",
                            0.95,
                            format!("Corrupt plugin manifest: {dir_name}"),
                            "Plugin directory has a plugin.toml that cannot be parsed.",
                            "Remove corrupt plugin directory.",
                            vec![format!("rm -rf \"{}\"", path.display())],
                            true,
                            false,
                        );
                        if repair && let Ok(()) = std::fs::remove_dir_all(&path) {
                            f.auto_repaired = true;
                        }
                        findings.push(f);
                    }
                }
            }
        }
    }

    // Mark security_configured if no security findings were emitted
    let has_security_findings = findings.iter().any(|f| f.id.starts_with("security-"));
    if !has_security_findings {
        actions.security_configured = true;
    }

    if repair {
        let state_db = ironclad_dir.join("state.db");
        if normalize_schema_safe(&state_db)? {
            actions.schema_normalized = true;
        }
    }
    Ok(())
}
