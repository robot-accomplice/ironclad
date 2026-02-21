pub mod api;
pub mod auth;
pub mod cli;
pub mod daemon;
pub mod dashboard;
pub mod plugins;
pub mod ws;

pub use api::{build_router, AppState};
pub use dashboard::{build_dashboard_html, dashboard_handler};
pub use ws::{EventBus, ws_route};

use std::sync::Arc;

use tokio::sync::RwLock;
use tower_http::cors::{Any, CorsLayer};

use auth::ApiKeyLayer;
use ironclad_agent::subagents::SubagentRegistry;
use ironclad_browser::Browser;
use ironclad_channels::ChannelAdapter;
use ironclad_channels::a2a::A2aProtocol;
use ironclad_channels::router::ChannelRouter;
use ironclad_channels::telegram::TelegramAdapter;
use ironclad_core::IroncladConfig;
use ironclad_core::personality;
use ironclad_db::Database;
use ironclad_llm::LlmService;
use ironclad_wallet::WalletService;

fn init_logging(config: &IroncladConfig) {
    use tracing_subscriber::fmt;
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    use tracing_subscriber::EnvFilter;

    let level = config.agent.log_level.as_str();
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(level));

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

        let stderr_layer = fmt::layer()
            .with_writer(std::io::stderr);

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
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(max_days) * 86400);

    let entries = match std::fs::read_dir(log_dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("log") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}

/// Builds the application state and router from config. Used by the binary and by tests.
pub async fn bootstrap(config: IroncladConfig) -> Result<axum::Router, Box<dyn std::error::Error>> {
    init_logging(&config);

    let workspace = &config.agent.workspace;
    let os = personality::load_os(workspace);
    let fw = personality::load_firmware(workspace);
    let operator = personality::load_operator(workspace);
    let directives = personality::load_directives(workspace);

    let soul_text = personality::compose_soul(
        os.as_ref(),
        fw.as_ref(),
        operator.as_ref(),
        directives.as_ref(),
    );

    if !soul_text.is_empty() {
        tracing::info!(
            personality = os.as_ref().map(|o| o.identity.name.as_str()).unwrap_or("none"),
            "Loaded personality files from workspace"
        );
    } else {
        tracing::info!("No personality files found in workspace, using defaults");
    }

    let db_path = config.database.path.to_string_lossy().to_string();
    let db = Database::new(&db_path)?;
    let llm = LlmService::new(&config);
    let wallet = WalletService::new(&config).await?;
    let a2a = A2aProtocol::new(config.a2a.clone());
    let plugin_registry = plugins::init_plugin_registry(&config.plugins).await;
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
                    ));
                    channel_router
                        .register(Arc::clone(&adapter) as Arc<dyn ChannelAdapter>)
                        .await;
                    tracing::info!("Telegram adapter registered");
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

    let state = AppState {
        db,
        config: Arc::new(RwLock::new(config.clone())),
        llm: Arc::new(RwLock::new(llm)),
        wallet: Arc::new(wallet),
        a2a: Arc::new(RwLock::new(a2a)),
        soul_text: Arc::new(soul_text),
        plugins: plugin_registry,
        browser,
        registry,
        event_bus: event_bus.clone(),
        channel_router,
        telegram,
    };

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
            .allow_methods(Any)
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    };
    let app = build_router(state)
        .route("/ws", ws_route(event_bus.clone()))
        .layer(auth_layer)
        .layer(cors);
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
        assert!(result.is_ok(), "bootstrap with :memory: should succeed: {:?}", result.err());
    }
}
