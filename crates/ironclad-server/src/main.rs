use std::net::{SocketAddr, ToSocketAddrs};
use std::path::Path;
use std::time::Instant;

use clap::{Parser, Subcommand};
use tracing::info;

use ironclad_core::config::{IroncladConfig, resolve_config_path};
use ironclad_core::style::Theme;
use ironclad_server::cli;

#[derive(Parser)]
#[command(
    name = "ironclad",
    version,
    about = "Ironclad Autonomous Agent Runtime"
)]
struct Cli {
    /// Gateway URL for management commands
    #[arg(
        long,
        global = true,
        default_value = "http://127.0.0.1:18789",
        env = "IRONCLAD_URL"
    )]
    url: String,

    /// API key for authenticating with the server
    #[arg(long, global = true, env = "IRONCLAD_API_KEY")]
    api_key: Option<String>,

    /// Profile name for state isolation
    #[arg(long, global = true, env = "IRONCLAD_PROFILE")]
    profile: Option<String>,

    /// Path to configuration file
    #[arg(short, long, global = true, env = "IRONCLAD_CONFIG")]
    config: Option<String>,

    /// Color output: auto, always, never
    #[arg(long, global = true, default_value = "auto")]
    color: String,

    /// Color theme: crt-green (default), crt-orange, terminal
    #[arg(
        long,
        global = true,
        default_value = "crt-green",
        env = "IRONCLAD_THEME"
    )]
    theme: String,

    /// Disable CRT typewriter draw effect
    #[arg(long, global = true)]
    no_draw: bool,

    /// Retro mode: CRT green tint, ASCII symbols, typewriter draw
    #[arg(long, global = true, env = "IRONCLAD_NERDMODE")]
    nerdmode: bool,

    /// Suppress informational output (errors only)
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Output structured JSON instead of formatted text
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

// ── Command hierarchy ────────────────────────────────────────

#[derive(Subcommand)]
enum Commands {
    // ── A-C ────────────────────────────────────────────────
    /// Manage agents
    #[command(next_help_heading = "A-C")]
    #[command(subcommand)]
    Agents(AgentsCmd),
    /// Manage OAuth authentication for providers
    #[command(next_help_heading = "A-C")]
    #[command(subcommand)]
    Auth(AuthCmd),
    /// Inspect channel adapters
    #[command(next_help_heading = "A-C")]
    #[command(subcommand)]
    Channels(ChannelsCmd),
    /// Validate configuration
    #[command(next_help_heading = "A-C")]
    Check,
    /// Inspect circuit breaker state
    #[command(next_help_heading = "A-C")]
    #[command(subcommand)]
    Circuit(CircuitCmd),
    /// Generate shell completions
    #[command(next_help_heading = "A-C")]
    Completion {
        /// Shell: bash, zsh, fish
        shell: String,
    },
    /// Read and write configuration
    #[command(next_help_heading = "A-C")]
    #[command(subcommand)]
    Config(ConfigCmd),

    // ── D-L ────────────────────────────────────────────────
    /// Manage daemon service
    #[command(next_help_heading = "D-L")]
    #[command(subcommand)]
    Daemon(DaemonCmd),
    /// Scan workspace for stale references, config drift, and orphaned artifacts
    #[command(next_help_heading = "D-L")]
    Defrag {
        /// Auto-fix fixable findings
        #[arg(long)]
        fix: bool,
        /// Skip confirmation prompts (use with --fix)
        #[arg(long)]
        yes: bool,
    },
    /// Ingest documents into the knowledge system
    #[command(next_help_heading = "D-L")]
    Ingest {
        /// File or directory path to ingest
        path: String,
        /// Emit machine-readable JSON output
        #[arg(long)]
        json: bool,
    },
    /// Initialize a new workspace
    #[command(next_help_heading = "D-L")]
    Init {
        /// Directory to initialize
        #[arg(default_value = ".")]
        path: String,
    },
    /// Manage encrypted credential store
    #[command(next_help_heading = "D-L")]
    #[command(subcommand)]
    Keystore(KeystoreCmd),
    /// View and tail logs
    #[command(next_help_heading = "D-L")]
    Logs {
        /// Number of lines to show
        #[arg(short = 'n', long, default_value = "50")]
        lines: usize,
        /// Follow log output (stream)
        #[arg(short, long)]
        follow: bool,
        /// Minimum log level: trace, debug, info, warn, error
        #[arg(short, long, default_value = "info")]
        level: String,
    },

    // ── M-R ────────────────────────────────────────────────
    /// Run diagnostics and self-repair
    #[command(alias = "doctor", next_help_heading = "M-R")]
    Mechanic {
        /// Attempt to auto-repair issues
        #[arg(long, short = 'r', alias = "rep")]
        repair: bool,
        /// Emit machine-readable JSON findings
        #[arg(long)]
        json: bool,
        /// Allowlisted paused cron job names to re-enable in --repair mode
        #[arg(long = "allow-job", value_delimiter = ',')]
        allow_job: Vec<String>,
    },
    /// Browse and search memory banks
    #[command(next_help_heading = "M-R")]
    #[command(subcommand)]
    Memory(MemoryCmd),
    /// View metrics and cost telemetry
    #[command(next_help_heading = "M-R")]
    #[command(subcommand)]
    Metrics(MetricsCmd),
    /// Migrate between Legacy and Ironclad
    #[command(next_help_heading = "M-R")]
    #[command(subcommand)]
    Migrate(MigrateCmd),
    /// Discover and manage models
    #[command(next_help_heading = "M-R")]
    #[command(subcommand)]
    Models(ModelsCmd),
    /// Manage plugins
    #[command(next_help_heading = "M-R")]
    #[command(subcommand)]
    Plugins(PluginsCmd),
    /// Reset state to factory defaults
    #[command(next_help_heading = "M-R")]
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    // ── S-Z ────────────────────────────────────────────────
    /// View and manage scheduled tasks
    #[command(alias = "cron", next_help_heading = "S-Z")]
    #[command(subcommand)]
    Schedule(ScheduleCmd),
    /// Security audit and hardening
    #[command(next_help_heading = "S-Z")]
    #[command(subcommand)]
    Security(SecurityCmd),
    /// Boot the Ironclad runtime
    #[command(alias = "start", alias = "run", next_help_heading = "S-Z")]
    Serve {
        /// Override bind port
        #[arg(short, long)]
        port: Option<u16>,
        /// Override bind address
        #[arg(short, long)]
        bind: Option<String>,
    },
    /// Manage sessions
    #[command(next_help_heading = "S-Z")]
    #[command(subcommand)]
    Sessions(SessionsCmd),
    /// Interactive setup wizard
    #[command(alias = "onboard", next_help_heading = "S-Z")]
    Setup,
    /// Manage skills
    #[command(next_help_heading = "S-Z")]
    #[command(subcommand)]
    Skills(SkillsCmd),
    /// Display system status
    #[command(next_help_heading = "S-Z")]
    Status,
    /// Uninstall Ironclad daemon and data
    #[command(next_help_heading = "S-Z")]
    Uninstall {
        /// Also remove ~/.ironclad/ data directory
        #[arg(long)]
        purge: bool,
    },
    /// Check for and install updates
    #[command(alias = "upgrade", next_help_heading = "S-Z")]
    #[command(subcommand)]
    Update(UpdateCmd),
    /// Report firmware version and build
    #[command(next_help_heading = "S-Z")]
    Version,
    /// Inspect wallet and treasury
    #[command(next_help_heading = "S-Z")]
    #[command(subcommand)]
    Wallet(WalletCmd),
    /// Open the web dashboard
    #[command(next_help_heading = "S-Z")]
    Web,
}

// ── Subcommand enums ────────────────────────────────────────

#[derive(Subcommand)]
enum AuthCmd {
    /// Log in to a provider via OAuth
    Login {
        /// Provider name (e.g., anthropic)
        #[arg(long)]
        provider: String,
        /// OAuth client ID (overrides config)
        #[arg(long)]
        client_id: Option<String>,
    },
    /// Show OAuth token status
    Status,
    /// Remove stored OAuth tokens for a provider
    Logout {
        /// Provider name (e.g., anthropic)
        #[arg(long)]
        provider: String,
    },
}

#[derive(Subcommand)]
enum SessionsCmd {
    /// List all sessions
    List,
    /// Show session details and messages
    Show { id: String },
    /// Create a new session
    Create { agent_id: String },
    /// Export session to file
    Export {
        /// Session ID
        id: String,
        /// Output format: json, html, markdown
        #[arg(short, long, default_value = "json")]
        format: String,
        /// Output file (default: stdout)
        #[arg(short, long)]
        output: Option<String>,
    },
    /// Backfill nicknames for all sessions missing one
    BackfillNicknames,
}

#[derive(Subcommand)]
enum MemoryCmd {
    /// List entries in a memory tier
    List {
        /// Memory tier: working, episodic, semantic
        tier: String,
        /// Session ID (required for working memory)
        #[arg(short, long)]
        session: Option<String>,
        /// Limit results
        #[arg(short, long)]
        limit: Option<i64>,
    },
    /// Search across memory tiers
    Search {
        /// Search query
        query: String,
        /// Limit results
        #[arg(short, long)]
        limit: Option<i64>,
    },
}

#[derive(Subcommand)]
enum ScheduleCmd {
    /// List scheduled tasks
    List,
    /// Execute a scheduled task once immediately
    Run {
        /// Cron job name or ID
        job: String,
    },
    /// Re-enable paused cron jobs after unknown-action containment
    Recover {
        /// Re-enable all paused jobs
        #[arg(long)]
        all: bool,
        /// Re-enable only specific job names (repeatable)
        #[arg(long = "name")]
        names: Vec<String>,
        /// Preview what would be changed without writing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(Subcommand)]
enum MetricsCmd {
    /// Show inference cost breakdown
    Costs,
    /// Show transaction history
    Transactions {
        /// Time window in hours
        #[arg(short = 'H', long)]
        hours: Option<i64>,
    },
    /// Show semantic cache statistics
    Cache,
}

#[derive(Subcommand)]
enum WalletCmd {
    /// Show wallet overview
    Show,
    /// Display wallet address
    Address,
    /// Check wallet balance
    Balance,
}

#[derive(Subcommand)]
enum CircuitCmd {
    /// Show circuit breaker status
    Status,
    /// Reset tripped circuit breakers
    Reset {
        /// Reset only one provider breaker (e.g., openai)
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
enum ChannelsCmd {
    /// List channel adapters and their status
    List,
    /// List dead-letter channel deliveries
    DeadLetter {
        /// Maximum number of dead-letter rows to show
        #[arg(long, default_value_t = 50)]
        limit: usize,
    },
    /// Replay a dead-letter delivery by id
    Replay { id: String },
}

#[derive(Subcommand)]
enum ModelsCmd {
    /// List configured models
    List,
    /// Scan providers for available models
    Scan {
        /// Provider to scan (e.g., ollama, openai)
        provider: Option<String>,
    },
}

#[derive(Subcommand)]
enum AgentsCmd {
    /// List all agents
    List,
    /// Start an agent
    Start { id: String },
    /// Stop an agent
    Stop { id: String },
}

#[derive(Subcommand)]
enum DaemonCmd {
    /// Install daemon service (LaunchAgent/systemd/Windows Service)
    Install {
        /// Path to config file
        #[arg(short, long, default_value = "ironclad.toml")]
        config: String,
        /// Start the daemon immediately after install without prompting
        #[arg(long)]
        start: bool,
    },
    /// Start the daemon
    Start,
    /// Stop the daemon
    Stop,
    /// Restart the daemon
    Restart,
    /// Show daemon status
    Status,
    /// Uninstall daemon service
    Uninstall,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show running configuration (from gateway)
    Show,
    /// Get a config value by TOML path
    Get { path: String },
    /// Set a config value
    Set {
        /// TOML path (e.g., models.primary)
        path: String,
        /// New value
        value: String,
        /// Config file to modify
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
        /// Skip immediate runtime apply via API
        #[arg(long, default_value_t = false)]
        no_apply: bool,
    },
    /// Remove a config key
    Unset {
        /// TOML path to remove
        path: String,
        /// Config file to modify
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
        /// Skip immediate runtime apply via API
        #[arg(long, default_value_t = false)]
        no_apply: bool,
    },
    /// Lint/validate a config file without applying it
    Lint {
        /// Config file to validate
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
    },
    /// Create a timestamped backup of a config file
    Backup {
        /// Config file to back up
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
    },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// List all registered skills
    List,
    /// Show skill details
    Show { id: String },
    /// Reload skills from disk
    Reload,
    /// List/search remote skills catalog entries
    CatalogList {
        /// Optional case-insensitive search query
        #[arg(long)]
        query: Option<String>,
    },
    /// Install skills from the catalog
    CatalogInstall {
        /// Skill names to install (accepts names or filenames)
        skills: Vec<String>,
        /// Activate (reload) after install
        #[arg(long)]
        activate: bool,
    },
    /// Activate installed skills (reload)
    CatalogActivate {
        /// Optional skill names for operator context
        skills: Vec<String>,
    },
    /// Import skills from an Legacy workspace or archive
    Import {
        /// Path to Legacy workspace/skills directory or .tar.gz archive
        source: String,
        /// Skip safety checks (dangerous)
        #[arg(long)]
        no_safety_check: bool,
        /// Auto-accept warnings (still blocks on critical findings)
        #[arg(long)]
        accept_warnings: bool,
    },
    /// Export skills to a portable archive
    Export {
        /// Output path for the archive (.tar.gz)
        #[arg(short, long, default_value = "ironclad-skills-export.tar.gz")]
        output: String,
        /// Specific skill IDs (default: all)
        ids: Vec<String>,
    },
}

#[derive(Subcommand)]
enum MigrateCmd {
    /// Import data from an Legacy workspace into Ironclad
    Import {
        /// Path to Legacy workspace root
        source: String,
        /// Specific areas to import (default: all)
        #[arg(short, long, value_delimiter = ',')]
        areas: Vec<String>,
        /// Skip confirmation prompts
        #[arg(long)]
        yes: bool,
        /// Skip safety checks on skill scripts
        #[arg(long)]
        no_safety_check: bool,
    },
    /// Export Ironclad data to Legacy format
    Export {
        /// Output directory for the Legacy workspace
        target: String,
        /// Specific areas to export (default: all)
        #[arg(short, long, value_delimiter = ',')]
        areas: Vec<String>,
    },
}

#[derive(Subcommand)]
enum PluginsCmd {
    /// List installed plugins
    List,
    /// Show plugin details
    Info { name: String },
    /// Install a plugin from a local dir, .ic.zip archive, or catalog name
    Install {
        /// Plugin source: directory path, .ic.zip path, or catalog plugin name
        source: String,
    },
    /// Uninstall a plugin
    Uninstall { name: String },
    /// Enable a disabled plugin
    Enable { name: String },
    /// Disable a plugin
    Disable { name: String },
    /// Search the plugin catalog
    Search {
        /// Search query (matches name, description, author)
        query: String,
    },
    /// Pack a plugin directory into a distributable .ic.zip archive
    Pack {
        /// Path to the plugin directory containing plugin.toml
        dir: String,
        /// Output directory for the archive (default: current directory)
        #[arg(short, long)]
        output: Option<String>,
    },
}

#[derive(Subcommand)]
enum KeystoreCmd {
    /// Store a secret in the keystore
    Set {
        /// Secret name
        key: String,
        /// Secret value (omit for interactive prompt)
        value: Option<String>,
        /// Custom passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
    /// Retrieve a secret
    Get {
        /// Secret name
        key: String,
        /// Custom passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
    /// List all stored secret names
    List {
        /// Custom passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
    /// Remove a secret
    Remove {
        /// Secret name
        key: String,
        /// Custom passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
    /// Import secrets from a JSON file
    Import {
        /// Path to JSON file with {"key": "value", ...} format
        path: String,
        /// Custom passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
    /// Change the keystore passphrase
    Rekey {
        /// Current passphrase (default: machine-derived key)
        #[arg(long)]
        password: Option<String>,
    },
}

#[derive(Subcommand)]
enum SecurityCmd {
    /// Run security audit on configuration and permissions
    Audit {
        /// Config file to audit
        #[arg(short, long, default_value = "ironclad.toml")]
        config: String,
    },
}

#[derive(Subcommand)]
enum UpdateCmd {
    /// Show available updates without installing anything
    Check {
        /// Update channel: stable, beta, dev
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Override registry URL for content packs
        #[arg(long, env = "IRONCLAD_REGISTRY_URL")]
        registry_url: Option<String>,
    },
    /// Update everything (binary + content packs)
    All {
        /// Update channel: stable, beta, dev
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Auto-accept unmodified files (still prompts for conflicts)
        #[arg(long)]
        yes: bool,
        /// Don't restart daemon after update
        #[arg(long)]
        no_restart: bool,
        /// Override registry URL for content packs
        #[arg(long, env = "IRONCLAD_REGISTRY_URL")]
        registry_url: Option<String>,
    },
    /// Update the Ironclad binary (download or build)
    Binary {
        /// Update channel: stable, beta, dev
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Auto-accept if newer version is available
        #[arg(long)]
        yes: bool,
        /// Update method: download (default) or build
        #[arg(long, default_value = "download", value_parser = ["download", "build"])]
        method: String,
    },
    /// Update bundled provider configurations
    Providers {
        /// Auto-accept unmodified files (still prompts for conflicts)
        #[arg(long)]
        yes: bool,
        /// Override registry URL
        #[arg(long, env = "IRONCLAD_REGISTRY_URL")]
        registry_url: Option<String>,
    },
    /// Update blessed skill pack
    Skills {
        /// Auto-accept unmodified files (still prompts for conflicts)
        #[arg(long)]
        yes: bool,
        /// Override registry URL
        #[arg(long, env = "IRONCLAD_REGISTRY_URL")]
        registry_url: Option<String>,
    },
}

fn prompt_yes_no(question: &str) -> bool {
    use std::io::Write;
    eprint!("  {question} [y/N] ");
    // best-effort: flush failure is non-critical for interactive prompt
    std::io::stderr().flush().ok();
    let mut input = String::new();
    if std::io::stdin().read_line(&mut input).is_err() {
        return false;
    }
    matches!(input.trim(), "y" | "Y" | "yes" | "Yes" | "YES")
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = Cli::parse();
    cli::init_theme(
        &parsed.color,
        &parsed.theme,
        parsed.no_draw,
        parsed.nerdmode,
    );
    cli::init_api_key(parsed.api_key.clone());
    let t = cli::theme();
    eprint!("{}", t.reset());
    let url = if parsed.url == "http://127.0.0.1:18789" {
        // Default URL — try to derive server address from resolved config
        resolve_config_path(parsed.config.as_deref())
            .and_then(|p| {
                std::fs::read_to_string(p)
                    .inspect_err(|e| {
                        tracing::warn!("failed to read config for URL resolution: {e}")
                    })
                    .ok()
            })
            .and_then(|contents| {
                IroncladConfig::from_str(&contents)
                    .inspect_err(|e| {
                        tracing::warn!("failed to parse config for URL resolution: {e}")
                    })
                    .ok()
            })
            .map(|cfg| format!("http://{}:{}", cfg.server.bind, cfg.server.port))
            .unwrap_or_else(|| parsed.url.clone())
    } else {
        parsed.url.clone()
    };
    let url = &url;
    let config_flag = parsed.config.clone();

    let result = match parsed.command {
        // ── Lifecycle ───────────────────────────────────────
        Some(Commands::Serve { port, bind }) => cmd_serve(config_flag.clone(), port, bind).await,
        Some(Commands::Init { path }) => cmd_init(&path),
        Some(Commands::Setup) => cli::cmd_setup(),
        Some(Commands::Check) => match resolve_config_path(config_flag.as_deref()) {
            Some(p) => cmd_check(&p.to_string_lossy()),
            None => {
                let t = cli::theme();
                print_banner(t);
                eprintln!("  {} No configuration file found.", t.icon_warn());
                eprintln!("    Searched: ~/.ironclad/ironclad.toml, ./ironclad.toml");
                eprintln!(
                    "    Specify a path with {}--config <path>{} or create one with {}ironclad init{}",
                    t.bold(),
                    t.reset(),
                    t.bold(),
                    t.reset()
                );
                eprintln!();
                Err("no configuration file found".into())
            }
        },
        Some(Commands::Version) => {
            cmd_version(parsed.json);
            Ok(())
        }
        Some(Commands::Update(subcmd)) => {
            let resolved = resolve_config_path(parsed.config.as_deref());
            let config_path = resolved
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "ironclad.toml".into());
            let config_path = config_path.as_str();
            match subcmd {
                UpdateCmd::Check {
                    channel,
                    registry_url,
                } => cli::cmd_update_check(&channel, registry_url.as_deref(), config_path).await,
                UpdateCmd::All {
                    channel,
                    yes,
                    no_restart,
                    registry_url,
                } => {
                    cli::cmd_update_all(
                        &channel,
                        yes,
                        no_restart,
                        registry_url.as_deref(),
                        config_path,
                    )
                    .await
                }
                UpdateCmd::Binary {
                    channel,
                    yes,
                    method,
                } => cli::cmd_update_binary(&channel, yes, &method).await,
                UpdateCmd::Providers { yes, registry_url } => {
                    cli::cmd_update_providers(yes, registry_url.as_deref(), config_path).await
                }
                UpdateCmd::Skills { yes, registry_url } => {
                    cli::cmd_update_skills(yes, registry_url.as_deref(), config_path).await
                }
            }
        }

        // ── Operations ──────────────────────────────────────
        Some(Commands::Status) => cli::cmd_status(url, parsed.json).await,
        Some(Commands::Mechanic {
            repair,
            json,
            allow_job,
        }) => cli::cmd_mechanic(url, repair, json, &allow_job).await,
        Some(Commands::Defrag { fix, yes }) => {
            let workspace = std::path::Path::new(".");
            cli::cmd_defrag(workspace, fix, yes, parsed.json).map_err(|e| e.into())
        }
        Some(Commands::Logs {
            lines,
            follow,
            level,
        }) => cli::cmd_logs(url, lines, follow, &level).await,
        Some(Commands::Circuit(sub)) => match sub {
            CircuitCmd::Status => cli::cmd_circuit_status(url).await,
            CircuitCmd::Reset { provider } => {
                cli::cmd_circuit_reset(url, provider.as_deref()).await
            }
        },

        // ── Data ────────────────────────────────────────────
        Some(Commands::Sessions(sub)) => match sub {
            SessionsCmd::List => cli::cmd_sessions_list(url).await,
            SessionsCmd::Show { id } => cli::cmd_session_detail(url, &id).await,
            SessionsCmd::Create { agent_id } => cli::cmd_session_create(url, &agent_id).await,
            SessionsCmd::Export { id, format, output } => {
                cli::cmd_session_export(url, &id, &format, output.as_deref()).await
            }
            SessionsCmd::BackfillNicknames => cli::cmd_sessions_backfill_nicknames(url).await,
        },
        Some(Commands::Memory(sub)) => match sub {
            MemoryCmd::List {
                tier,
                session,
                limit,
            } => cli::cmd_memory(url, &tier, session.as_deref(), None, limit).await,
            MemoryCmd::Search { query, limit } => {
                cli::cmd_memory(url, "search", None, Some(query.as_str()), limit).await
            }
        },
        Some(Commands::Ingest { path, json }) => cmd_ingest(&path, json, config_flag.as_deref()),
        Some(Commands::Skills(sub)) => match sub {
            SkillsCmd::List => cli::cmd_skills_list(url).await,
            SkillsCmd::Show { id } => cli::cmd_skill_detail(url, &id).await,
            SkillsCmd::Reload => cli::cmd_skills_reload(url).await,
            SkillsCmd::CatalogList { query } => {
                cli::cmd_skills_catalog_list(url, query.as_deref()).await
            }
            SkillsCmd::CatalogInstall { skills, activate } => {
                cli::cmd_skills_catalog_install(url, &skills, activate).await
            }
            SkillsCmd::CatalogActivate { skills } => {
                cli::cmd_skills_catalog_activate(url, &skills).await
            }
            SkillsCmd::Import {
                source,
                no_safety_check,
                accept_warnings,
            } => ironclad_server::migrate::cmd_skill_import(
                &source,
                no_safety_check,
                accept_warnings,
            ),
            SkillsCmd::Export { output, ids } => {
                ironclad_server::migrate::cmd_skill_export(&output, &ids)
            }
        },
        Some(Commands::Schedule(sub)) => match sub {
            ScheduleCmd::List => cli::cmd_schedule_list(url).await,
            ScheduleCmd::Run { job } => cli::cmd_schedule_run(url, &job).await,
            ScheduleCmd::Recover {
                all,
                names,
                dry_run,
            } => cli::cmd_schedule_recover(url, &names, all, dry_run).await,
        },
        Some(Commands::Metrics(sub)) => match sub {
            MetricsCmd::Costs => cli::cmd_metrics(url, "costs", None).await,
            MetricsCmd::Transactions { hours } => {
                cli::cmd_metrics(url, "transactions", hours).await
            }
            MetricsCmd::Cache => cli::cmd_metrics(url, "cache", None).await,
        },
        Some(Commands::Wallet(sub)) => match sub {
            WalletCmd::Show => cli::cmd_wallet(url).await,
            WalletCmd::Address => cli::cmd_wallet_address(url).await,
            WalletCmd::Balance => cli::cmd_wallet_balance(url).await,
        },

        // ── Authentication ──────────────────────────────────
        Some(Commands::Auth(sub)) => match sub {
            AuthCmd::Login {
                provider,
                client_id,
            } => cmd_auth_login(&provider, client_id.as_deref()).await,
            AuthCmd::Status => cmd_auth_status().await,
            AuthCmd::Logout { provider } => cmd_auth_logout(&provider).await,
        },

        // ── Configuration ───────────────────────────────────
        Some(Commands::Config(sub)) => match sub {
            ConfigCmd::Show => cli::cmd_config(url).await,
            ConfigCmd::Get { path } => cli::cmd_config_get(url, &path).await,
            ConfigCmd::Set {
                path,
                value,
                file,
                no_apply,
            } => {
                cli::cmd_config_set(&path, &value, &file)?;
                if !no_apply {
                    cli::cmd_config_apply(url, &file).await?;
                }
                Ok(())
            }
            ConfigCmd::Unset {
                path,
                file,
                no_apply,
            } => {
                cli::cmd_config_unset(&path, &file)?;
                if !no_apply {
                    cli::cmd_config_apply(url, &file).await?;
                }
                Ok(())
            }
            ConfigCmd::Lint { file } => cli::cmd_config_lint(&file),
            ConfigCmd::Backup { file } => cli::cmd_config_backup(&file),
        },
        Some(Commands::Models(sub)) => match sub {
            ModelsCmd::List => cli::cmd_models_list(url).await,
            ModelsCmd::Scan { provider } => cli::cmd_models_scan(url, provider.as_deref()).await,
        },
        Some(Commands::Plugins(sub)) => match sub {
            PluginsCmd::List => cli::cmd_plugins_list(url).await,
            PluginsCmd::Info { name } => cli::cmd_plugin_info(url, &name).await,
            PluginsCmd::Install { source } => cli::cmd_plugin_install(&source).await,
            PluginsCmd::Uninstall { name } => cli::cmd_plugin_uninstall(&name),
            PluginsCmd::Enable { name } => cli::cmd_plugin_toggle(url, &name, true).await,
            PluginsCmd::Disable { name } => cli::cmd_plugin_toggle(url, &name, false).await,
            PluginsCmd::Search { query } => cli::cmd_plugin_search(&query).await,
            PluginsCmd::Pack { dir, output } => cli::cmd_plugin_pack(&dir, output.as_deref()),
        },
        Some(Commands::Agents(sub)) => match sub {
            AgentsCmd::List => cli::cmd_agents_list(url).await,
            AgentsCmd::Start { id } => cli::cmd_agent_start(url, &id).await,
            AgentsCmd::Stop { id } => cli::cmd_agent_stop(url, &id).await,
        },
        Some(Commands::Channels(sub)) => match sub {
            ChannelsCmd::List => cli::cmd_channels_status(url).await,
            ChannelsCmd::DeadLetter { limit } => cli::cmd_channels_dead_letter(url, limit).await,
            ChannelsCmd::Replay { id } => cli::cmd_channels_replay(url, &id).await,
        },
        Some(Commands::Security(sub)) => match sub {
            SecurityCmd::Audit { config } => cli::cmd_security_audit(&config),
        },

        // ── Credentials ──────────────────────────────────────
        Some(Commands::Keystore(sub)) => cmd_keystore(sub).await,

        // ── Migration ────────────────────────────────────────
        Some(Commands::Migrate(sub)) => match sub {
            MigrateCmd::Import {
                source,
                areas,
                yes,
                no_safety_check,
            } => {
                ironclad_server::migrate::cmd_migrate_import(&source, &areas, yes, no_safety_check)
            }
            MigrateCmd::Export { target, areas } => {
                ironclad_server::migrate::cmd_migrate_export(&target, &areas)
            }
        },

        // ── System ──────────────────────────────────────────
        Some(Commands::Daemon(sub)) => match sub {
            DaemonCmd::Install { config, start } => {
                let binary = std::env::current_exe()?.to_string_lossy().to_string();
                let abs_config = std::path::Path::new(&config)
                    .canonicalize()
                    .or_else(|_| {
                        let home_cfg = ironclad_core::home_dir().join(".ironclad").join(&config);
                        home_cfg.canonicalize()
                    })
                    .map_err(|_| {
                        ironclad_core::IroncladError::Config(format!(
                            "config file not found: {config}"
                        ))
                    })?;
                let path = ironclad_server::daemon::install_daemon(
                    &binary,
                    &abs_config.to_string_lossy(),
                    18789,
                )?;
                eprintln!("  Daemon installed: {}", path.display());

                let should_start =
                    start || prompt_yes_no("Would you like to start the daemon now?");
                if should_start {
                    ironclad_server::daemon::start_daemon()?;
                    eprintln!("  Daemon started");
                } else {
                    eprintln!("  Run `ironclad daemon start` when you're ready");
                }
                Ok(())
            }
            DaemonCmd::Start => {
                if !ironclad_server::daemon::is_installed() {
                    eprintln!("  Daemon is not installed. Run `ironclad daemon install` first.");
                    std::process::exit(1);
                }
                ironclad_server::daemon::start_daemon()?;
                eprintln!("  Daemon started");
                Ok(())
            }
            DaemonCmd::Stop => {
                ironclad_server::daemon::stop_daemon()?;
                eprintln!("  Daemon stopped");
                Ok(())
            }
            DaemonCmd::Restart => {
                if !ironclad_server::daemon::is_installed() {
                    eprintln!("  Daemon is not installed. Run `ironclad daemon install` first.");
                    std::process::exit(1);
                }
                ironclad_server::daemon::restart_daemon()?;
                eprintln!("  Daemon restarted");
                Ok(())
            }
            DaemonCmd::Status => {
                let status = ironclad_server::daemon::daemon_status()?;
                eprintln!("  {status}");
                Ok(())
            }
            DaemonCmd::Uninstall => {
                ironclad_server::daemon::uninstall_daemon()?;
                eprintln!("  Daemon uninstalled");
                Ok(())
            }
        },
        Some(Commands::Web) => cmd_web(config_flag.as_deref(), url),
        Some(Commands::Uninstall { purge }) => cli::cmd_uninstall(purge),
        Some(Commands::Reset { yes }) => cli::cmd_reset(yes),
        Some(Commands::Completion { shell }) => cli::cmd_completion(&shell),

        // No subcommand: show help
        None => {
            use clap::CommandFactory;
            let mut cmd = Cli::command();
            cmd.print_help()?;
            eprintln!();
            Ok(())
        }
    };
    eprint!("{}", t.hard_reset());
    result
}

const BANNER: &str = include_str!("../banner.txt");

fn print_banner(t: &Theme) {
    use ironclad_core::style::sleep_ms;

    let version = env!("CARGO_PKG_VERSION");
    let p = t.accent();
    let d = t.dim();
    let r = t.reset();
    let scan = if t.colors_enabled() { 55 } else { 0 };

    eprintln!();
    for line in BANNER.lines() {
        if line.contains("I R O N C L A D") {
            let (art, _) = line.split_once("I R O N C L A D").unwrap();
            eprint!("{p}{art}{r}");
            t.typewrite_line(&format!("{p}I R O N C L A D{r}"), 35);
        } else if line.contains("Autonomous Agent Runtime") {
            let (art, _) = line.split_once("Autonomous Agent Runtime").unwrap();
            eprint!("{p}{art}{r}");
            t.typewrite_line(&format!("{d}Autonomous Agent Runtime v{version}{r}"), 18);
        } else {
            eprintln!("{p}{line}{r}");
            sleep_ms(scan);
        }
    }
    eprintln!();
}

fn step(t: &Theme, n: u32, total: u32, msg: &str) {
    let (d, b, r) = (t.dim(), t.bold(), t.reset());
    let ok = t.icon_ok();
    t.typewrite_line(&format!("  {ok} {d}[{n:>2}/{total}]{r} {b}{msg}{r}"), 4);
}

fn step_warn(t: &Theme, n: u32, total: u32, msg: &str) {
    let (d, r) = (t.dim(), t.reset());
    let warn = t.icon_warn();
    t.typewrite_line(&format!("  {warn} {d}[{n:>2}/{total}]{r} {msg}"), 4);
}

fn step_detail(t: &Theme, label: &str, value: &str) {
    let (d, a, r) = (t.dim(), t.accent(), t.reset());
    let detail = t.icon_detail();
    t.typewrite_line(&format!("       {detail} {d}{label}: {a}{value}{r}"), 4);
}

async fn cmd_auth_login(
    provider: &str,
    client_id_override: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    let (a, d, r) = (t.accent(), t.dim(), t.reset());

    let client_id = client_id_override
        .map(String::from)
        .or_else(|| {
            let path = ironclad_core::home_dir()
                .join(".ironclad")
                .join("ironclad.toml");
            let cfg = IroncladConfig::from_file(&path).ok()?;
            cfg.providers
                .get(provider)
                .and_then(|p| p.oauth_client_id.clone())
        })
        .unwrap_or_else(|| {
            std::env::var("IRONCLAD_OAUTH_CLIENT_ID").unwrap_or_else(|_| "ironclad-cli".into())
        });

    let verifier = ironclad_llm::oauth::generate_code_verifier();
    let challenge = ironclad_llm::oauth::compute_code_challenge(&verifier);
    let state_param = ironclad_llm::oauth::generate_code_verifier();
    let redirect_uri = ironclad_llm::oauth::default_redirect_uri();
    let auth_url = ironclad_llm::oauth::build_authorization_url(
        &client_id,
        &redirect_uri,
        &challenge,
        &state_param,
    );

    eprintln!("\n  {a}OAuth Login — {provider}{r}\n");
    eprintln!("  {d}Opening browser for authorization...{r}");
    eprintln!("  {d}If the browser doesn't open, visit:{r}");
    eprintln!("  {a}{auth_url}{r}\n");

    let _ = open::that(&auth_url);

    eprintln!(
        "  {d}Waiting for callback on port {}...{r}",
        ironclad_llm::oauth::callback_port()
    );

    let (code, returned_state) = listen_for_callback().await?;

    if returned_state != state_param {
        return Err("OAuth state mismatch — possible CSRF attack".into());
    }

    eprintln!("  {a}Authorization code received, exchanging for tokens...{r}");

    let http = reqwest::Client::new();
    let mut params = std::collections::HashMap::new();
    params.insert("grant_type", "authorization_code");
    params.insert("code", &code);
    params.insert("redirect_uri", &redirect_uri);
    params.insert("client_id", &client_id);
    params.insert("code_verifier", &verifier);

    let resp = http
        .post(ironclad_llm::oauth::token_url())
        .form(&params)
        .send()
        .await?;

    if !resp.status().is_success() {
        let body = resp
            .text()
            .await
            .inspect_err(|e| tracing::warn!(error = %e, "CLI response parse failed"))
            .unwrap_or_default();
        return Err(format!("Token exchange failed: {body}").into());
    }

    #[derive(serde::Deserialize)]
    struct TokenResp {
        access_token: String,
        refresh_token: Option<String>,
        expires_in: Option<i64>,
    }

    let token_resp: TokenResp = resp.json().await?;
    let expires_at = token_resp
        .expires_in
        .map(|secs| chrono::Utc::now().timestamp() + secs);

    let manager = ironclad_llm::OAuthManager::new()?;
    manager
        .store_tokens(ironclad_llm::oauth::StoredTokens {
            provider: provider.to_string(),
            access_token: token_resp.access_token,
            refresh_token: token_resp.refresh_token,
            expires_at,
            client_id: None,
        })
        .await;

    let ok = t.icon_ok();
    eprintln!("\n  {ok} {a}Successfully authenticated with {provider}{r}");
    eprintln!("  {d}Tokens stored in the encrypted keystore{r}\n");
    eprintln!("  {d}To use OAuth auth, set auth_mode = \"oauth\" in your provider config:{r}");
    eprintln!("  {d}  [providers.{provider}]{r}");
    eprintln!("  {d}  auth_mode = \"oauth\"{r}\n");

    Ok(())
}

async fn listen_for_callback() -> Result<(String, String), Box<dyn std::error::Error>> {
    use tokio::io::AsyncReadExt;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    let addr = format!("127.0.0.1:{}", ironclad_llm::oauth::callback_port());
    let listener = TcpListener::bind(&addr).await?;

    let (mut stream, _) = listener.accept().await?;

    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await?;
    let request = String::from_utf8_lossy(&buf[..n]);

    let first_line = request.lines().next().unwrap_or("");
    let path = first_line.split_whitespace().nth(1).unwrap_or("/");

    let mut code = String::new();
    let mut state = String::new();

    if let Some(query_start) = path.find('?') {
        let query = &path[query_start + 1..];
        for pair in query.split('&') {
            if let Some((k, v)) = pair.split_once('=') {
                match k {
                    "code" => code = v.to_string(),
                    "state" => state = v.to_string(),
                    _ => {}
                }
            }
        }
    }

    let html = "<html><body><h2>Authentication successful!</h2><p>You can close this window and return to the terminal.</p></body></html>";
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream.write_all(response.as_bytes()).await?;

    if code.is_empty() {
        return Err("No authorization code received in callback".into());
    }

    Ok((code, state))
}

async fn cmd_auth_status() -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    let (a, d, r) = (t.accent(), t.dim(), t.reset());

    let manager = ironclad_llm::OAuthManager::new()?;
    let statuses = manager.status().await;

    eprintln!("\n  {a}OAuth Token Status{r}\n");

    if statuses.is_empty() {
        eprintln!("  {d}No OAuth tokens stored.{r}");
        eprintln!("  {d}Run `ironclad auth login --provider <name>` to authenticate.{r}\n");
        return Ok(());
    }

    for s in &statuses {
        let status_icon = if s.expired {
            t.icon_warn()
        } else {
            t.icon_ok()
        };
        let status_text = if s.expired { "EXPIRED" } else { "active" };
        eprintln!("  {status_icon} {a}{}{r}  {d}{status_text}{r}", s.provider);
        if let Some(exp) = s.expires_at {
            let dt = chrono::DateTime::from_timestamp(exp, 0)
                .map(|d| d.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "unknown".into());
            eprintln!("       {d}expires: {dt}{r}");
        }
        let refresh = if s.has_refresh_token { "yes" } else { "no" };
        eprintln!("       {d}refresh token: {refresh}{r}");
    }
    eprintln!();

    Ok(())
}

async fn cmd_auth_logout(provider: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    let (a, d, r) = (t.accent(), t.dim(), t.reset());

    let manager = ironclad_llm::OAuthManager::new()?;
    let removed = manager.remove_tokens(provider).await;

    if removed {
        let ok = t.icon_ok();
        eprintln!("\n  {ok} {a}Removed OAuth tokens for {provider}{r}\n");
    } else {
        eprintln!("\n  {d}No tokens found for provider '{provider}'{r}\n");
    }

    Ok(())
}

fn open_keystore(
    password: &Option<String>,
) -> Result<ironclad_core::keystore::Keystore, Box<dyn std::error::Error>> {
    let ks =
        ironclad_core::keystore::Keystore::new(ironclad_core::keystore::Keystore::default_path());
    match password {
        Some(p) => ks.unlock(p)?,
        None => ks.unlock_machine()?,
    }
    Ok(ks)
}

async fn cmd_keystore(sub: KeystoreCmd) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    let (a, d, r) = (t.accent(), t.dim(), t.reset());
    let ok = t.icon_ok();

    match sub {
        KeystoreCmd::Set {
            key,
            value,
            password,
        } => {
            let ks = open_keystore(&password)?;

            let secret = match value {
                Some(v) => v,
                None => dialoguer::Password::new()
                    .with_prompt("Secret value")
                    .interact()?,
            };
            ks.set(&key, &secret)?;
            eprintln!("  {ok} {a}Stored secret '{key}'{r}");
        }
        KeystoreCmd::Get { key, password } => {
            let ks = open_keystore(&password)?;

            match ks.get(&key) {
                Some(val) => println!("{val}"),
                None => {
                    eprintln!("  {d}Key '{key}' not found{r}");
                    std::process::exit(1);
                }
            }
        }
        KeystoreCmd::List { password } => {
            let ks = open_keystore(&password)?;

            let mut keys = ks.list_keys();
            keys.sort();
            if keys.is_empty() {
                eprintln!("  {d}Keystore is empty{r}");
            } else {
                for k in &keys {
                    eprintln!("  {a}{k}{r}");
                }
                eprintln!("\n  {d}{} secret(s){r}", keys.len());
            }
        }
        KeystoreCmd::Remove { key, password } => {
            let ks = open_keystore(&password)?;

            if ks.remove(&key)? {
                eprintln!("  {ok} {a}Removed '{key}'{r}");
            } else {
                eprintln!("  {d}Key '{key}' not found{r}");
            }
        }
        KeystoreCmd::Import { path, password } => {
            let ks = open_keystore(&password)?;

            let contents = std::fs::read_to_string(&path)?;
            let entries: std::collections::HashMap<String, String> =
                serde_json::from_str(&contents)?;
            let count = ks.import(entries)?;
            eprintln!("  {ok} {a}Imported {count} secret(s){r}");
        }
        KeystoreCmd::Rekey { password } => {
            let ks = open_keystore(&password)?;

            let new_pass = dialoguer::Password::new()
                .with_prompt("New passphrase")
                .with_confirmation("Confirm new passphrase", "Passphrases do not match")
                .interact()?;
            ks.rekey(&new_pass)?;
            eprintln!("  {ok} {a}Keystore re-encrypted with new passphrase{r}");
        }
    }

    Ok(())
}

async fn cmd_serve(
    config_path: Option<String>,
    port_override: Option<u16>,
    bind_override: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    let boot_start = Instant::now();
    print_banner(t);

    const STEPS: u32 = 12;

    let resolved_path =
        resolve_config_path(config_path.as_deref()).map(|p| p.to_string_lossy().into_owned());

    let mut config = match resolved_path {
        Some(ref p) => {
            step(t, 1, STEPS, "Loading configuration");
            step_detail(t, "source", p);
            IroncladConfig::from_file(Path::new(p))?
        }
        None => {
            step(t, 1, STEPS, "Using default configuration");
            step_detail(t, "source", "built-in defaults");
            IroncladConfig::from_str(FALLBACK_CONFIG)?
        }
    };

    if let Some(p) = port_override {
        config.server.port = p;
    }
    if let Some(b) = bind_override {
        config.server.bind = b;
    }

    match legacy_loopback_mode() {
        LegacyLoopbackMode::MigrateDeprecated => {
            let migrations =
                migrate_legacy_proxy_urls(&mut config, resolved_path.as_deref().map(Path::new))?;
            if !migrations.is_empty() {
                step_warn(
                    t,
                    2,
                    STEPS,
                    &format!(
                        "Migrated {} legacy provider URL(s) from loopback proxy to in-process routing",
                        migrations.len()
                    ),
                );
                for m in &migrations {
                    step_detail(t, &format!("providers.{}", m.provider), &m.to_url);
                }
                step_warn(
                    t,
                    2,
                    STEPS,
                    "Legacy loopback provider URLs are deprecated and will be removed in v0.8.0",
                );
            }
        }
        LegacyLoopbackMode::Unsupported => {
            if let Err(msg) =
                validate_legacy_loopback_urls_for_mode(&config, LegacyLoopbackMode::Unsupported)
            {
                let (er, r) = (t.error(), t.reset());
                let err_icon = t.icon_error();
                eprintln!(
                    "  {er}{err_icon}{r} Legacy loopback provider URLs are not supported in v0.8.0+"
                );
                for item in collect_legacy_loopback_providers(&config) {
                    step_detail(t, "update required", &item);
                }
                return Err(msg.into());
            }
        }
    }

    config.validate().map_err(|e| {
        let (er, r) = (t.error(), t.reset());
        let err_icon = t.icon_error();
        eprintln!("  {er}{err_icon}{r} Configuration validation failed: {e}");
        e
    })?;
    step(t, 2, STEPS, "Configuration validated");

    let unreachable_proxies = check_internal_proxy_reachability(&config);
    if !unreachable_proxies.is_empty() {
        let (w, r) = (t.warn(), t.reset());
        let warn_icon = t.icon_warn();
        eprintln!(
            "  {w}{warn_icon}{r} Some local provider proxies are not currently reachable: {}",
            unreachable_proxies.join(", ")
        );
        eprintln!(
            "         These providers will be skipped until reachable (circuit breaker will manage availability)."
        );
    }

    let is_localhost = config.server.bind == "127.0.0.1"
        || config.server.bind == "localhost"
        || config.server.bind == "::1";
    if !is_localhost && config.server.api_key.is_none() {
        let (er, r) = (t.error(), t.reset());
        eprintln!();
        eprintln!(
            "  {er}ERROR:{r} Server bound to {} without API key.",
            config.server.bind
        );
        eprintln!("         Set [server] api_key = \"your-secret\" in config to secure the API.");
        eprintln!();
        return Err("Refusing to start on non-localhost without API key".into());
    }

    let app = ironclad_server::bootstrap_with_config_path(
        config.clone(),
        resolved_path.clone().map(std::path::PathBuf::from),
    )
    .await?;
    step(t, 3, STEPS, "Tracing initialized");
    step_detail(t, "level", &config.agent.log_level);

    let db_path = config.database.path.to_string_lossy();
    step(t, 4, STEPS, "Database initialized");
    step_detail(t, "path", &db_path);
    if db_path == ":memory:" {
        step_detail(t, "mode", "in-memory (ephemeral)");
    } else {
        step_detail(t, "mode", "WAL (persistent)");
    }

    step(t, 5, STEPS, "Wallet service ready");
    step_detail(t, "chain", &format!("chain_id={}", config.wallet.chain_id));
    step_detail(t, "rpc", &config.wallet.rpc_url);

    step(t, 6, STEPS, "Identity resolved");
    step_detail(t, "name", &config.agent.name);
    step_detail(t, "id", &config.agent.id);

    // LlmService is already constructed inside bootstrap_with_config_path and
    // stored in AppState.  No need to instantiate a second (dead) copy here.
    step(t, 7, STEPS, "LLM service ready");
    step_detail(t, "primary", &config.models.primary);
    let fallback_str = if config.models.fallbacks.is_empty() {
        "none".to_string()
    } else {
        config.models.fallbacks.join(", ")
    };
    step_detail(t, "fallbacks", &fallback_str);
    step_detail(t, "routing", &config.models.routing.mode);

    step(t, 8, STEPS, "Agent loop initialized");

    if config.skills.skills_dir.exists() {
        step(t, 9, STEPS, "Skills loaded");
        step_detail(t, "dir", &config.skills.skills_dir.display().to_string());
    } else {
        step_warn(
            t,
            9,
            STEPS,
            &format!(
                "Skills directory not found: {}",
                config.skills.skills_dir.display()
            ),
        );
    }

    let _heartbeat = ironclad_schedule::HeartbeatDaemon::new(60_000);
    step(t, 10, STEPS, "Scheduler initialized");
    step_detail(t, "heartbeat", "60s");

    let mut channels = vec!["web"];
    if config.channels.telegram.is_some() {
        channels.push("telegram");
    }
    if config.channels.whatsapp.is_some() {
        channels.push("whatsapp");
    }
    if config.channels.discord.is_some() {
        channels.push("discord");
    }
    if config.channels.signal.is_some() {
        channels.push("signal");
    }
    if config.a2a.enabled {
        channels.push("a2a");
    }
    step(t, 11, STEPS, "Channel adapters ready");
    step_detail(t, "active", &channels.join(", "));

    let bind_addr = format!("{}:{}", config.server.bind, config.server.port);

    step(t, 12, STEPS, "HTTP server starting");
    step_detail(t, "bind", &bind_addr);
    step_detail(t, "dashboard", &format!("http://{bind_addr}"));

    let shutdown_signal = async {
        let ctrl_c = tokio::signal::ctrl_c();

        #[cfg(unix)]
        {
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(mut sigterm) => {
                    tokio::select! {
                        _ = ctrl_c => info!("SIGINT received, shutting down gracefully"),
                        _ = sigterm.recv() => info!("SIGTERM received, shutting down gracefully"),
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "failed to install SIGTERM handler, falling back to SIGINT only");
                    // best-effort: signal wait result is irrelevant during shutdown
                    ctrl_c.await.ok();
                    info!("SIGINT received, shutting down gracefully");
                }
            }
        }
        #[cfg(not(unix))]
        {
            // best-effort: signal wait result is irrelevant during shutdown
            ctrl_c.await.ok();
            info!("SIGINT received, shutting down gracefully");
        }
    };

    let listener = match tokio::net::TcpListener::bind(&bind_addr).await {
        Ok(l) => l,
        Err(e) if e.kind() == std::io::ErrorKind::AddrInUse => {
            let (w, r) = (t.icon_warn(), t.reset());
            eprintln!("  {w} Port {bind_addr} in use, shutting down previous instance...{r}");

            #[cfg(unix)]
            {
                if let Ok(pids) = find_listeners(config.server.port) {
                    let own_pid = std::process::id();
                    for pid in pids.iter().filter(|&&p| p != own_pid) {
                        if let Ok(p) = i32::try_from(*pid) {
                            unsafe {
                                libc::kill(p, libc::SIGTERM);
                            }
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;

                    if let Ok(remaining) = find_listeners(config.server.port) {
                        for pid in remaining.iter().filter(|&&p| p != own_pid) {
                            if let Ok(p) = i32::try_from(*pid) {
                                unsafe {
                                    libc::kill(p, libc::SIGKILL);
                                }
                            }
                        }
                        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                    }
                }
            }

            tokio::net::TcpListener::bind(&bind_addr)
                .await
                .map_err(|e2| {
                    format!("port {bind_addr} still in use after killing previous instance: {e2}")
                })?
        }
        Err(e) => return Err(e.into()),
    };
    let elapsed = boot_start.elapsed();
    let (a, b, r) = (t.accent(), t.bold(), t.reset());
    eprintln!();
    let action_icon = t.icon_action();
    eprint!("  {action_icon} ");
    t.typewrite(&format!("{b}Ready{r} in {a}{:.0?}{r}", elapsed), 25);
    eprintln!();
    eprintln!();

    if std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        ironclad_server::enable_stderr_logging();
    }
    info!("Ironclad listening on http://{bind_addr}");

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal)
    .await?;

    info!("Server shut down");
    Ok(())
}

fn provider_requires_internal_proxy(
    name: &str,
    cfg: &ironclad_core::config::ProviderConfig,
) -> bool {
    if cfg.is_local.unwrap_or(false) {
        return false;
    }
    let lowered = name.to_ascii_lowercase();
    if lowered.contains("ollama") {
        return false;
    }
    let parsed = match reqwest::Url::parse(cfg.url.trim()) {
        Ok(u) => u,
        Err(_) => return false,
    };
    matches!(
        parsed
            .host_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "127.0.0.1" | "localhost" | "::1"
    )
}

fn tcp_endpoint_reachable(host: &str, port: u16) -> bool {
    let addr = format!("{host}:{port}");
    let resolved = match addr.to_socket_addrs() {
        Ok(mut addrs) => addrs.next(),
        Err(_) => None,
    };
    let Some(sock) = resolved else {
        return false;
    };
    std::net::TcpStream::connect_timeout(&sock, std::time::Duration::from_millis(400)).is_ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ProviderUrlMigration {
    provider: String,
    from_url: String,
    to_url: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LegacyLoopbackMode {
    MigrateDeprecated,
    Unsupported,
}

fn legacy_loopback_mode_for_version(version: &str) -> LegacyLoopbackMode {
    let mut it = version.split('.');
    let major = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let minor = it.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    if major > 0 || minor >= 8 {
        LegacyLoopbackMode::Unsupported
    } else {
        LegacyLoopbackMode::MigrateDeprecated
    }
}

fn legacy_loopback_mode() -> LegacyLoopbackMode {
    legacy_loopback_mode_for_version(env!("CARGO_PKG_VERSION"))
}

fn canonical_provider_base_url(provider_name: &str) -> Option<&'static str> {
    match provider_name.to_ascii_lowercase().as_str() {
        "anthropic" => Some("https://api.anthropic.com"),
        "google" => Some("https://generativelanguage.googleapis.com"),
        "openai" => Some("https://api.openai.com"),
        "openrouter" => Some("https://openrouter.ai/api"),
        "moonshot" => Some("https://api.moonshot.ai"),
        _ => None,
    }
}

fn parse_legacy_proxy_url(provider_name: &str, url: &str) -> bool {
    let parsed = match reqwest::Url::parse(url.trim()) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let host = parsed.host_str().unwrap_or_default().to_ascii_lowercase();
    if !matches!(host.as_str(), "127.0.0.1" | "localhost" | "::1") {
        return false;
    }
    if parsed.port_or_known_default().unwrap_or(80) != 8788 {
        return false;
    }
    let mut segs = match parsed.path_segments() {
        Some(v) => v,
        None => return false,
    };
    let Some(first) = segs.next() else {
        return false;
    };
    first.eq_ignore_ascii_case(provider_name)
}

fn rewrite_provider_urls_in_toml(
    original: &str,
    migrations: &[ProviderUrlMigration],
) -> (String, bool) {
    let mut migration_map = std::collections::HashMap::<String, String>::new();
    for m in migrations {
        migration_map.insert(m.provider.to_ascii_lowercase(), m.to_url.clone());
    }
    if migration_map.is_empty() {
        return (original.to_string(), false);
    }

    let mut current_provider: Option<String> = None;
    let mut changed = false;
    let mut out = Vec::<String>::new();
    for line in original.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = &trimmed[1..trimmed.len() - 1];
            if let Some(rest) = section.strip_prefix("providers.") {
                if !rest.contains('.') && !rest.is_empty() {
                    current_provider = Some(rest.to_ascii_lowercase());
                } else {
                    current_provider = None;
                }
            } else {
                current_provider = None;
            }
            out.push(line.to_string());
            continue;
        }

        if let Some(provider) = current_provider.as_deref()
            && trimmed.starts_with("url")
            && trimmed.contains('=')
            && let Some(new_url) = migration_map.get(provider)
        {
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            out.push(format!("{indent}url = \"{new_url}\""));
            changed = true;
            continue;
        }

        out.push(line.to_string());
    }

    let mut rewritten = out.join("\n");
    if original.ends_with('\n') {
        rewritten.push('\n');
    }
    (rewritten, changed)
}

fn persist_provider_url_migrations(
    config_path: &Path,
    migrations: &[ProviderUrlMigration],
) -> Result<(), Box<dyn std::error::Error>> {
    if migrations.is_empty() || !config_path.exists() {
        return Ok(());
    }

    let original = std::fs::read_to_string(config_path)?;
    let (rewritten, changed) = rewrite_provider_urls_in_toml(&original, migrations);
    if !changed {
        return Ok(());
    }

    let backup = config_path.with_extension("toml.bak");
    if !backup.exists() {
        std::fs::copy(config_path, &backup)?;
    }

    let tmp = config_path.with_extension("toml.tmp");
    std::fs::write(&tmp, rewritten)?;
    std::fs::rename(&tmp, config_path)?;
    Ok(())
}

fn migrate_legacy_proxy_urls(
    config: &mut IroncladConfig,
    config_path: Option<&Path>,
) -> Result<Vec<ProviderUrlMigration>, Box<dyn std::error::Error>> {
    let mut migrations = Vec::new();
    for (name, provider) in &mut config.providers {
        if provider.is_local.unwrap_or(false) {
            continue;
        }
        if !parse_legacy_proxy_url(name, &provider.url) {
            continue;
        }
        let Some(canonical) = canonical_provider_base_url(name) else {
            continue;
        };
        if provider.url.trim().eq_ignore_ascii_case(canonical) {
            continue;
        }
        let from = provider.url.clone();
        provider.url = canonical.to_string();
        migrations.push(ProviderUrlMigration {
            provider: name.clone(),
            from_url: from,
            to_url: canonical.to_string(),
        });
    }

    if let Some(path) = config_path {
        persist_provider_url_migrations(path, &migrations)?;
    }

    Ok(migrations)
}

fn collect_legacy_loopback_providers(config: &IroncladConfig) -> Vec<String> {
    let mut providers = Vec::new();
    for (name, provider) in &config.providers {
        if provider.is_local.unwrap_or(false) {
            continue;
        }
        if parse_legacy_proxy_url(name, &provider.url) {
            providers.push(format!("providers.{name}.url={}", provider.url));
        }
    }
    providers
}

fn validate_legacy_loopback_urls_for_mode(
    config: &IroncladConfig,
    mode: LegacyLoopbackMode,
) -> Result<(), String> {
    if mode != LegacyLoopbackMode::Unsupported {
        return Ok(());
    }
    let legacy = collect_legacy_loopback_providers(config);
    if legacy.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "unsupported legacy provider URLs detected (replace with direct provider bases): {}",
            legacy.join(", ")
        ))
    }
}

/// Check reachability of locally-configured provider proxies and return any that
/// are currently unreachable. This is purely informational — provider availability
/// is a runtime concern handled by the circuit breaker + fallback chain, not a
/// startup precondition.
fn check_internal_proxy_reachability(config: &IroncladConfig) -> Vec<String> {
    let mut candidates = Vec::<(String, String, u16)>::new();
    for (name, provider) in &config.providers {
        if !provider_requires_internal_proxy(name, provider) {
            continue;
        }
        let Ok(parsed) = reqwest::Url::parse(provider.url.trim()) else {
            continue;
        };
        let Some(host) = parsed.host_str() else {
            continue;
        };
        let port = parsed.port_or_known_default().unwrap_or(80);
        candidates.push((name.clone(), host.to_string(), port));
    }

    let mut unreachable = Vec::new();
    for (name, host, port) in candidates {
        if !tcp_endpoint_reachable(&host, port) {
            unreachable.push(format!("{name} ({host}:{port})"));
        }
    }
    unreachable
}

fn cmd_init(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    print_banner(t);
    let dir = std::path::Path::new(path);
    let (b, r) = (t.bold(), t.reset());
    let (ok, action, warn) = (t.icon_ok(), t.icon_action(), t.icon_warn());

    t.typewrite_line(
        &format!(
            "  {b}Initializing Ironclad workspace{r} at {}\n",
            dir.display()
        ),
        4,
    );

    let config_path = dir.join("ironclad.toml");
    if config_path.exists() {
        t.typewrite_line(
            &format!("  {warn} ironclad.toml already exists, skipping"),
            4,
        );
    } else {
        std::fs::write(&config_path, INIT_CONFIG)?;
        t.typewrite_line(&format!("  {action} Created ironclad.toml"), 4);
    }

    let skills_dir = dir.join("skills");
    if skills_dir.exists() {
        t.typewrite_line(
            &format!("  {warn} skills/ directory already exists, skipping"),
            4,
        );
    } else {
        std::fs::create_dir_all(&skills_dir)?;
        let count = cli::write_starter_skills(&skills_dir)?;
        t.typewrite_line(
            &format!("  {action} Created skills/ with {count} starter skills"),
            4,
        );
    }

    eprintln!();
    t.typewrite_line(
        &format!("  {ok} Done. Run {b}ironclad serve -c ironclad.toml{r} to start."),
        4,
    );
    eprintln!();

    Ok(())
}

fn cmd_check(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    print_banner(t);
    let (s, b, r) = (t.success(), t.bold(), t.reset());
    let (ok, warn) = (t.icon_ok(), t.icon_warn());
    let tw = |text: &str| t.typewrite_line(text, 4);

    tw(&format!("  {b}Validating{r} {config_path}\n"));

    let config = match IroncladConfig::from_file(Path::new(config_path)) {
        Ok(c) => c,
        Err(e) => {
            let msg = format!("{e}");
            if msg.contains("No such file") || msg.contains("not found") || msg.contains("NotFound")
            {
                eprintln!("  {warn} Config file not found: {config_path}");
                eprintln!(
                    "    Specify a path with {b}--config <path>{r} or create one with {b}ironclad init{r}"
                );
                eprintln!();
            }
            return Err(Box::new(e));
        }
    };
    tw(&format!("  {ok} TOML syntax valid"));

    config.validate()?;
    tw(&format!("  {ok} Configuration semantics valid"));

    tw(&format!(
        "  {ok} Agent: {} ({})",
        config.agent.name, config.agent.id
    ));
    tw(&format!(
        "  {ok} Server: {}:{}",
        config.server.bind, config.server.port
    ));
    tw(&format!("  {ok} Primary model: {}", config.models.primary));
    tw(&format!(
        "  {ok} Database: {}",
        config.database.path.display()
    ));

    let mem_sum = config.memory.working_budget_pct
        + config.memory.episodic_budget_pct
        + config.memory.semantic_budget_pct
        + config.memory.procedural_budget_pct
        + config.memory.relationship_budget_pct;
    tw(&format!("  {ok} Memory budgets sum to {mem_sum}%"));

    tw(&format!(
        "  {ok} Treasury: cap=${:.2}/payment, reserve=${:.2}",
        config.treasury.per_payment_cap, config.treasury.minimum_reserve
    ));

    if config.skills.skills_dir.exists() {
        tw(&format!(
            "  {ok} Skills dir exists: {}",
            config.skills.skills_dir.display()
        ));
    } else {
        tw(&format!(
            "  {warn} Skills dir missing: {}",
            config.skills.skills_dir.display()
        ));
    }

    if config.a2a.enabled {
        tw(&format!(
            "  {ok} A2A enabled (rate limit: {}/peer)",
            config.a2a.rate_limit_per_peer
        ));
    }

    // ── Security (RBAC) ──────────────────────────────────────
    tw(&format!(
        "  {ok} Security: deny_on_empty_allowlist={}, allowlist={:?}, trusted={:?}, api={:?}, threat_ceiling={:?}",
        config.security.deny_on_empty_allowlist,
        config.security.allowlist_authority,
        config.security.trusted_authority,
        config.security.api_authority,
        config.security.threat_caution_ceiling,
    ));
    tw(&format!(
        "  {ok} Trusted senders: {} configured",
        config.channels.trusted_sender_ids.len()
    ));

    // Per-channel allow-list warnings
    {
        let mut any_channel_warn = false;
        if let Some(ref tg) = config.channels.telegram {
            if tg.allowed_chat_ids.is_empty() && config.security.deny_on_empty_allowlist {
                tw(&format!(
                    "  {warn} Telegram: allowed_chat_ids is empty (all messages will be rejected)"
                ));
                tw("         Hint: find your chat ID by messaging @userinfobot on Telegram");
                any_channel_warn = true;
            } else if !tg.allowed_chat_ids.is_empty() {
                tw(&format!(
                    "  {ok} Telegram: {} chat ID(s) configured",
                    tg.allowed_chat_ids.len()
                ));
            }
        }
        if let Some(ref dc) = config.channels.discord {
            if dc.allowed_guild_ids.is_empty() && config.security.deny_on_empty_allowlist {
                tw(&format!(
                    "  {warn} Discord: allowed_guild_ids is empty (all messages will be rejected)"
                ));
                any_channel_warn = true;
            } else if !dc.allowed_guild_ids.is_empty() {
                tw(&format!(
                    "  {ok} Discord: {} guild ID(s) configured",
                    dc.allowed_guild_ids.len()
                ));
            }
        }
        if let Some(ref wa) = config.channels.whatsapp {
            if wa.allowed_numbers.is_empty() && config.security.deny_on_empty_allowlist {
                tw(&format!(
                    "  {warn} WhatsApp: allowed_numbers is empty (all messages will be rejected)"
                ));
                any_channel_warn = true;
            } else if !wa.allowed_numbers.is_empty() {
                tw(&format!(
                    "  {ok} WhatsApp: {} number(s) configured",
                    wa.allowed_numbers.len()
                ));
            }
        }
        if let Some(ref sig) = config.channels.signal {
            if sig.allowed_numbers.is_empty() && config.security.deny_on_empty_allowlist {
                tw(&format!(
                    "  {warn} Signal: allowed_numbers is empty (all messages will be rejected)"
                ));
                any_channel_warn = true;
            } else if !sig.allowed_numbers.is_empty() {
                tw(&format!(
                    "  {ok} Signal: {} number(s) configured",
                    sig.allowed_numbers.len()
                ));
            }
        }
        if config.channels.email.enabled {
            if config.channels.email.allowed_senders.is_empty()
                && config.security.deny_on_empty_allowlist
            {
                tw(&format!(
                    "  {warn} Email: allowed_senders is empty (all messages will be rejected)"
                ));
                any_channel_warn = true;
            } else if !config.channels.email.allowed_senders.is_empty() {
                tw(&format!(
                    "  {ok} Email: {} sender(s) configured",
                    config.channels.email.allowed_senders.len()
                ));
            }
        }
        if config.channels.trusted_sender_ids.is_empty() && !any_channel_warn {
            tw(&format!(
                "  {warn} No trusted senders — no user can reach Creator authority"
            ));
        }
    }

    eprintln!();
    tw(&format!("  {ok} {s}All checks passed.{r}"));
    eprintln!();

    Ok(())
}

fn cmd_version(json: bool) {
    if json {
        let out = serde_json::json!({
            "version": env!("CARGO_PKG_VERSION"),
            "edition": "Rust 2024",
            "target": std::env::consts::ARCH,
            "os": std::env::consts::OS,
        });
        println!("{}", serde_json::to_string_pretty(&out).unwrap_or_default());
        return;
    }
    let t = cli::theme();
    print_banner(t);
    let tw = |text: &str| t.typewrite_line(text, 4);
    tw(&format!("  version:    {}", env!("CARGO_PKG_VERSION")));
    tw("  edition:    Rust 2024");
    tw(&format!("  target:     {}", std::env::consts::ARCH));
    tw(&format!("  os:         {}", std::env::consts::OS));
    eprintln!();
}

fn cmd_ingest(
    path: &str,
    json: bool,
    config_path: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use ironclad_agent::ingest::{ingest_directory, ingest_file};

    let cfg = match resolve_config_path(config_path) {
        Some(p) => IroncladConfig::from_file(&p)?,
        None => IroncladConfig::from_str(FALLBACK_CONFIG)?,
    };

    let db_path = cfg.database.path.to_string_lossy();
    let db = ironclad_db::Database::new(&db_path)?;

    let target = std::path::Path::new(path);

    let results = if target.is_dir() {
        ingest_directory(&db, target)?
    } else if target.is_file() {
        vec![ingest_file(&db, target)?]
    } else {
        return Err(format!("{path} does not exist or is not accessible").into());
    };

    if json {
        let out = serde_json::to_string_pretty(&results)?;
        std::io::Write::write_all(&mut std::io::stdout(), out.as_bytes())?;
        std::io::Write::write_all(&mut std::io::stdout(), b"\n")?;
    } else {
        if results.is_empty() {
            eprintln!("No supported files found.");
            return Ok(());
        }
        for r in &results {
            eprintln!(
                "  ✓ {} — {} ({} chunks, {} chars)",
                r.file_path,
                r.file_type.label(),
                r.chunks_stored,
                r.total_chars
            );
        }
        let total_chunks: usize = results.iter().map(|r| r.chunks_stored).sum();
        eprintln!(
            "\nIngested {} file(s), {} total chunks.",
            results.len(),
            total_chunks
        );
    }

    Ok(())
}

fn cmd_web(config_path: Option<&str>, cli_url: &str) -> Result<(), Box<dyn std::error::Error>> {
    let url = if let Some(path) = config_path {
        let raw = std::fs::read_to_string(path)?;
        let cfg: ironclad_core::config::IroncladConfig = toml::from_str(&raw)?;
        let host = if cfg.server.bind == "0.0.0.0" {
            "127.0.0.1"
        } else {
            &cfg.server.bind
        };
        format!("http://{}:{}", host, cfg.server.port)
    } else {
        cli_url.to_string()
    };
    eprintln!("  Opening {url}");
    open::that(&url)?;
    Ok(())
}

/// Find PIDs of processes listening on the given port using `lsof`.
fn find_listeners(port: u16) -> Result<Vec<u32>, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("lsof")
        .args(["-ti", &format!(":{port}")])
        .output()?;

    let pids: Vec<u32> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.trim().parse::<u32>().ok())
        .collect();

    Ok(pids)
}

const FALLBACK_CONFIG: &str = r#"
[agent]
name = "Ironclad"
id = "ironclad-dev"

[server]
port = 18789
bind = "127.0.0.1"

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
"#;

const INIT_CONFIG: &str = r#"# Ironclad Configuration
# See: https://roboticus.ai/docs/configuration

[agent]
name = "Roboticus"
id = "roboticus"
workspace = "~/.ironclad/workspace"
log_level = "info"

[server]
port = 18789
bind = "127.0.0.1"

[database]
path = "~/.ironclad/state.db"

[models]
primary = "ollama/qwen3:8b"
fallbacks = []

[models.routing]
mode = "metascore"
confidence_threshold = 0.9
local_first = true

[memory]
working_budget_pct = 30.0
episodic_budget_pct = 25.0
semantic_budget_pct = 20.0
procedural_budget_pct = 15.0
relationship_budget_pct = 10.0

[cache]
enabled = true
exact_match_ttl_seconds = 3600
semantic_threshold = 0.95
max_entries = 10000

[treasury]
per_payment_cap = 100.0
hourly_transfer_limit = 500.0
daily_transfer_limit = 2000.0
minimum_reserve = 5.0
daily_inference_budget = 50.0

[skills]
skills_dir = "~/.ironclad/skills"
script_timeout_seconds = 30
script_max_output_bytes = 1048576
allowed_interpreters = ["bash", "python3", "node"]
sandbox_env = true
hot_reload = true

[a2a]
enabled = true
max_message_size = 65536
rate_limit_per_peer = 10
session_timeout_seconds = 3600
require_on_chain_identity = true
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn minimal_cfg_with_providers(providers_block: &str) -> IroncladConfig {
        let cfg = format!(
            r#"
[agent]
name = "T"
id = "t"
[server]
bind = "127.0.0.1"
port = 18789
[database]
path = ":memory:"
[models]
primary = "moonshot/kimi-k2-turbo-preview"
{providers}
"#,
            providers = providers_block
        );
        IroncladConfig::from_str(&cfg).expect("config parses")
    }

    #[test]
    fn provider_requires_internal_proxy_true_for_non_local_loopback() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#,
        );
        let p = cfg.providers.get("anthropic").unwrap();
        assert!(provider_requires_internal_proxy("anthropic", p));
    }

    #[test]
    fn provider_requires_internal_proxy_false_for_ollama_and_local() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.ollama]
url = "http://127.0.0.1:11434"
tier = "T1"
is_local = true
"#,
        );
        let p = cfg.providers.get("ollama").unwrap();
        assert!(!provider_requires_internal_proxy("ollama", p));
    }

    #[test]
    fn tcp_endpoint_reachable_detects_open_listener() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        assert!(tcp_endpoint_reachable("127.0.0.1", port));
    }

    #[test]
    fn tcp_endpoint_reachable_detects_closed_port() {
        // Use TEST-NET-1 (RFC 5737) — a non-routable address that guarantees connection
        // failure without the TOCTOU race of binding+dropping an ephemeral port.
        assert!(!tcp_endpoint_reachable("192.0.2.1", 1));
    }

    #[test]
    fn parse_legacy_proxy_url_requires_loopback_8788_and_provider_prefix() {
        assert!(parse_legacy_proxy_url(
            "anthropic",
            "http://127.0.0.1:8788/anthropic"
        ));
        assert!(!parse_legacy_proxy_url(
            "anthropic",
            "http://127.0.0.1:8789/anthropic"
        ));
        assert!(!parse_legacy_proxy_url(
            "anthropic",
            "https://api.anthropic.com"
        ));
    }

    #[test]
    fn rewrite_provider_urls_in_toml_updates_only_targeted_provider_blocks() {
        let source = r#"[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"

[providers.google]
url = "http://127.0.0.1:8788/google"
tier = "T2"
"#;
        let migrations = vec![ProviderUrlMigration {
            provider: "anthropic".into(),
            from_url: "http://127.0.0.1:8788/anthropic".into(),
            to_url: "https://api.anthropic.com".into(),
        }];
        let (rewritten, changed) = rewrite_provider_urls_in_toml(source, &migrations);
        assert!(changed);
        assert!(rewritten.contains("url = \"https://api.anthropic.com\""));
        assert!(rewritten.contains("url = \"http://127.0.0.1:8788/google\""));
    }

    #[test]
    fn migrate_legacy_proxy_urls_rewrites_config_and_persists_file() {
        let cfg = r#"
[agent]
name = "T"
id = "t"
[server]
bind = "127.0.0.1"
port = 18789
[database]
path = ":memory:"
[models]
primary = "anthropic/x"
[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ironclad.toml");
        std::fs::write(&path, cfg).unwrap();
        let mut parsed = IroncladConfig::from_str(cfg).unwrap();
        let migrations = migrate_legacy_proxy_urls(&mut parsed, Some(&path)).unwrap();
        assert_eq!(migrations.len(), 1);
        assert_eq!(
            parsed.providers.get("anthropic").unwrap().url,
            "https://api.anthropic.com"
        );
        let persisted = std::fs::read_to_string(&path).unwrap();
        assert!(persisted.contains("url = \"https://api.anthropic.com\""));
    }

    #[test]
    fn legacy_loopback_mode_for_version_changes_at_0_8() {
        assert_eq!(
            legacy_loopback_mode_for_version("0.7.1"),
            LegacyLoopbackMode::MigrateDeprecated
        );
        assert_eq!(
            legacy_loopback_mode_for_version("0.8.0"),
            LegacyLoopbackMode::Unsupported
        );
    }

    #[test]
    fn collect_legacy_loopback_providers_finds_legacy_urls() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"

[providers.google]
url = "https://generativelanguage.googleapis.com"
tier = "T2"
"#,
        );
        let legacy = collect_legacy_loopback_providers(&cfg);
        assert_eq!(legacy.len(), 1);
        assert!(legacy[0].contains("providers.anthropic.url"));
    }

    #[test]
    fn validate_legacy_loopback_urls_for_mode_rejects_in_0_8_mode() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#,
        );
        let err = validate_legacy_loopback_urls_for_mode(&cfg, LegacyLoopbackMode::Unsupported)
            .expect_err("v0.8 mode must reject legacy loopback");
        assert!(err.contains("providers.anthropic.url"));
    }

    #[test]
    fn canonical_provider_base_url_is_case_insensitive() {
        assert_eq!(
            canonical_provider_base_url("AnThRoPiC"),
            Some("https://api.anthropic.com")
        );
        assert_eq!(
            canonical_provider_base_url("GOOGLE"),
            Some("https://generativelanguage.googleapis.com")
        );
        assert_eq!(canonical_provider_base_url("unknown"), None);
    }

    #[test]
    fn parse_legacy_proxy_url_rejects_wrong_path_prefix() {
        assert!(!parse_legacy_proxy_url(
            "anthropic",
            "http://127.0.0.1:8788/google"
        ));
    }

    #[test]
    fn rewrite_provider_urls_in_toml_noop_without_migrations() {
        let source = r#"[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#;
        let (rewritten, changed) = rewrite_provider_urls_in_toml(source, &[]);
        assert!(!changed);
        assert_eq!(rewritten, source);
    }

    #[test]
    fn persist_provider_url_migrations_writes_backup_and_new_url() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().join("ironclad.toml");
        std::fs::write(
            &config_path,
            r#"[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#,
        )
        .unwrap();
        let migrations = vec![ProviderUrlMigration {
            provider: "anthropic".into(),
            from_url: "http://127.0.0.1:8788/anthropic".into(),
            to_url: "https://api.anthropic.com".into(),
        }];
        persist_provider_url_migrations(&config_path, &migrations).unwrap();
        let updated = std::fs::read_to_string(&config_path).unwrap();
        assert!(updated.contains("https://api.anthropic.com"));
        assert!(config_path.with_extension("toml.bak").exists());
    }

    #[test]
    fn check_internal_proxy_reachability_skips_non_loopback_providers() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "https://api.anthropic.com"
tier = "T3"
"#,
        );
        let unreachable = check_internal_proxy_reachability(&cfg);
        assert!(unreachable.is_empty());
    }

    #[test]
    fn cmd_init_then_cmd_check_succeeds_for_generated_workspace() {
        let dir = tempfile::tempdir().unwrap();
        cmd_init(dir.path().to_str().unwrap()).expect("init should succeed");
        let cfg_path = dir.path().join("ironclad.toml");
        assert!(cfg_path.exists());
        cmd_check(cfg_path.to_str().unwrap()).expect("check should succeed");
    }

    #[test]
    fn provider_requires_internal_proxy_false_for_invalid_url() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "not-a-url"
tier = "T3"
"#,
        );
        let p = cfg.providers.get("anthropic").unwrap();
        assert!(!provider_requires_internal_proxy("anthropic", p));
    }

    #[test]
    fn parse_legacy_proxy_url_rejects_non_loopback_hosts() {
        assert!(!parse_legacy_proxy_url(
            "anthropic",
            "http://10.0.0.1:8788/anthropic"
        ));
    }

    #[test]
    fn persist_provider_url_migrations_is_noop_for_missing_config() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing.toml");
        let migrations = vec![ProviderUrlMigration {
            provider: "anthropic".into(),
            from_url: "http://127.0.0.1:8788/anthropic".into(),
            to_url: "https://api.anthropic.com".into(),
        }];
        persist_provider_url_migrations(&missing, &migrations).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn collect_legacy_loopback_providers_ignores_local_flagged_provider() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.ollama]
url = "http://127.0.0.1:8788/ollama"
tier = "T1"
is_local = true
"#,
        );
        let legacy = collect_legacy_loopback_providers(&cfg);
        assert!(legacy.is_empty());
    }

    #[test]
    fn validate_legacy_loopback_urls_for_mode_allows_pre_0_8_mode() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
"#,
        );
        let result =
            validate_legacy_loopback_urls_for_mode(&cfg, LegacyLoopbackMode::MigrateDeprecated);
        assert!(result.is_ok());
    }

    #[test]
    fn check_internal_proxy_reachability_reports_unreachable_loopback_proxy() {
        let cfg = minimal_cfg_with_providers(
            r#"
[providers.anthropic]
url = "http://127.0.0.1:9/anthropic"
tier = "T3"
"#,
        );
        let unreachable = check_internal_proxy_reachability(&cfg);
        assert!(!unreachable.is_empty(), "should report unreachable proxy");
        assert!(unreachable[0].contains("anthropic"));
    }

    #[test]
    fn migrate_legacy_proxy_urls_noop_for_already_canonical_urls() {
        let cfg_text = r#"
[agent]
name = "T"
id = "t"
[server]
bind = "127.0.0.1"
port = 18789
[database]
path = ":memory:"
[models]
primary = "anthropic/x"
[providers.anthropic]
url = "https://api.anthropic.com"
tier = "T3"
"#;
        let mut cfg = IroncladConfig::from_str(cfg_text).unwrap();
        let migrations = migrate_legacy_proxy_urls(&mut cfg, None).unwrap();
        assert!(migrations.is_empty());
        assert_eq!(
            cfg.providers.get("anthropic").unwrap().url,
            "https://api.anthropic.com"
        );
    }

    #[test]
    fn rewrite_provider_urls_in_toml_only_changes_top_level_provider_section_url_field() {
        let source = r#"[providers.anthropic]
url = "http://127.0.0.1:8788/anthropic"
tier = "T3"
[providers.anthropic.extra]
url = "http://127.0.0.1:8788/should-not-change"
"#;
        let migrations = vec![ProviderUrlMigration {
            provider: "anthropic".into(),
            from_url: "http://127.0.0.1:8788/anthropic".into(),
            to_url: "https://api.anthropic.com".into(),
        }];
        let (rewritten, changed) = rewrite_provider_urls_in_toml(source, &migrations);
        assert!(changed);
        assert!(rewritten.contains("url = \"https://api.anthropic.com\""));
        assert!(rewritten.contains("url = \"http://127.0.0.1:8788/should-not-change\""));
    }

    #[test]
    fn legacy_loopback_mode_matches_current_package_version_rule() {
        let mode = legacy_loopback_mode();
        // Current package version in this workspace is 0.8+.
        assert_eq!(mode, LegacyLoopbackMode::Unsupported);
    }

    #[test]
    fn canonical_provider_base_url_covers_known_providers() {
        assert_eq!(
            canonical_provider_base_url("openrouter"),
            Some("https://openrouter.ai/api")
        );
        assert_eq!(
            canonical_provider_base_url("moonshot"),
            Some("https://api.moonshot.ai")
        );
        assert_eq!(canonical_provider_base_url("unknown-provider"), None);
    }

    #[test]
    fn cmd_init_is_idempotent_when_config_exists() {
        let dir = tempfile::tempdir().unwrap();
        let cfg_path = dir.path().join("ironclad.toml");
        std::fs::write(&cfg_path, "sentinel").unwrap();
        cmd_init(dir.path().to_str().unwrap()).unwrap();
        let after = std::fs::read_to_string(&cfg_path).unwrap();
        assert_eq!(after, "sentinel");
    }

    #[test]
    fn find_listeners_returns_empty_for_closed_ephemeral_port() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let pids = find_listeners(port).unwrap();
        assert!(pids.is_empty());
    }
}
