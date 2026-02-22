pub mod api;
pub mod auth;
pub mod cli;
pub mod daemon;
pub mod dashboard;
pub mod migrate;
pub mod plugins;
pub mod rate_limit;
pub mod ws;

pub use api::{AppState, PersonalityState, build_router};
pub use dashboard::{build_dashboard_html, dashboard_handler};
pub use ws::{EventBus, ws_route};

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use auth::ApiKeyLayer;
use ironclad_agent::policy::{AuthorityRule, CommandSafetyRule, PolicyEngine};
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
use ironclad_wallet::WalletService;
use rate_limit::GlobalRateLimitLayer;

fn init_logging(config: &IroncladConfig) {
    use tracing_subscriber::EnvFilter;
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let level = config.agent.log_level.as_str();
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level));

    let log_dir = &config.server.log_dir;
    if std::fs::create_dir_all(log_dir).is_ok() {
        let file_appender = tracing_appender::rolling::daily(log_dir, "ironclad.log");
        let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

        // Leak the guard so it lives for the entire process
        std::mem::forget(_guard);

        let file_layer = fmt::layer()
            .with_writer(non_blocking)
            .with_ansi(false)
            .json();

        let stderr_layer = fmt::layer().with_writer(std::io::stderr);

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .try_init();
    } else {
        let stderr_layer = fmt::layer().with_writer(std::io::stderr);
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

/// Builds the application state and router from config. Used by the binary and by tests.
pub async fn bootstrap(config: IroncladConfig) -> Result<axum::Router, Box<dyn std::error::Error>> {
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
    let llm = LlmService::new(&config)?;
    let wallet = WalletService::new(&config).await?;
    let a2a = A2aProtocol::new(config.a2a.clone());
    let plugin_registry = plugins::init_plugin_registry(&config.plugins).await;
    let mut policy_engine = PolicyEngine::new();
    policy_engine.add_rule(Box::new(AuthorityRule));
    policy_engine.add_rule(Box::new(CommandSafetyRule));
    let policy_engine = Arc::new(policy_engine);
    let browser = Arc::new(Browser::new(config.browser.clone()));
    let registry = Arc::new(SubagentRegistry::new(4, vec![]));
    let event_bus = EventBus::new(256);

    let channel_router = Arc::new(ChannelRouter::new());
    let telegram: Option<Arc<TelegramAdapter>> =
        if let Some(ref tg_config) = config.channels.telegram {
            if tg_config.enabled {
                let token = std::env::var(&tg_config.token_env).unwrap_or_default();
                if !token.is_empty() {
                    let adapter = Arc::new(TelegramAdapter::with_config(
                        token,
                        tg_config.poll_timeout_seconds,
                        tg_config.allowed_chat_ids.clone(),
                        tg_config.webhook_secret.clone(),
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
                        "Telegram enabled but token env var is empty"
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
                let token = std::env::var(&wa_config.token_env).unwrap_or_default();
                if !token.is_empty() && !wa_config.phone_number_id.is_empty() {
                    let adapter = Arc::new(WhatsAppAdapter::with_config(
                        token,
                        wa_config.phone_number_id.clone(),
                        wa_config.verify_token.clone(),
                        wa_config.allowed_numbers.clone(),
                        wa_config.app_secret.clone(),
                    ));
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

    let hmac_secret = {
        use rand::RngCore;
        let mut buf = vec![0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut buf);
        buf
    };

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
        started_at: std::time::Instant::now(),
    };

    // Start heartbeat daemon
    {
        let hb_wallet = Arc::clone(&state.wallet);
        let hb_db = state.db.clone();
        let daemon = ironclad_schedule::HeartbeatDaemon::new(60_000);
        tokio::spawn(async move {
            ironclad_schedule::run_heartbeat(daemon, hb_wallet, hb_db).await;
        });
        tracing::info!("Heartbeat daemon spawned (60s interval)");
    }

    // Start cron worker
    {
        let cron_db = state.db.clone();
        let instance_id = config.agent.id.clone();
        tokio::spawn(async move {
            ironclad_schedule::run_cron_worker(cron_db, instance_id).await;
        });
        tracing::info!("Cron worker spawned");
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

    let auth_layer = ApiKeyLayer::new(config.server.api_key.clone());
    let cors = if config.server.api_key.is_some() {
        let origin = format!("http://{}:{}", config.server.bind, config.server.port);
        CorsLayer::new()
            .allow_origin(
                origin
                    .parse::<axum::http::HeaderValue>()
                    .unwrap_or_else(|_| axum::http::HeaderValue::from_static("*")),
            )
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
            ])
    } else {
        CorsLayer::new()
            .allow_origin(Any)
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
            ])
    };
    let app = build_router(state)
        .route("/ws", ws_route(event_bus.clone()))
        .layer(auth_layer)
        .layer(cors)
        .layer(GlobalRateLimitLayer::new(100, Duration::from_secs(60)));
    Ok(app)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
