//! Platform-specific polling loops for Telegram, Discord, Signal, and Email.

use std::sync::Arc;

use ironclad_channels::ChannelAdapter;

use super::AppState;

pub(crate) const CHANNEL_PROCESSING_ERROR_REPLY: &str =
    "I hit an internal processing error while handling that message. Please retry in a moment.";

pub async fn telegram_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));

    let adapter = match &state.telegram {
        Some(a) => a.clone(),
        None => return,
    };

    tracing::info!("Telegram long-poll loop started");
    let mut consecutive_auth_failures: u32 = 0;

    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                consecutive_auth_failures = 0;
                state.channel_router.record_received("telegram").await;
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                let inbound_for_error = inbound.clone();
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = super::process_channel_message(&state, inbound).await {
                        state
                            .channel_router
                            .record_processing_error("telegram", e.clone())
                            .await;
                        let chat_id = super::resolve_channel_chat_id(&inbound_for_error);
                        if let Err(send_err) = state
                            .channel_router
                            .send_reply(
                                "telegram",
                                &chat_id,
                                CHANNEL_PROCESSING_ERROR_REPLY.to_string(),
                            )
                            .await
                        {
                            tracing::warn!(
                                error = %send_err,
                                "failed to send Telegram processing failure reply"
                            );
                        }
                        tracing::error!(error = %e, "Telegram message processing failed");
                    }
                });
            }
            Ok(None) => {
                consecutive_auth_failures = 0;
            }
            Err(e) => {
                let err_text = e.to_string();
                let looks_like_auth = err_text.contains("Telegram API 404")
                    || err_text.contains("Telegram API 401")
                    || err_text
                        .to_ascii_lowercase()
                        .contains("invalid/revoked bot token");
                if looks_like_auth {
                    consecutive_auth_failures = consecutive_auth_failures.saturating_add(1);
                    let backoff = if consecutive_auth_failures < 3 {
                        15
                    } else if consecutive_auth_failures < 10 {
                        30
                    } else {
                        60
                    };
                    if consecutive_auth_failures == 1
                        || consecutive_auth_failures.is_multiple_of(10)
                    {
                        tracing::error!(
                            error = %e,
                            failures = consecutive_auth_failures,
                            "Telegram poll authentication failed (likely invalid/revoked token). Repair with: `ironclad keystore set telegram_bot_token \"<TOKEN>\"` then restart."
                        );
                    } else {
                        tracing::warn!(
                            error = %e,
                            failures = consecutive_auth_failures,
                            "Telegram auth failure persists; backing off"
                        );
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(backoff)).await;
                } else {
                    consecutive_auth_failures = 0;
                    tracing::error!(error = %e, "Telegram poll error, backing off 5s");
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                }
            }
        }
    }
}

pub async fn discord_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.discord {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Discord inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                state.channel_router.record_received("discord").await;
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = super::process_channel_message(&state, inbound).await {
                        state
                            .channel_router
                            .record_processing_error("discord", e.clone())
                            .await;
                        tracing::error!(error = %e, "Discord message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            Err(e) => {
                tracing::error!(error = %e, "Discord inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

pub async fn signal_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.signal {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Signal inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                state.channel_router.record_received("signal").await;
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = super::process_channel_message(&state, inbound).await {
                        state
                            .channel_router
                            .record_processing_error("signal", e.clone())
                            .await;
                        tracing::error!(error = %e, "Signal message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_millis(300)).await,
            Err(e) => {
                tracing::error!(error = %e, "Signal inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}

pub async fn email_poll_loop(state: AppState) {
    static CHANNEL_SEMAPHORE: std::sync::LazyLock<Arc<tokio::sync::Semaphore>> =
        std::sync::LazyLock::new(|| Arc::new(tokio::sync::Semaphore::new(8)));
    let adapter = match &state.email {
        Some(a) => a.clone(),
        None => return,
    };
    tracing::info!("Email inbound loop started");
    loop {
        match adapter.recv().await {
            Ok(Some(inbound)) => {
                state.channel_router.record_received("email").await;
                let state = state.clone();
                let semaphore = Arc::clone(&CHANNEL_SEMAPHORE);
                tokio::spawn(async move {
                    let _permit = match semaphore.acquire_owned().await {
                        Ok(p) => p,
                        Err(_) => return,
                    };
                    if let Err(e) = super::process_channel_message(&state, inbound).await {
                        state
                            .channel_router
                            .record_processing_error("email", e.clone())
                            .await;
                        tracing::error!(error = %e, "Email message processing failed");
                    }
                });
            }
            Ok(None) => tokio::time::sleep(std::time::Duration::from_secs(1)).await,
            Err(e) => {
                tracing::error!(error = %e, "Email inbound loop error, backing off 5s");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    }
}
