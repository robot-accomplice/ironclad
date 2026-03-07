pub fn cmd_completion(shell: &str) -> Result<(), Box<dyn std::error::Error>> {
    match shell {
        "bash" => {
            println!("# Ironclad bash completion");
            println!("# Add to ~/.bashrc: eval \"$(ironclad completion bash)\"");
            println!(
                "complete -W \"agents auth channels check circuit completion config daemon defrag ingest init keystore logs mechanic memory metrics migrate models plugins reset schedule security serve sessions setup skills status uninstall update version wallet web\" ironclad"
            );
        }
        "zsh" => {
            println!("# Ironclad zsh completion");
            println!("# Add to ~/.zshrc: eval \"$(ironclad completion zsh)\"");
            println!(
                "compctl -k \"(agents auth channels check circuit completion config daemon defrag ingest init keystore logs mechanic memory metrics migrate models plugins reset schedule security serve sessions setup skills status uninstall update version wallet web)\" ironclad"
            );
        }
        "fish" => {
            println!("# Ironclad fish completion");
            println!("# Run: ironclad completion fish | source");
            for cmd in [
                "agents",
                "auth",
                "channels",
                "check",
                "circuit",
                "completion",
                "config",
                "daemon",
                "defrag",
                "ingest",
                "init",
                "keystore",
                "logs",
                "mechanic",
                "memory",
                "metrics",
                "migrate",
                "models",
                "plugins",
                "reset",
                "schedule",
                "security",
                "serve",
                "sessions",
                "setup",
                "skills",
                "status",
                "uninstall",
                "update",
                "version",
                "wallet",
                "web",
            ] {
                println!("complete -c ironclad -a {cmd}");
            }
        }
        _ => {
            eprintln!("Unsupported shell: {shell}. Use bash, zsh, or fish.");
        }
    }
    Ok(())
}

pub fn cmd_uninstall(purge: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Uninstall{RESET}\n");

    match crate::daemon::uninstall_daemon() {
        Ok(()) => println!("  {OK} Daemon service removed"),
        Err(e) => println!("  {WARN} Daemon removal: {e}"),
    }

    if purge {
        let data_dir = ironclad_core::home_dir().join(".ironclad");
        if data_dir.exists() {
            std::fs::remove_dir_all(&data_dir)?;
            println!("  {OK} Removed {}", data_dir.display());
        } else {
            println!("  {WARN} Data directory not found: {}", data_dir.display());
        }
    } else {
        println!("  {DIM}Data preserved at ~/.ironclad/ (use --purge to remove){RESET}");
    }

    println!("\n  {GREEN}Uninstall complete.{RESET} CLI binary remains at current location.\n");
    Ok(())
}

pub fn cmd_reset(yes: bool) -> Result<(), Box<dyn std::error::Error>> {
    let (DIM, BOLD, ACCENT, GREEN, YELLOW, RED, CYAN, RESET, MONO) = colors();
    let (OK, ACTION, WARN, DETAIL, ERR) = icons();
    println!("\n  {BOLD}Ironclad Reset{RESET}\n");

    if !yes {
        println!("  This will reset configuration and clear the database.");
        println!("  Wallet files will be preserved.");
        println!("  Run with --yes to skip this prompt.\n");
        print!("  Continue? [y/N] ");
        use std::io::Write;
        std::io::stdout().flush()?;
        let mut input = String::new();
        std::io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            println!("  Aborted.");
            return Ok(());
        }
    }

    let ironclad_dir = ironclad_core::home_dir().join(".ironclad");

    let db_path = ironclad_dir.join("state.db");
    if db_path.exists() {
        std::fs::remove_file(&db_path)?;
        println!("  {OK} Database cleared");
    }

    let db_wal = ironclad_dir.join("state.db-wal");
    if db_wal.exists() {
        let _ = std::fs::remove_file(&db_wal);
    }
    let db_shm = ironclad_dir.join("state.db-shm");
    if db_shm.exists() {
        let _ = std::fs::remove_file(&db_shm);
    }

    let config_path = ironclad_dir.join("ironclad.toml");
    if config_path.exists() {
        std::fs::remove_file(&config_path)?;
        println!("  {OK} Configuration removed (re-run `ironclad init` to recreate)");
    }

    let logs_dir = ironclad_dir.join("logs");
    if logs_dir.exists() {
        std::fs::remove_dir_all(&logs_dir)?;
        println!("  {OK} Logs cleared");
    }

    let wallet_dir = ironclad_dir.join("wallet.json");
    if wallet_dir.exists() {
        println!("  {WARN} Wallet preserved: {}", wallet_dir.display());
    }

    println!("\n  {GREEN}Reset complete.{RESET}\n");
    Ok(())
}

