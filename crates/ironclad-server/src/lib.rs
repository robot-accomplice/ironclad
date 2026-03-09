//! # ironclad-server
//!
//! Top-level binary crate that assembles all Ironclad workspace crates into a
//! single runtime. The [`bootstrap()`] function initializes the database,
//! wallet, LLM pipeline, agent loop, channel adapters, and background daemons,
//! then returns an axum `Router` ready to serve.
//!
//! ## Key Types
//!
//! - [`AppState`] -- Shared application state passed to all route handlers
//! - [`PersonalityState`] -- Loaded personality files (soul, firmware, identity)
//! - [`EventBus`] -- Tokio broadcast channel for WebSocket event push
//!
//! ## Modules
//!
//! - `api` -- REST API mount point, `build_router()`, route modules
//! - `auth` -- API key authentication middleware layer
//! - `rate_limit` -- Global + per-IP rate limiting (sliding window)
//! - `dashboard` -- Embedded SPA serving (compile-time or filesystem)
//! - `ws` -- WebSocket upgrade and event broadcasting
//! - `cli/` -- CLI command handlers (serve, status, sessions, memory, wallet, etc.)
//! - `daemon` -- Daemon install, status, uninstall
//! - `migrate/` -- Migration engine, skill import/export
//! - `plugins` -- Plugin registry initialization and loading
//!
//! ## Bootstrap Sequence
//!
//! 1. Parse CLI → load config → init DB → load wallet → generate HMAC secret
//! 2. Init LLM client + router + embedding → load semantic cache
//! 3. Init agent loop + tool registry + memory retriever
//! 4. Register channel adapters (Telegram, WhatsApp, Discord, Signal)
//! 5. Spawn background daemons (heartbeat, cron, cache flush, ANN rebuild)
//! 6. Build axum router with auth + CORS + rate limiting

pub mod abuse;
pub mod api;
pub mod auth;
pub mod cli;
pub mod config_maintenance;
pub mod config_runtime;
mod cron_runtime;
pub mod daemon;
pub mod dashboard;
pub mod migrate;
pub mod plugins;
pub mod rate_limit;
pub mod state_hygiene;
pub mod ws;
pub mod ws_ticket;

pub use api::{AppState, PersonalityState, build_mcp_router, build_public_router, build_router};
pub use dashboard::{build_dashboard_html, dashboard_handler};
pub use ws::{EventBus, ws_route};
pub use ws_ticket::TicketStore;

use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::RwLock;
use tower_http::cors::CorsLayer;

use auth::ApiKeyLayer;
use ironclad_agent::policy::{
    AuthorityRule, CommandSafetyRule, FinancialRule, PathProtectionRule, PolicyEngine,
    RateLimitRule, ValidationRule,
};
use ironclad_agent::subagents::SubagentRegistry;
use ironclad_browser::Browser;
use ironclad_channels::ChannelAdapter;
use ironclad_channels::a2a::A2aProtocol;
use ironclad_channels::router::ChannelRouter;
use ironclad_channels::telegram::TelegramAdapter;
use ironclad_channels::whatsapp::WhatsAppAdapter;
use ironclad_core::IroncladConfig;
use ironclad_db::Database;
use ironclad_llm::LlmService;
use ironclad_llm::OAuthManager;
use ironclad_wallet::{WalletPaymentHandler, WalletService};

use ironclad_agent::approvals::ApprovalManager;
use ironclad_agent::obsidian::ObsidianVault;
use ironclad_agent::obsidian_tools::{ObsidianReadTool, ObsidianSearchTool, ObsidianWriteTool};
use ironclad_agent::tools::{
    BashTool, EchoTool, EditFileTool, GlobFilesTool, ListDirectoryTool, ReadFileTool,
    ScriptRunnerTool, SearchFilesTool, ToolRegistry, WriteFileTool,
};
use ironclad_channels::discord::DiscordAdapter;
use ironclad_channels::email::EmailAdapter;
use ironclad_channels::signal::SignalAdapter;
use ironclad_channels::voice::{VoiceConfig, VoicePipeline};

use rate_limit::GlobalRateLimitLayer;

static STDERR_ENABLED: AtomicBool = AtomicBool::new(false);
static LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

fn is_taskable_subagent_role(role: &str) -> bool {
    role.eq_ignore_ascii_case("subagent") || role.eq_ignore_ascii_case("specialist")
}

pub fn enable_stderr_logging() {
    STDERR_ENABLED.store(true, Ordering::Release);
}

fn init_logging(config: &IroncladConfig) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::Layer;
    use tracing_subscriber::filter::filter_fn;
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let level = config.agent.log_level.as_str();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let stderr_gate = filter_fn(|_| STDERR_ENABLED.load(Ordering::Acquire));

    let log_dir = &config.server.log_dir;
    if std::fs::create_dir_all(log_dir).is_ok() {
        let file_appender = tracing_appender::rolling::daily(log_dir, "ironclad.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        let _ = LOG_GUARD.set(guard);

        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .json();

        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(stderr_gate);

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init();
    } else {
        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr)
            .with_filter(stderr_gate);
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .try_init();
    }

    cleanup_old_logs(log_dir, config.server.log_max_days);
}

fn cleanup_old_logs(log_dir: &std::path::Path, max_days: u32) {
    let cutoff =
        std::time::SystemTime::now() - std::time::Duration::from_secs(u64::from(max_days) * 86400);

    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = entry.metadata()
            && let Ok(modified) = meta.modified()
            && modified < cutoff
        {
            let _ = std::fs::remove_file(&path);
        }
    }
}

/// Resolve a channel token: keystore reference first, then env var fallback.
fn resolve_token(
    token_ref: &Option<String>,
    token_env: &str,
    keystore: &ironclad_core::Keystore,
) -> String {
    if let Some(r) = token_ref
        && let Some(name) = r.strip_prefix("keystore:")
    {
        if let Some(val) = keystore.get(name) {
            return val;
        }
        tracing::warn!(key = %name, "keystore reference not found, falling back to env var");
    }
    if !token_env.is_empty() {
        return match std::env::var(token_env) {
            Ok(val) if !val.is_empty() => val,
            Ok(_) => {
                tracing::warn!(env_var = %token_env, "API key env var is set but empty");
                String::new()
            }
            Err(_) => {
                tracing::warn!(env_var = %token_env, "API key env var is not set");
                String::new()
            }
        };
    }
    String::new()
}

/// Builds the application state and router from config. Used by the binary and by tests.
pub async fn bootstrap(config: IroncladConfig) -> Result<axum::Router, Box<dyn std::error::Error>> {
    bootstrap_with_config_path(config, None).await
}

pub async fn bootstrap_with_config_path(
    config: IroncladConfig,
    config_path: Option<std::path::PathBuf>,
) -> Result<axum::Router, Box<dyn std::error::Error>> {
    init_logging(&config);

    let personality_state = api::PersonalityState::from_workspace(&config.agent.workspace);

    if !personality_state.soul_text.is_empty() {
        tracing::info!(
            personality = %personality_state.identity.name,
            generated_by = %personality_state.identity.generated_by,
            "Loaded personality files from workspace"
        );
    } else {
        tracing::info!("No personality files found in workspace, using defaults");
    }

    let db_path = config.database.path.to_string_lossy().to_string();
    let db = Database::new(&db_path)?;
    match crate::state_hygiene::run_state_hygiene(&config.database.path) {
        Ok(report) if report.changed => {
            tracing::info!(
                changed_rows = report.changed_rows,
                subagent_rows_normalized = report.subagent_rows_normalized,
                cron_payload_rows_repaired = report.cron_payload_rows_repaired,
                cron_jobs_disabled_invalid_expr = report.cron_jobs_disabled_invalid_expr,
                "applied startup mechanic checks"
            );
        }
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "startup mechanic checks failed"),
    }
    match ironclad_db::sessions::backfill_nicknames(&db) {
        Ok(0) => {}
        Ok(n) => tracing::info!(count = n, "Backfilled session nicknames"),
        Err(e) => tracing::warn!(error = %e, "Failed to backfill session nicknames"),
    }
    let mut llm = LlmService::new(&config)?;
    // Seed quality tracker with recent observations so metascore routing
    // has a warm start instead of assuming 0.8 for every model.
    match ironclad_db::metrics::recent_quality_scores(&db, 200) {
        Ok(scores) if !scores.is_empty() => llm.quality.seed_from_history(&scores),
        Ok(_) => {}
        Err(e) => tracing::warn!(error = %e, "failed to seed quality tracker from history"),
    }
    let wallet = WalletService::new(&config).await?;

    // Wire x402 payment handler: when the LLM client hits an HTTP 402, it
    // signs an EIP-3009 authorization via the agent wallet and retries.
    let wallet_arc = Arc::new(wallet.wallet.clone());
    let x402_handler = Arc::new(WalletPaymentHandler::new(wallet_arc));
    llm.set_payment_handler(x402_handler);

    let a2a = A2aProtocol::new(config.a2a.clone());
    let plugin_registry = plugins::init_plugin_registry(&config.plugins).await;
    let mut policy_engine = PolicyEngine::new();
    policy_engine.add_rule(Box::new(AuthorityRule));
    policy_engine.add_rule(Box::new(CommandSafetyRule));
    policy_engine.add_rule(Box::new(FinancialRule::new(
        config.treasury.per_payment_cap,
    )));
    policy_engine.add_rule(Box::new(PathProtectionRule::default()));
    policy_engine.add_rule(Box::new(RateLimitRule::default()));
    policy_engine.add_rule(Box::new(ValidationRule));
    let policy_engine = Arc::new(policy_engine);
    let browser = Arc::new(Browser::new(config.browser.clone()));
    let registry = Arc::new(SubagentRegistry::new(4, vec![]));

    if let Ok(sub_agents) = ironclad_db::agents::list_enabled_sub_agents(&db) {
        for sa in &sub_agents {
            if !is_taskable_subagent_role(&sa.role) {
                continue;
            }
            let resolved_model = match sa.model.trim().to_ascii_lowercase().as_str() {
                "auto" | "orchestrator" => llm.router.select_model().to_string(),
                _ => sa.model.clone(),
            };
            let fixed_skills = sa
                .skills_json
                .as_deref()
                .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
                .unwrap_or_default();
            let agent_config = ironclad_agent::subagents::AgentInstanceConfig {
                id: sa.name.clone(),
                name: sa.display_name.clone().unwrap_or_else(|| sa.name.clone()),
                model: resolved_model,
                skills: fixed_skills,
                allowed_subagents: vec![],
                max_concurrent: 4,
            };
            if let Err(e) = registry.register(agent_config).await {
                tracing::warn!(agent = %sa.name, err = %e, "failed to register sub-agent");
            } else if let Err(e) = registry.start_agent(&sa.name).await {
                tracing::warn!(agent = %sa.name, err = %e, "failed to auto-start sub-agent");
            }
        }
        if !sub_agents.is_empty() {
            tracing::info!(
                count = sub_agents.len(),
                "registered sub-agents from database"
            );
        }
    }

    let event_bus = EventBus::new(256);

    let keystore =
        ironclad_core::keystore::Keystore::new(ironclad_core::keystore::Keystore::default_path());
    if let Err(e) = keystore.unlock_machine() {
        tracing::warn!("keystore auto-unlock failed: {e}");
    }
    let keystore = Arc::new(keystore);

    let channel_router = Arc::new(ChannelRouter::with_store(db.clone()).await);
    let telegram: Option<Arc<TelegramAdapter>> =
        if let Some(ref tg_config) = config.channels.telegram {
            if tg_config.enabled {
                let token = resolve_token(&tg_config.token_ref, &tg_config.token_env, &keystore);
                if !token.is_empty() {
                    let adapter = Arc::new(TelegramAdapter::with_config(
                        token,
                        tg_config.poll_timeout_seconds,
                        tg_config.allowed_chat_ids.clone(),
                        tg_config.webhook_secret.clone(),
                        config.security.deny_on_empty_allowlist,
                    ));
                    channel_router
                        .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                        .await;
                    tracing::info!("Telegram adapter registered");
                    if tg_config.webhook_secret.is_none() {
                        tracing::warn!(
                            "Telegram webhook_secret not set; webhook endpoint will reject with 503"
                        );
                    }
                    Some(adapter)
                } else {
                    tracing::warn!(
                        token_env = %tg_config.token_env,
                        "Telegram enabled but token is empty"
                    );
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

    let whatsapp: Option<Arc<WhatsAppAdapter>> =
        if let Some(ref wa_config) = config.channels.whatsapp {
            if wa_config.enabled {
                let token = resolve_token(&wa_config.token_ref, &wa_config.token_env, &keystore);
                if !token.is_empty() && !wa_config.phone_number_id.is_empty() {
                    let adapter = Arc::new(WhatsAppAdapter::with_config(
                        token,
                        wa_config.phone_number_id.clone(),
                        wa_config.verify_token.clone(),
                        wa_config.allowed_numbers.clone(),
                        wa_config.app_secret.clone(),
                        config.security.deny_on_empty_allowlist,
                    )?);
                    channel_router
                        .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                        .await;
                    tracing::info!("WhatsApp adapter registered");
                    if wa_config.app_secret.is_none() {
                        tracing::warn!(
                            "WhatsApp app_secret not set; webhook endpoint will reject with 503"
                        );
                    }
                    Some(adapter)
                } else {
                    tracing::warn!("WhatsApp enabled but token env or phone_number_id is empty");
                    None
                }
            } else {
                None
            }
        } else {
            None
        };

    let discord: Option<Arc<DiscordAdapter>> = if let Some(ref dc_config) = config.channels.discord
    {
        if dc_config.enabled {
            let token = resolve_token(&dc_config.token_ref, &dc_config.token_env, &keystore);
            if !token.is_empty() {
                let adapter = Arc::new(DiscordAdapter::with_config(
                    token,
                    dc_config.allowed_guild_ids.clone(),
                    config.security.deny_on_empty_allowlist,
                ));
                channel_router
                    .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                    .await;
                // Start WebSocket gateway for real-time event reception
                if let Err(e) = adapter.connect_gateway().await {
                    tracing::error!(error = %e, "Failed to connect Discord gateway");
                }
                tracing::info!("Discord adapter registered with gateway");
                Some(adapter)
            } else {
                tracing::warn!(
                    token_env = %dc_config.token_env,
                    "Discord enabled but token env var is empty"
                );
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let signal: Option<Arc<SignalAdapter>> = if let Some(ref sig_config) = config.channels.signal {
        if sig_config.enabled {
            if !sig_config.phone_number.is_empty() {
                let adapter = Arc::new(SignalAdapter::with_config(
                    sig_config.phone_number.clone(),
                    sig_config.daemon_url.clone(),
                    sig_config.allowed_numbers.clone(),
                    config.security.deny_on_empty_allowlist,
                ));
                channel_router
                    .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                    .await;
                tracing::info!("Signal adapter registered");
                Some(adapter)
            } else {
                tracing::warn!("Signal enabled but phone_number is empty");
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let email: Option<Arc<EmailAdapter>> = if config.channels.email.enabled {
        let email_cfg = &config.channels.email;
        let password = if email_cfg.password_env.is_empty() {
            String::new()
        } else {
            match std::env::var(&email_cfg.password_env) {
                Ok(val) => val,
                Err(_) => {
                    tracing::warn!(env_var = %email_cfg.password_env, "email password env var is not set");
                    String::new()
                }
            }
        };
        if email_cfg.smtp_host.is_empty()
            || email_cfg.username.is_empty()
            || password.is_empty()
            || email_cfg.from_address.is_empty()
        {
            tracing::warn!("Email enabled but SMTP credentials are incomplete");
            None
        } else {
            match EmailAdapter::new(
                email_cfg.from_address.clone(),
                email_cfg.smtp_host.clone(),
                email_cfg.smtp_port,
                email_cfg.imap_host.clone(),
                email_cfg.imap_port,
                email_cfg.username.clone(),
                password,
            ) {
                Ok(email_adapter) => {
                    // Resolve OAuth2 token from env if configured
                    let oauth2_token =
                        if email_cfg.use_oauth2 && !email_cfg.oauth2_token_env.is_empty() {
                            std::env::var(&email_cfg.oauth2_token_env).ok()
                        } else {
                            None
                        };
                    let adapter = Arc::new(
                        email_adapter
                            .with_allowed_senders(email_cfg.allowed_senders.clone())
                            .with_deny_on_empty(config.security.deny_on_empty_allowlist)
                            .with_poll_interval(std::time::Duration::from_secs(
                                email_cfg.poll_interval_seconds,
                            ))
                            .with_oauth2_token(oauth2_token)
                            .with_imap_idle_enabled(email_cfg.imap_idle_enabled),
                    );
                    channel_router
                        .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                        .await;
                    tracing::info!("Email adapter registered");

                    // Start background IMAP listener if IMAP is configured
                    if !email_cfg.imap_host.is_empty()
                        && let Err(e) = adapter.start_imap_listener().await
                    {
                        tracing::error!(error = %e, "Failed to start email IMAP listener");
                    }

                    Some(adapter)
                }
                Err(e) => {
                    tracing::error!(error = %e, "Failed to create email adapter");
                    None
                }
            }
        }
    } else {
        None
    };

    let voice: Option<Arc<RwLock<VoicePipeline>>> = if config.channels.voice.enabled {
        let mut voice_config = VoiceConfig::default();
        if let Some(stt) = &config.channels.voice.stt_model {
            voice_config.stt_model = stt.clone();
        }
        if let Some(tts) = &config.channels.voice.tts_model {
            voice_config.tts_model = tts.clone();
        }
        if let Some(v) = &config.channels.voice.tts_voice {
            voice_config.tts_voice = v.clone();
        }
        if let Ok(key) = std::env::var("OPENAI_API_KEY")
            && !key.is_empty()
        {
            voice_config.api_key = Some(key);
        }
        tracing::info!("Voice pipeline initialized");
        Some(Arc::new(RwLock::new(VoicePipeline::new(voice_config))))
    } else {
        None
    };

    let hmac_secret = {
        use rand::RngCore;
        let mut buf = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        buf
    };

    let retriever = Arc::new(ironclad_agent::retrieval::MemoryRetriever::new(
        config.memory.clone(),
    ));

    let mut tool_registry = ToolRegistry::new();
    tool_registry.register(Box::new(EchoTool));
    tool_registry.register(Box::new(BashTool));
    tool_registry.register(Box::new(ScriptRunnerTool::new(config.skills.clone())));
    tool_registry.register(Box::new(ReadFileTool));
    tool_registry.register(Box::new(WriteFileTool));
    tool_registry.register(Box::new(EditFileTool));
    tool_registry.register(Box::new(ListDirectoryTool));
    tool_registry.register(Box::new(GlobFilesTool));
    tool_registry.register(Box::new(SearchFilesTool));

    // Introspection tools — read-only runtime probes for agent self-awareness
    use ironclad_agent::tools::{
        GetChannelHealthTool, GetMemoryStatsTool, GetRuntimeContextTool, GetSubagentStatusTool,
    };
    tool_registry.register(Box::new(GetRuntimeContextTool));
    tool_registry.register(Box::new(GetMemoryStatsTool));
    tool_registry.register(Box::new(GetChannelHealthTool));
    tool_registry.register(Box::new(GetSubagentStatusTool));

    // Data tools — agent-managed tables via hippocampus
    use ironclad_agent::tools::{AlterTableTool, CreateTableTool, DropTableTool};
    tool_registry.register(Box::new(CreateTableTool));
    tool_registry.register(Box::new(AlterTableTool));
    tool_registry.register(Box::new(DropTableTool));

    // Obsidian vault integration
    let obsidian_vault: Option<Arc<RwLock<ObsidianVault>>> = if config.obsidian.enabled {
        match ObsidianVault::from_config(&config.obsidian) {
            Ok(vault) => {
                let vault = Arc::new(RwLock::new(vault));
                tool_registry.register(Box::new(ObsidianReadTool::new(Arc::clone(&vault))));
                tool_registry.register(Box::new(ObsidianWriteTool::new(Arc::clone(&vault))));
                tool_registry.register(Box::new(ObsidianSearchTool::new(Arc::clone(&vault))));
                tracing::info!("Obsidian vault integration enabled");
                Some(vault)
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to initialize Obsidian vault");
                None
            }
        }
    } else {
        None
    };

    // Start vault file watcher if configured
    if let Some(ref vault) = obsidian_vault
        && config.obsidian.watch_for_changes
    {
        match ironclad_agent::obsidian::watcher::VaultWatcher::start(Arc::clone(vault)) {
            Ok(_watcher) => {
                // watcher lives in the spawned task — drop is fine here
                tracing::info!("Obsidian vault file watcher started");
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to start Obsidian vault watcher");
            }
        }
    }

    let tool_registry = Arc::new(tool_registry);

    let approvals = Arc::new(ApprovalManager::new(config.approvals.clone()));

    let oauth = Arc::new(OAuthManager::new()?);

    let discovery_registry = Arc::new(RwLock::new(
        ironclad_agent::discovery::DiscoveryRegistry::new(),
    ));
    let device_manager = Arc::new(RwLock::new(ironclad_agent::device::DeviceManager::new(
        ironclad_agent::device::DeviceIdentity::generate(&config.agent.id),
        config.devices.max_paired_devices,
    )));

    let mut mcp_clients = ironclad_agent::mcp::McpClientManager::new();
    for c in &config.mcp.clients {
        let mut conn = ironclad_agent::mcp::McpClientConnection::new(c.name.clone(), c.url.clone());
        if let Err(e) = conn.discover() {
            tracing::warn!(mcp_name = %c.name, error = %e, "MCP client discovery failed at startup");
        }
        mcp_clients.add_connection(conn);
    }
    let mcp_clients = Arc::new(RwLock::new(mcp_clients));

    let mut mcp_server_registry = ironclad_agent::mcp::McpServerRegistry::new();
    let exported = ironclad_agent::mcp::export_tools_as_mcp(
        &tool_registry
            .list()
            .iter()
            .map(|t| {
                (
                    t.name().to_string(),
                    t.description().to_string(),
                    t.parameters_schema(),
                )
            })
            .collect::<Vec<_>>(),
    );
    for tool in exported {
        mcp_server_registry.register_tool(tool);
    }
    mcp_server_registry.register_resource(ironclad_agent::mcp::McpResource {
        uri: "ironclad://sessions/active".to_string(),
        name: "Active Sessions".to_string(),
        description: "Active sessions in the local runtime".to_string(),
        mime_type: "application/json".to_string(),
    });
    mcp_server_registry.register_resource(ironclad_agent::mcp::McpResource {
        uri: "ironclad://metrics/capacity".to_string(),
        name: "Provider Capacity Stats".to_string(),
        description: "Current provider utilization and headroom".to_string(),
        mime_type: "application/json".to_string(),
    });
    let mcp_server_registry = Arc::new(RwLock::new(mcp_server_registry));

    let ann_index = ironclad_db::ann::AnnIndex::new(config.memory.ann_index);
    if config.memory.ann_index {
        match ann_index.build_from_db(&db) {
            Ok(count) => {
                if ann_index.is_built() {
                    tracing::info!(count, "ANN index built from database");
                } else {
                    tracing::info!(
                        count,
                        min = ann_index.min_entries_for_index,
                        "ANN index below threshold, brute-force search will be used"
                    );
                }
            }
            Err(e) => {
                tracing::warn!(error = %e, "Failed to build ANN index, falling back to brute-force");
            }
        }
    }

    // Initialize media service for multimodal attachment handling
    let media_service = if config.multimodal.enabled {
        match ironclad_channels::media::MediaService::new(&config.multimodal) {
            Ok(svc) => {
                tracing::info!(
                    media_dir = ?config.multimodal.media_dir,
                    "Media service initialized"
                );
                Some(Arc::new(svc))
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to initialize media service");
                None
            }
        }
    } else {
        None
    };

    let resolved_config_path =
        config_path.unwrap_or_else(crate::config_runtime::resolve_default_config_path);

    // Build rate limiter once — AppState and the middleware layer share the same
    // Arc<Mutex<…>> so admin observability sees live counters.
    let rate_limiter = GlobalRateLimitLayer::new(
        u64::from(config.server.rate_limit_requests),
        Duration::from_secs(config.server.rate_limit_window_secs),
    )
    .with_per_ip_capacity(u64::from(config.server.per_ip_rate_limit_requests))
    .with_per_actor_capacity(u64::from(config.server.per_actor_rate_limit_requests))
    .with_trusted_proxy_cidrs(&config.server.trusted_proxy_cidrs);

    let state = AppState {
        db,
        config: Arc::new(RwLock::new(config.clone())),
        llm: Arc::new(RwLock::new(llm)),
        wallet: Arc::new(wallet),
        a2a: Arc::new(RwLock::new(a2a)),
        personality: Arc::new(RwLock::new(personality_state)),
        hmac_secret: Arc::new(hmac_secret),
        interviews: Arc::new(RwLock::new(std::collections::HashMap::new())),
        plugins: plugin_registry,
        policy_engine,
        browser,
        registry,
        event_bus: event_bus.clone(),
        channel_router,
        telegram,
        whatsapp,
        retriever,
        ann_index,
        tools: tool_registry,
        approvals,
        discord,
        signal,
        email,
        voice,
        media_service,
        discovery: discovery_registry,
        devices: device_manager,
        mcp_clients,
        mcp_server: mcp_server_registry,
        oauth,
        keystore,
        obsidian: obsidian_vault,
        started_at: std::time::Instant::now(),
        config_path: Arc::new(resolved_config_path.clone()),
        config_apply_status: crate::config_runtime::status_for_path(&resolved_config_path),
        pending_specialist_proposals: Arc::new(RwLock::new(std::collections::HashMap::new())),
        ws_tickets: ws_ticket::TicketStore::new(),
        rate_limiter: rate_limiter.clone(),
    };

    // Periodic ANN index rebuild (every 10 minutes)
    if config.memory.ann_index {
        let ann_db = state.db.clone();
        let ann_idx = state.ann_index.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(600));
            interval.tick().await;
            loop {
                interval.tick().await;
                match ann_idx.rebuild(&ann_db) {
                    Ok(count) => {
                        tracing::debug!(count, "ANN index rebuilt");
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "ANN index rebuild failed");
                    }
                }
            }
        });
        tracing::info!("ANN index rebuild daemon spawned (10min interval)");
    }

    // Load persisted semantic cache
    {
        let loaded = ironclad_db::cache::load_cache_entries(&state.db)
            .inspect_err(|e| tracing::warn!(error = %e, "failed to load semantic cache entries"))
            .unwrap_or_default();
        if !loaded.is_empty() {
            let imported: Vec<(String, ironclad_llm::ExportedCacheEntry)> = loaded
                .into_iter()
                .map(|(id, pe)| {
                    let ttl = pe
                        .expires_at
                        .and_then(|e| {
                            chrono::NaiveDateTime::parse_from_str(&e, "%Y-%m-%dT%H:%M:%S")
                                .ok()
                                .or_else(|| {
                                    chrono::NaiveDateTime::parse_from_str(&e, "%Y-%m-%d %H:%M:%S")
                                        .ok()
                                })
                        })
                        .map(|exp| {
                            let now = chrono::Utc::now().naive_utc();
                            if exp > now {
                                (exp - now).num_seconds().max(0) as u64
                            } else {
                                0
                            }
                        })
                        .unwrap_or(3600);

                    (
                        id,
                        ironclad_llm::ExportedCacheEntry {
                            content: pe.response,
                            model: pe.model,
                            tokens_saved: pe.tokens_saved,
                            hits: pe.hit_count,
                            involved_tools: false,
                            embedding: pe.embedding,
                            ttl_remaining_secs: ttl,
                        },
                    )
                })
                .collect();
            let count = imported.len();
            let mut llm = state.llm.write().await;
            llm.cache.import_entries(imported);
            tracing::info!(count, "Loaded semantic cache from database");
        }
    }

    // Periodic cache flush (every 5 minutes)
    {
        let flush_db = state.db.clone();
        let flush_llm = Arc::clone(&state.llm);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(300));
            interval.tick().await; // skip first immediate tick
            loop {
                interval.tick().await;
                let entries = {
                    let llm = flush_llm.read().await;
                    llm.cache.export_entries()
                };
                for (hash, entry) in &entries {
                    let expires = chrono::Utc::now()
                        + chrono::Duration::seconds(entry.ttl_remaining_secs as i64);
                    let pe = ironclad_db::cache::PersistedCacheEntry {
                        prompt_hash: hash.clone(),
                        response: entry.content.clone(),
                        model: entry.model.clone(),
                        tokens_saved: entry.tokens_saved,
                        hit_count: entry.hits,
                        embedding: entry.embedding.clone(),
                        created_at: chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
                        expires_at: Some(expires.format("%Y-%m-%dT%H:%M:%S").to_string()),
                    };
                    ironclad_db::cache::save_cache_entry(&flush_db, hash, &pe)
                        .inspect_err(
                            |e| tracing::warn!(error = %e, hash, "failed to persist cache entry"),
                        )
                        .ok();
                }
                ironclad_db::cache::evict_expired_cache(&flush_db)
                    .inspect_err(|e| tracing::warn!(error = %e, "failed to evict expired cache"))
                    .ok();
                tracing::debug!(count = entries.len(), "Flushed semantic cache to database");
            }
        });
        tracing::info!("Cache flush daemon spawned (5min interval)");
    }

    // Start heartbeat daemon
    {
        let hb_wallet = Arc::clone(&state.wallet);
        let hb_db = state.db.clone();
        let hb_session_cfg = config.session.clone();
        let hb_digest_cfg = config.digest.clone();
        let hb_agent_id = config.agent.id.clone();
        let daemon = ironclad_schedule::HeartbeatDaemon::new(60_000);
        tokio::spawn(async move {
            ironclad_schedule::run_heartbeat(
                daemon,
                hb_wallet,
                hb_db,
                hb_session_cfg,
                hb_digest_cfg,
                hb_agent_id,
            )
            .await;
        });
        tracing::info!("Heartbeat daemon spawned (60s interval)");
    }

    // Delivery retry queue drain (every 30 seconds)
    {
        let drain_router = Arc::clone(&state.channel_router);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(30));
            interval.tick().await;
            loop {
                interval.tick().await;
                drain_router.drain_retry_queue().await;
            }
        });
        tracing::info!("Delivery retry queue drain daemon spawned (30s interval)");
    }

    // Start cron worker
    {
        let instance_id = config.agent.id.clone();
        let cron_state = state.clone();
        tokio::spawn(async move {
            crate::cron_runtime::run_cron_worker(cron_state, instance_id).await;
        });
        tracing::info!("Cron worker spawned");
    }

    // Periodic mechanic-check sweep (default 6h, per-instance).
    {
        let db_path = config.database.path.clone();
        let interval_secs = std::env::var("IRONCLAD_MECHANIC_CHECK_INTERVAL_SECS")
            .ok()
            .or_else(|| std::env::var("IRONCLAD_STATE_HYGIENE_INTERVAL_SECS").ok())
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v >= 300)
            .unwrap_or(21_600);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            interval.tick().await;
            loop {
                interval.tick().await;
                match crate::state_hygiene::run_state_hygiene(&db_path) {
                    Ok(report) if report.changed => {
                        tracing::info!(
                            changed_rows = report.changed_rows,
                            subagent_rows_normalized = report.subagent_rows_normalized,
                            cron_payload_rows_repaired = report.cron_payload_rows_repaired,
                            cron_jobs_disabled_invalid_expr =
                                report.cron_jobs_disabled_invalid_expr,
                            "periodic mechanic checks applied"
                        );
                    }
                    Ok(_) => tracing::debug!("periodic mechanic checks: no changes"),
                    Err(e) => tracing::warn!(error = %e, "periodic mechanic checks failed"),
                }
            }
        });
        tracing::info!(interval_secs, "Mechanic checks daemon spawned");
    }

    {
        let startup_announce_channels = config.channels.startup_announcement_channels();

        let announce_text = format!(
            "Ironclad online\nagent: {} ({})\nmodel.primary: {}\nrouting: {}\nserver: {}:{}\nskills_dir: {}\nstarted_at: {}",
            config.agent.name,
            config.agent.id,
            config.models.primary,
            config.models.routing.mode,
            config.server.bind,
            config.server.port,
            config.skills.skills_dir.display(),
            chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
        );

        if startup_announce_channels.iter().any(|c| c == "telegram") {
            if let (Some(adapter), Some(tg_cfg)) =
                (state.telegram.clone(), config.channels.telegram.as_ref())
            {
                let announce_targets = tg_cfg.allowed_chat_ids.clone();
                if announce_targets.is_empty() {
                    tracing::warn!(
                        "Telegram startup announcement skipped: channels.telegram.allowed_chat_ids is empty"
                    );
                } else {
                    let text = announce_text.clone();
                    tokio::spawn(async move {
                        for chat_id in announce_targets {
                            let chat = chat_id.to_string();
                            match adapter
                                .send(ironclad_channels::OutboundMessage {
                                    content: text.clone(),
                                    recipient_id: chat.clone(),
                                    metadata: None,
                                })
                                .await
                            {
                                Ok(()) => {
                                    tracing::info!(chat_id = %chat, "telegram startup announcement sent")
                                }
                                Err(e) => {
                                    tracing::warn!(chat_id = %chat, error = %e, "telegram startup announcement failed")
                                }
                            }
                        }
                    });
                }
            } else {
                tracing::warn!(
                    "Telegram startup announcement requested but telegram channel is not enabled/configured"
                );
            }
        }

        if startup_announce_channels.iter().any(|c| c == "whatsapp") {
            if let (Some(adapter), Some(wa_cfg)) =
                (state.whatsapp.clone(), config.channels.whatsapp.as_ref())
            {
                let targets = wa_cfg.allowed_numbers.clone();
                if targets.is_empty() {
                    tracing::warn!(
                        "WhatsApp startup announcement skipped: channels.whatsapp.allowed_numbers is empty"
                    );
                } else {
                    let text = announce_text.clone();
                    tokio::spawn(async move {
                        for number in targets {
                            match adapter
                                .send(ironclad_channels::OutboundMessage {
                                    content: text.clone(),
                                    recipient_id: number.clone(),
                                    metadata: None,
                                })
                                .await
                            {
                                Ok(()) => {
                                    tracing::info!(recipient = %number, "whatsapp startup announcement sent")
                                }
                                Err(e) => {
                                    tracing::warn!(recipient = %number, error = %e, "whatsapp startup announcement failed")
                                }
                            }
                        }
                    });
                }
            } else {
                tracing::warn!(
                    "WhatsApp startup announcement requested but whatsapp channel is not enabled/configured"
                );
            }
        }

        if startup_announce_channels.iter().any(|c| c == "signal") {
            if let (Some(adapter), Some(sig_cfg)) =
                (state.signal.clone(), config.channels.signal.as_ref())
            {
                let targets = sig_cfg.allowed_numbers.clone();
                if targets.is_empty() {
                    tracing::warn!(
                        "Signal startup announcement skipped: channels.signal.allowed_numbers is empty"
                    );
                } else {
                    let text = announce_text.clone();
                    tokio::spawn(async move {
                        for number in targets {
                            match adapter
                                .send(ironclad_channels::OutboundMessage {
                                    content: text.clone(),
                                    recipient_id: number.clone(),
                                    metadata: None,
                                })
                                .await
                            {
                                Ok(()) => {
                                    tracing::info!(recipient = %number, "signal startup announcement sent")
                                }
                                Err(e) => {
                                    tracing::warn!(recipient = %number, error = %e, "signal startup announcement failed")
                                }
                            }
                        }
                    });
                }
            } else {
                tracing::warn!(
                    "Signal startup announcement requested but signal channel is not enabled/configured"
                );
            }
        }

        for ch in &startup_announce_channels {
            if ch != "telegram" && ch != "whatsapp" && ch != "signal" {
                tracing::warn!(
                    channel = %ch,
                    "startup announcement requested for channel without recipient mapping support"
                );
            }
        }
    }

    if state.telegram.is_some() {
        let use_polling = config
            .channels
            .telegram
            .as_ref()
            .map(|c| !c.webhook_mode)
            .unwrap_or(true);
        if use_polling {
            let poll_state = state.clone();
            tokio::spawn(async move {
                api::telegram_poll_loop(poll_state).await;
            });
        }
    }
    if state.discord.is_some() {
        let poll_state = state.clone();
        tokio::spawn(async move {
            api::discord_poll_loop(poll_state).await;
        });
    }
    if state.signal.is_some() {
        let poll_state = state.clone();
        tokio::spawn(async move {
            api::signal_poll_loop(poll_state).await;
        });
    }
    if state.email.is_some() {
        let poll_state = state.clone();
        tokio::spawn(async move {
            api::email_poll_loop(poll_state).await;
        });
    }

    let auth_layer = ApiKeyLayer::new(config.server.api_key.clone());
    let local_origin = format!("http://{}:{}", config.server.bind, config.server.port);
    let origin_header = local_origin
        .parse::<axum::http::HeaderValue>()
        .unwrap_or_else(|e| {
            tracing::warn!(
                origin = %local_origin,
                error = %e,
                "CORS origin failed to parse, falling back to 127.0.0.1 loopback"
            );
            axum::http::HeaderValue::from_static("http://127.0.0.1:3000")
        });
    let cors = CorsLayer::new()
        .allow_origin(origin_header)
        .allow_methods([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::PUT,
            axum::http::Method::DELETE,
        ])
        .allow_headers([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::HeaderName::from_static("x-api-key"),
        ]);
    let authed_routes = build_router(state.clone()).layer(auth_layer);

    // WebSocket route handles its own auth (header OR ticket) so it
    // lives outside the API-key middleware layer.
    let ws_routes = axum::Router::new().route(
        "/ws",
        ws_route(
            event_bus.clone(),
            state.ws_tickets.clone(),
            config.server.api_key.clone(),
        ),
    );

    // MCP protocol endpoint uses bearer token auth (same API key).
    // Lives outside the main API-key middleware because the MCP transport
    // service (StreamableHttpService) needs its own route handling.
    let mcp_routes = build_mcp_router(&state, config.server.api_key.clone());

    let public_routes = build_public_router(state);

    let app = authed_routes
        .merge(ws_routes)
        .merge(mcp_routes)
        .merge(public_routes)
        .layer(cors)
        .layer(rate_limiter);
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    const BOOTSTRAP_CONFIG: &str = r#"
[agent]
name = "Ironclad"
id = "ironclad-test"

[server]
port = 18789
bind = "127.0.0.1"

[database]
path = ":memory:"

[models]
primary = "ollama/qwen3:8b"
"#;

    #[tokio::test]
    async fn bootstrap_with_memory_db_succeeds() {
        let config = IroncladConfig::from_str(BOOTSTRAP_CONFIG).expect("parse config");
        let result = bootstrap(config).await;
        assert!(
            result.is_ok(),
            "bootstrap with :memory: should succeed: {:?}",
            result.err()
        );
    }

    #[test]
    fn cleanup_old_logs_no_panic_on_missing_dir() {
        let dir = std::path::Path::new("/tmp/ironclad-test-nonexistent-dir-cleanup");
        cleanup_old_logs(dir, 30);
    }

    #[test]
    fn cleanup_old_logs_keeps_recent_logs() {
        let dir = tempfile::tempdir().unwrap();
        let recent = dir.path().join("recent.log");
        std::fs::write(&recent, "fresh log").unwrap();

        cleanup_old_logs(dir.path(), 30);
        assert!(recent.exists(), "recent logs should remain");
    }

    #[test]
    fn cleanup_old_logs_ignores_non_log_files() {
        let dir = tempfile::tempdir().unwrap();
        let txt = dir.path().join("data.txt");
        std::fs::write(&txt, "keep me").unwrap();

        cleanup_old_logs(dir.path(), 0);
        assert!(txt.exists(), "non-log files should not be deleted");
    }

    #[test]
    fn cleanup_old_logs_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        cleanup_old_logs(dir.path(), 1);
    }

    #[test]
    fn taskable_subagent_roles_are_strict() {
        assert!(is_taskable_subagent_role("subagent"));
        assert!(is_taskable_subagent_role("specialist"));
        assert!(is_taskable_subagent_role("SubAgent"));
        assert!(!is_taskable_subagent_role("model-proxy"));
    }

    struct EnvGuard {
        key: &'static str,
        old: Option<OsString>,
    }

    impl EnvGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let old = std::env::var_os(key);
            // SAFETY: test-local env change restored on Drop.
            unsafe { std::env::set_var(key, value) };
            Self { key, old }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            if let Some(v) = &self.old {
                // SAFETY: restoring previous env state.
                unsafe { std::env::set_var(self.key, v) };
            } else {
                // SAFETY: restoring previous env state.
                unsafe { std::env::remove_var(self.key) };
            }
        }
    }

    fn env_lock() -> &'static Mutex<()> {
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
    }

    #[test]
    fn resolve_token_prefers_keystore_reference_then_env_then_empty() {
        let _lock = env_lock().lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let keystore_path = dir.path().join("keystore.enc");
        let keystore = ironclad_core::keystore::Keystore::new(keystore_path);
        keystore.unlock("pw").unwrap();
        keystore.set("telegram_bot_token", "from_keystore").unwrap();

        let _env = EnvGuard::set("TEST_TELEGRAM_TOKEN", "from_env");
        let token = resolve_token(
            &Some("keystore:telegram_bot_token".to_string()),
            "TEST_TELEGRAM_TOKEN",
            &keystore,
        );
        assert_eq!(token, "from_keystore");

        let fallback = resolve_token(
            &Some("keystore:missing".to_string()),
            "TEST_TELEGRAM_TOKEN",
            &keystore,
        );
        assert_eq!(fallback, "from_env");

        let empty = resolve_token(&None, "", &keystore);
        assert!(empty.is_empty());
    }

    #[test]
    fn cleanup_old_logs_can_prune_when_window_is_zero_days() {
        let dir = tempfile::tempdir().unwrap();
        let old_log = dir.path().join("old.log");
        std::fs::write(&old_log, "stale").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        cleanup_old_logs(dir.path(), 0);
        assert!(!old_log.exists());
    }
}
