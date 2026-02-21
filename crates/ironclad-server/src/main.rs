use std::path::Path;
use std::time::Instant;

use clap::{Parser, Subcommand};
use tracing::info;

use ironclad_core::IroncladConfig;
use ironclad_core::style::Theme;
use ironclad_server::{bootstrap, cli};

#[derive(Parser)]
#[command(name = "ironclad", version, about = "Ironclad Autonomous Agent Runtime")]
struct Cli {
    /// Server URL for management commands
    #[arg(long, global = true, default_value = "http://127.0.0.1:18789", env = "IRONCLAD_URL")]
    url: String,

    /// Profile name for state isolation
    #[arg(long, global = true, env = "IRONCLAD_PROFILE")]
    profile: Option<String>,

    /// Color output: auto (default), always, never
    #[arg(long, global = true, default_value = "auto")]
    color: String,

    /// Disable CRT typewriter draw effect
    #[arg(long, global = true)]
    no_draw: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Ironclad server
    Serve {
        /// Path to config file (TOML)
        #[arg(short, long)]
        config: Option<String>,
        /// Override bind port
        #[arg(short, long)]
        port: Option<u16>,
        /// Override bind address
        #[arg(short, long)]
        bind: Option<String>,
    },
    /// Initialize a new Ironclad workspace
    Init {
        /// Directory to initialize
        #[arg(default_value = ".")]
        path: String,
    },
    /// Interactive setup wizard
    Onboard,
    /// Validate configuration
    Check {
        /// Path to config file
        #[arg(short, long, default_value = "ironclad.toml")]
        config: String,
    },
    /// Print version and build info
    Version,

    // ── Management commands (talk to running server) ──

    /// Show agent status overview
    Status,

    /// Manage sessions
    #[command(subcommand)]
    Sessions(SessionsCmd),

    /// Discover and manage models
    #[command(subcommand)]
    Models(ModelsCmd),

    /// Browse and search memory
    Memory {
        /// Memory tier: working, episodic, semantic, search
        tier: String,
        /// Session ID (required for working memory) or category (for semantic)
        #[arg(short, long)]
        session: Option<String>,
        /// Search query (for search tier)
        #[arg(short, long)]
        query: Option<String>,
        /// Limit results
        #[arg(short, long)]
        limit: Option<i64>,
    },

    /// Manage skills
    #[command(subcommand)]
    Skills(SkillsCmd),

    /// View and manage cron jobs
    Cron,

    /// View metrics and costs
    Metrics {
        /// Metric kind: costs, transactions, cache
        #[arg(default_value = "costs")]
        kind: String,
        /// Time window in hours (for transactions)
        #[arg(short = 'H', long)]
        hours: Option<i64>,
    },

    /// Show wallet info
    Wallet,

    /// Configuration management
    #[command(subcommand)]
    Config(ConfigCmd),

    /// Show circuit breaker status
    Breaker,

    /// Show channel status
    Channels,

    /// View and tail logs
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

    /// Manage plugins
    #[command(subcommand)]
    Plugins(PluginsCmd),

    /// Manage agents
    #[command(subcommand)]
    Agents(AgentsCmd),

    /// Run health checks
    Mechanic {
        /// Attempt to auto-repair issues
        #[arg(long, short = 'r', alias = "rep")]
        repair: bool,
    },

    /// Manage daemon service
    #[command(subcommand)]
    Daemon(DaemonCmd),

    /// Uninstall Ironclad daemon and optionally remove data
    Uninstall {
        /// Also remove ~/.ironclad/ data directory
        #[arg(long)]
        purge: bool,
    },

    /// Reset configuration and state to defaults
    Reset {
        /// Skip confirmation prompt
        #[arg(long)]
        yes: bool,
    },

    /// Check for and install updates
    Update {
        /// Update channel: stable, beta, dev
        #[arg(long, default_value = "stable")]
        channel: String,
        /// Skip confirmation
        #[arg(long)]
        yes: bool,
        /// Don't restart daemon after update
        #[arg(long)]
        no_restart: bool,
    },

    /// Security audit and checks
    #[command(subcommand)]
    Security(SecurityCmd),

    /// Generate shell completions
    Completion {
        /// Shell: bash, zsh, fish
        shell: String,
    },
}

#[derive(Subcommand)]
enum SessionsCmd {
    /// List all sessions
    List,
    /// Show session details and messages
    Show {
        /// Session ID
        id: String,
    },
    /// Create a new session
    Create {
        /// Agent ID for the new session
        agent_id: String,
    },
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
    /// Install daemon service (LaunchAgent/systemd)
    Install {
        /// Path to config file
        #[arg(short, long, default_value = "ironclad.toml")]
        config: String,
    },
    /// Show daemon status
    Status,
    /// Uninstall daemon service
    Uninstall,
}

#[derive(Subcommand)]
enum ConfigCmd {
    /// Show running configuration (from server)
    Show,
    /// Get a config value by TOML path
    Get {
        /// TOML path (e.g., models.primary)
        path: String,
    },
    /// Set a config value
    Set {
        /// TOML path (e.g., models.primary)
        path: String,
        /// New value
        value: String,
        /// Config file to modify
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
    },
    /// Remove a config key
    Unset {
        /// TOML path to remove
        path: String,
        /// Config file to modify
        #[arg(short, long, default_value = "ironclad.toml")]
        file: String,
    },
}

#[derive(Subcommand)]
enum SkillsCmd {
    /// List all registered skills
    List,
    /// Show skill details
    Show {
        /// Skill ID
        id: String,
    },
    /// Reload skills from disk
    Reload,
}

#[derive(Subcommand)]
enum PluginsCmd {
    /// List installed plugins
    List,
    /// Show plugin details
    Info {
        /// Plugin name
        name: String,
    },
    /// Install a plugin from a directory
    Install {
        /// Path to plugin directory
        source: String,
    },
    /// Uninstall a plugin
    Uninstall {
        /// Plugin name
        name: String,
    },
    /// Enable a disabled plugin
    Enable {
        /// Plugin name
        name: String,
    },
    /// Disable a plugin
    Disable {
        /// Plugin name
        name: String,
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let parsed = Cli::parse();
    cli::init_theme(&parsed.color, parsed.no_draw);
    let t = cli::theme();
    eprint!("{}", t.reset());
    let url = &parsed.url;

    let result = match parsed.command {
        Some(Commands::Serve { config, port, bind }) => cmd_serve(config, port, bind).await,
        Some(Commands::Init { path }) => cmd_init(&path),
        Some(Commands::Onboard) => cli::cmd_onboard(),
        Some(Commands::Check { config }) => cmd_check(&config),
        Some(Commands::Version) => {
            cmd_version();
            Ok(())
        }
        Some(Commands::Status) => cli::cmd_status(url).await,
        Some(Commands::Sessions(sub)) => match sub {
            SessionsCmd::List => cli::cmd_sessions_list(url).await,
            SessionsCmd::Show { id } => cli::cmd_session_detail(url, &id).await,
            SessionsCmd::Create { agent_id } => cli::cmd_session_create(url, &agent_id).await,
            SessionsCmd::Export { id, format, output } => cli::cmd_session_export(url, &id, &format, output.as_deref()).await,
        },
        Some(Commands::Models(sub)) => match sub {
            ModelsCmd::List => cli::cmd_models_list(url).await,
            ModelsCmd::Scan { provider } => cli::cmd_models_scan(url, provider.as_deref()).await,
        },
        Some(Commands::Memory {
            tier,
            session,
            query,
            limit,
        }) => cli::cmd_memory(url, &tier, session.as_deref(), query.as_deref(), limit).await,
        Some(Commands::Skills(sub)) => match sub {
            SkillsCmd::List => cli::cmd_skills_list(url).await,
            SkillsCmd::Show { id } => cli::cmd_skill_detail(url, &id).await,
            SkillsCmd::Reload => cli::cmd_skills_reload(url).await,
        },
        Some(Commands::Cron) => cli::cmd_cron_list(url).await,
        Some(Commands::Metrics { kind, hours }) => cli::cmd_metrics(url, &kind, hours).await,
        Some(Commands::Wallet) => cli::cmd_wallet(url).await,
        Some(Commands::Config(sub)) => match sub {
            ConfigCmd::Show => cli::cmd_config(url).await,
            ConfigCmd::Get { path } => cli::cmd_config_get(&path),
            ConfigCmd::Set { path, value, file } => cli::cmd_config_set(&path, &value, &file),
            ConfigCmd::Unset { path, file } => cli::cmd_config_unset(&path, &file),
        },
        Some(Commands::Breaker) => cli::cmd_breaker(url).await,
        Some(Commands::Channels) => cli::cmd_channels_status(url).await,
        Some(Commands::Logs { lines, follow, level }) => cli::cmd_logs(url, lines, follow, &level).await,
        Some(Commands::Plugins(sub)) => match sub {
            PluginsCmd::List => cli::cmd_plugins_list(url).await,
            PluginsCmd::Info { name } => cli::cmd_plugin_info(url, &name).await,
            PluginsCmd::Install { source } => cli::cmd_plugin_install(&source),
            PluginsCmd::Uninstall { name } => cli::cmd_plugin_uninstall(&name),
            PluginsCmd::Enable { name } => cli::cmd_plugin_toggle(url, &name, true).await,
            PluginsCmd::Disable { name } => cli::cmd_plugin_toggle(url, &name, false).await,
        },
        Some(Commands::Agents(sub)) => match sub {
            AgentsCmd::List => cli::cmd_agents_list(url).await,
            AgentsCmd::Start { id } => {
                let client = reqwest::Client::new();
                client.post(format!("{url}/api/agents/{id}/start")).send().await?;
                eprintln!("  Agent {id} started");
                Ok(())
            }
            AgentsCmd::Stop { id } => {
                let client = reqwest::Client::new();
                client.post(format!("{url}/api/agents/{id}/stop")).send().await?;
                eprintln!("  Agent {id} stopped");
                Ok(())
            }
        },
        Some(Commands::Mechanic { repair }) => cli::cmd_mechanic(url, repair).await,
        Some(Commands::Daemon(sub)) => match sub {
            DaemonCmd::Install { config } => {
                let binary = std::env::current_exe()?.to_string_lossy().to_string();
                let path = ironclad_server::daemon::install_daemon(&binary, &config, 18789)?;
                eprintln!("  Daemon installed: {}", path.display());
                Ok(())
            }
            DaemonCmd::Status => {
                let pid_path = ironclad_core::config::DaemonConfig::default().pid_file;
                match ironclad_server::daemon::read_pid_file(&pid_path) {
                    Ok(Some(pid)) => eprintln!("  Daemon running (PID {pid})"),
                    Ok(None) => eprintln!("  Daemon not running"),
                    Err(e) => eprintln!("  Error reading PID: {e}"),
                }
                Ok(())
            }
            DaemonCmd::Uninstall => {
                ironclad_server::daemon::uninstall_daemon()?;
                eprintln!("  Daemon uninstalled");
                Ok(())
            }
        },
        Some(Commands::Uninstall { purge }) => cli::cmd_uninstall(purge),
        Some(Commands::Reset { yes }) => cli::cmd_reset(yes),
        Some(Commands::Update { channel, yes, no_restart }) => cli::cmd_update(&channel, yes, no_restart).await,
        Some(Commands::Security(sub)) => match sub {
            SecurityCmd::Audit { config } => cli::cmd_security_audit(&config),
        },
        Some(Commands::Completion { shell }) => cli::cmd_completion(&shell),
        None => cmd_serve(None, None, None).await,
    };
    eprint!("{}", t.hard_reset());
    result
}

fn print_banner(t: &Theme) {
    use ironclad_core::style::sleep_ms;

    let version = env!("CARGO_PKG_VERSION");
    let p = t.accent();
    let d = t.dim();
    let r = t.reset();
    let scan = if t.colors_enabled() { 55 } else { 0 };

    let _sound = t.start_typing_sound();

    eprintln!();
    eprintln!("{p}        \u{2554}\u{2550}\u{2550}\u{2550}\u{2557}{r}");
    sleep_ms(scan);
    eprintln!("{p}        \u{2551}\u{25c9} \u{25c9}\u{2551}{r}");
    sleep_ms(scan);
    eprintln!("{p}        \u{2551} \u{25ac} \u{2551}{r}");
    sleep_ms(scan);
    eprintln!("{p}        \u{255a}\u{2550}\u{2564}\u{2550}\u{255d}{r}");
    sleep_ms(scan);

    eprint!("{p}      \u{2554}\u{2550}\u{2550}\u{2550}\u{256a}\u{2550}\u{2550}\u{2550}\u{2557}{r}       ");
    t.typewrite_line(&format!("{p}I R O N C L A D{r}"), 35);

    eprint!("{p}  \u{2554}\u{2550}\u{2550}\u{2550}\u{2563} \u{2593}\u{2593}\u{2551}\u{2593}\u{2593} \u{2560}\u{2550}\u{2550}\u{2550}\u{2557}{r}   ");
    t.typewrite_line(&format!("{d}Autonomous Agent Runtime v{version}{r}"), 18);

    eprintln!("{p}  \u{2588}   \u{2551} \u{2593}\u{2593}\u{2551}\u{2593}\u{2593} \u{2551}   \u{2588}{r}");
    sleep_ms(scan);
    eprintln!("{p}      \u{255a}\u{2550}\u{2550}\u{2564}\u{2550}\u{2564}\u{2550}\u{2550}\u{255d}{r}");
    sleep_ms(scan);
    eprintln!("{p}         \u{2551} \u{2551}{r}");
    sleep_ms(scan);
    eprintln!("{p}        \u{2550}\u{2569}\u{2550}\u{2569}\u{2550}{r}");
    eprintln!();

    drop(_sound);
}

fn step(t: &Theme, n: u32, total: u32, msg: &str) {
    let (d, b, r) = (t.dim(), t.bold(), t.reset());
    t.typewrite_line(
        &format!("  \u{2705} {d}[{n:>2}/{total}]{r} {b}{msg}{r}"),
        4,
    );
}

fn step_warn(t: &Theme, n: u32, total: u32, msg: &str) {
    let (d, r) = (t.dim(), t.reset());
    t.typewrite_line(
        &format!("  \u{26a0}\u{fe0f} {d}[{n:>2}/{total}]{r} {msg}"),
        4,
    );
}

fn step_detail(t: &Theme, label: &str, value: &str) {
    let (d, a, r) = (t.dim(), t.accent(), t.reset());
    t.typewrite_line(
        &format!("       \u{25b8} {d}{label}: {a}{value}{r}"),
        4,
    );
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

    let mut config = match config_path {
        Some(ref p) => {
            step(&t, 1, STEPS, "Loading configuration");
            step_detail(&t, "source", p);
            IroncladConfig::from_file(Path::new(p))?
        }
        None => {
            step(&t, 1, STEPS, "Using default configuration");
            step_detail(&t, "source", "built-in defaults");
            IroncladConfig::from_str(FALLBACK_CONFIG)?
        }
    };

    if let Some(p) = port_override {
        config.server.port = p;
    }
    if let Some(b) = bind_override {
        config.server.bind = b;
    }

    config.validate().map_err(|e| {
        let (er, r) = (t.error(), t.reset());
        eprintln!("  {er}\u{26d3}{r} Configuration validation failed: {e}");
        e
    })?;
    step(&t, 2, STEPS, "Configuration validated");

    if config.server.bind != "127.0.0.1" && config.server.api_key.is_none() {
        let (w, r) = (t.warn(), t.reset());
        eprintln!();
        eprintln!("  {w}WARNING:{r} Server bound to {} with no API key.", config.server.bind);
        eprintln!("           All endpoints are unauthenticated.");
        eprintln!("           Set [server] api_key = \"...\" in your config to secure the API.");
        eprintln!();
    }

    let app = bootstrap(config.clone()).await?;
    step(&t, 3, STEPS, "Tracing initialized");
    step_detail(&t, "level", &config.agent.log_level);

    let db_path = config.database.path.to_string_lossy();
    step(&t, 4, STEPS, "Database initialized");
    step_detail(&t, "path", &db_path);
    if db_path == ":memory:" {
        step_detail(&t, "mode", "in-memory (ephemeral)");
    } else {
        step_detail(&t, "mode", "WAL (persistent)");
    }

    step(&t, 5, STEPS, "Wallet service ready");
    step_detail(&t, "chain", &format!("chain_id={}", config.wallet.chain_id));
    step_detail(&t, "rpc", &config.wallet.rpc_url);

    step(&t, 6, STEPS, "Identity resolved");
    step_detail(&t, "name", &config.agent.name);
    step_detail(&t, "id", &config.agent.id);

    let _llm = ironclad_llm::LlmService::new(&config);
    step(&t, 7, STEPS, "LLM service ready");
    step_detail(&t, "primary", &config.models.primary);
    let fallback_str = if config.models.fallbacks.is_empty() {
        "none".to_string()
    } else {
        config.models.fallbacks.join(", ")
    };
    step_detail(&t, "fallbacks", &fallback_str);
    step_detail(&t, "routing", &config.models.routing.mode);

    step(&t, 8, STEPS, "Agent loop initialized");

    if config.skills.skills_dir.exists() {
        step(&t, 9, STEPS, "Skills loaded");
        step_detail(&t, "dir", &config.skills.skills_dir.display().to_string());
    } else {
        step_warn(
            &t,
            9,
            STEPS,
            &format!(
                "Skills directory not found: {}",
                config.skills.skills_dir.display()
            ),
        );
    }

    let _heartbeat = ironclad_schedule::HeartbeatDaemon::new(60_000);
    step(&t, 10, STEPS, "Scheduler initialized");
    step_detail(&t, "heartbeat", "60s");

    let mut channels = vec!["web"];
    if config.channels.telegram.is_some() {
        channels.push("telegram");
    }
    if config.channels.whatsapp.is_some() {
        channels.push("whatsapp");
    }
    if config.a2a.enabled {
        channels.push("a2a");
    }
    step(&t, 11, STEPS, "Channel adapters ready");
    step_detail(&t, "active", &channels.join(", "));

    let bind_addr = format!("{}:{}", config.server.bind, config.server.port);
    step(&t, 12, STEPS, "HTTP server starting");
    step_detail(&t, "bind", &bind_addr);
    step_detail(&t, "dashboard", &format!("http://{bind_addr}"));

    let elapsed = boot_start.elapsed();
    let (a, b, r) = (t.accent(), t.bold(), t.reset());
    eprintln!();
    eprint!("  \u{26a1} ");
    t.typewrite(&format!("{b}Ready{r} in {a}{:.0?}{r}", elapsed), 25);
    eprintln!();
    eprintln!();

    info!("Ironclad listening on http://{bind_addr}");
    let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
    axum::serve(listener, app).await?;

    Ok(())
}

fn cmd_init(path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    print_banner(t);
    let dir = std::path::Path::new(path);
    let (b, r) = (t.bold(), t.reset());

    t.typewrite_line(&format!("  {b}Initializing Ironclad workspace{r} at {}\n", dir.display()), 4);

    let config_path = dir.join("ironclad.toml");
    if config_path.exists() {
        t.typewrite_line(&format!("  \u{26a0}\u{fe0f} ironclad.toml already exists, skipping"), 4);
    } else {
        std::fs::write(&config_path, INIT_CONFIG)?;
        t.typewrite_line(&format!("  \u{26a1} Created ironclad.toml"), 4);
    }

    let skills_dir = dir.join("skills");
    if skills_dir.exists() {
        t.typewrite_line(&format!("  \u{26a0}\u{fe0f} skills/ directory already exists, skipping"), 4);
    } else {
        std::fs::create_dir_all(&skills_dir)?;
        std::fs::write(
            skills_dir.join("example.md"),
            EXAMPLE_SKILL,
        )?;
        t.typewrite_line(&format!("  \u{26a1} Created skills/ with example skill"), 4);
    }

    eprintln!();
    t.typewrite_line(&format!("  \u{2705} Done. Run {b}ironclad serve -c ironclad.toml{r} to start."), 4);
    eprintln!();

    Ok(())
}

fn cmd_check(config_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    let t = cli::theme();
    print_banner(t);
    let (s, b, r) = (t.success(), t.bold(), t.reset());
    let tw = |text: &str| t.typewrite_line(text, 4);

    tw(&format!("  {b}Validating{r} {config_path}\n"));

    let config = IroncladConfig::from_file(Path::new(config_path))?;
    tw("  \u{2705} TOML syntax valid");

    config.validate()?;
    tw("  \u{2705} Configuration semantics valid");

    tw(&format!("  \u{2705} Agent: {} ({})", config.agent.name, config.agent.id));
    tw(&format!("  \u{2705} Server: {}:{}", config.server.bind, config.server.port));
    tw(&format!("  \u{2705} Primary model: {}", config.models.primary));
    tw(&format!("  \u{2705} Database: {}", config.database.path.display()));

    let mem_sum = config.memory.working_budget_pct
        + config.memory.episodic_budget_pct
        + config.memory.semantic_budget_pct
        + config.memory.procedural_budget_pct
        + config.memory.relationship_budget_pct;
    tw(&format!("  \u{2705} Memory budgets sum to {mem_sum}%"));

    tw(&format!("  \u{2705} Treasury: cap=${:.2}/payment, reserve=${:.2}", config.treasury.per_payment_cap, config.treasury.minimum_reserve));

    if config.skills.skills_dir.exists() {
        tw(&format!("  \u{2705} Skills dir exists: {}", config.skills.skills_dir.display()));
    } else {
        tw(&format!("  \u{26a0}\u{fe0f} Skills dir missing: {}", config.skills.skills_dir.display()));
    }

    if config.a2a.enabled {
        tw(&format!("  \u{2705} A2A enabled (rate limit: {}/peer)", config.a2a.rate_limit_per_peer));
    }

    eprintln!();
    tw(&format!("  \u{2705} {s}All checks passed.{r}"));
    eprintln!();

    Ok(())
}

fn cmd_version() {
    let t = cli::theme();
    print_banner(t);
    let tw = |text: &str| t.typewrite_line(text, 4);
    tw(&format!("  version:    {}", env!("CARGO_PKG_VERSION")));
    tw(&format!("  edition:    Rust 2024"));
    tw(&format!("  target:     {}", std::env::consts::ARCH));
    tw(&format!("  os:         {}", std::env::consts::OS));
    eprintln!();
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
# See: https://github.com/ironclad/ironclad#configuration

[agent]
name = "MyAgent"
id = "my-agent"
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
mode = "rule"
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

const EXAMPLE_SKILL: &str = r#"---
name: hello-world
description: A simple example skill that demonstrates the instruction format
triggers:
  keywords: [hello, greet, introduce]
priority: 5
---

When triggered, greet the user warmly and explain that you are an Ironclad agent.
Mention your agent name and current capabilities.
Keep the greeting concise but friendly.
"#;
