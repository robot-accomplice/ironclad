use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use ironclad_core::{IroncladError, Result};
use ironclad_db::Database;

use crate::delivery::{DeliveryItem, DeliveryQueue};
use crate::{ChannelAdapter, InboundMessage, OutboundMessage};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelStatus {
    pub name: String,
    pub connected: bool,
    pub messages_received: u64,
    pub messages_sent: u64,
    pub last_error: Option<String>,
    pub last_activity: Option<DateTime<Utc>>,
}

struct ChannelEntry {
    adapter: Arc<dyn ChannelAdapter>,
    status: ChannelStatus,
}

pub struct ChannelRouter {
    channels: Mutex<HashMap<String, ChannelEntry>>,
    delivery_queue: DeliveryQueue,
}

impl ChannelRouter {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            delivery_queue: DeliveryQueue::new(),
        }
    }

    pub async fn with_store(store: Database) -> Self {
        let delivery_queue = DeliveryQueue::with_store(store);
        delivery_queue.recover_from_store().await;
        Self {
            channels: Mutex::new(HashMap::new()),
            delivery_queue,
        }
    }

    pub async fn register(&self, adapter: Arc<dyn ChannelAdapter>) {
        let name = adapter.platform_name().to_string();
        let entry = ChannelEntry {
            adapter,
            status: ChannelStatus {
                name: name.clone(),
                connected: true,
                messages_received: 0,
                messages_sent: 0,
                last_error: None,
                last_activity: None,
            },
        };
        let mut channels = self.channels.lock().await;
        channels.insert(name, entry);
    }

    pub async fn poll_all(&self) -> Vec<(String, InboundMessage)> {
        let adapters: Vec<(String, Arc<dyn ChannelAdapter>)> = {
            let channels = self.channels.lock().await;
            channels
                .iter()
                .map(|(name, entry)| (name.clone(), Arc::clone(&entry.adapter)))
                .collect()
        };

        let mut received = Vec::new();

        for (name, adapter) in &adapters {
            match adapter.recv().await {
                Ok(Some(msg)) => {
                    debug!(channel = %name, msg_id = %msg.id, "received message");
                    let mut channels = self.channels.lock().await;
                    if let Some(entry) = channels.get_mut(name) {
                        entry.status.messages_received += 1;
                        entry.status.last_activity = Some(Utc::now());
                        entry.status.last_error = None;
                    }
                    received.push((name.clone(), msg));
                }
                Ok(None) => {}
                Err(e) => {
                    error!(channel = %name, error = %e, "channel recv error");
                    let mut channels = self.channels.lock().await;
                    if let Some(entry) = channels.get_mut(name) {
                        entry.status.last_error = Some(e.to_string());
                    }
                }
            }
        }

        received
    }

    pub async fn send_to(&self, channel_name: &str, msg: OutboundMessage) -> Result<()> {
        let adapter = {
            let channels = self.channels.lock().await;
            channels
                .get(channel_name)
                .map(|e| Arc::clone(&e.adapter))
                .ok_or_else(|| {
                    IroncladError::Network(format!("channel not found: {channel_name}"))
                })?
        };

        let queued_msg = msg.clone();
        match adapter.send(msg).await {
            Ok(()) => {
                let mut channels = self.channels.lock().await;
                if let Some(entry) = channels.get_mut(channel_name) {
                    entry.status.messages_sent += 1;
                    entry.status.last_activity = Some(Utc::now());
                    entry.status.last_error = None;
                }
            }
            Err(e) => {
                let err_str = e.to_string();
                let mut channels = self.channels.lock().await;
                if let Some(entry) = channels.get_mut(channel_name) {
                    entry.status.last_error = Some(err_str.clone());
                }
                if DeliveryItem::is_permanent_error(&err_str) {
                    warn!(
                        channel = %channel_name,
                        error = %err_str,
                        "send failed with permanent error, not retrying"
                    );
                } else {
                    warn!(
                        channel = %channel_name,
                        error = %err_str,
                        "send failed, queuing for retry"
                    );
                    self.delivery_queue
                        .enqueue(channel_name.to_string(), queued_msg)
                        .await;
                }
            }
        }

        Ok(())
    }

    pub async fn send_reply(
        &self,
        platform: &str,
        recipient_id: &str,
        content: String,
    ) -> Result<()> {
        let msg = OutboundMessage {
            content,
            recipient_id: recipient_id.to_string(),
            metadata: None,
        };
        self.send_to(platform, msg).await
    }

    pub async fn record_received(&self, channel_name: &str) {
        let mut channels = self.channels.lock().await;
        if let Some(entry) = channels.get_mut(channel_name) {
            entry.status.messages_received += 1;
            entry.status.last_activity = Some(Utc::now());
        }
    }

    pub async fn record_processing_error(&self, channel_name: &str, error: String) {
        let mut channels = self.channels.lock().await;
        if let Some(entry) = channels.get_mut(channel_name) {
            entry.status.last_error = Some(error);
            entry.status.last_activity = Some(Utc::now());
        }
    }

    pub async fn drain_retry_queue(&self) {
        while let Some(item) = self.delivery_queue.next_ready().await {
            let adapter = {
                let channels = self.channels.lock().await;
                channels.get(&item.channel).map(|e| Arc::clone(&e.adapter))
            };

            let Some(adapter) = adapter else {
                warn!(channel = %item.channel, id = %item.id, "channel gone, dead-lettering item");
                self.delivery_queue
                    .requeue_failed(item, "channel no longer registered".into())
                    .await;
                continue;
            };

            let msg = OutboundMessage {
                content: item.content.clone(),
                recipient_id: item.recipient_id.clone(),
                metadata: None,
            };

            match adapter.send(msg).await {
                Ok(()) => {
                    debug!(id = %item.id, channel = %item.channel, "retry delivered");
                    self.delivery_queue.mark_success(&item.id).await;
                    let mut channels = self.channels.lock().await;
                    if let Some(entry) = channels.get_mut(&item.channel) {
                        entry.status.messages_sent += 1;
                        entry.status.last_activity = Some(Utc::now());
                        entry.status.last_error = None;
                    }
                }
                Err(e) => {
                    let err_str = e.to_string();
                    let mut channels = self.channels.lock().await;
                    if let Some(entry) = channels.get_mut(&item.channel) {
                        entry.status.last_error = Some(err_str.clone());
                    }
                    if DeliveryItem::is_permanent_error(&err_str) {
                        warn!(
                            id = %item.id,
                            channel = %item.channel,
                            error = %err_str,
                            "permanent error, dead-lettering"
                        );
                    } else {
                        warn!(
                            id = %item.id,
                            channel = %item.channel,
                            error = %err_str,
                            attempt = item.attempts + 1,
                            "retry failed, requeuing"
                        );
                    }
                    self.delivery_queue.requeue_failed(item, err_str).await;
                }
            }
        }
    }

    pub fn delivery_queue(&self) -> &DeliveryQueue {
        &self.delivery_queue
    }

    pub async fn channel_status(&self) -> Vec<ChannelStatus> {
        let channels = self.channels.lock().await;
        channels.values().map(|e| e.status.clone()).collect()
    }

    pub async fn channel_names(&self) -> Vec<String> {
        let channels = self.channels.lock().await;
        channels.keys().cloned().collect()
    }

    pub async fn channel_count(&self) -> usize {
        let channels = self.channels.lock().await;
        channels.len()
    }

    pub async fn dead_letters(&self, max_items: usize) -> Vec<DeliveryItem> {
        let mut merged = self.delivery_queue.dead_letters_from_store(max_items);
        if merged.is_empty() {
            merged = self.delivery_queue.dead_letters().await;
        }
        merged
    }

    pub async fn replay_dead_letter(&self, id: &str) -> bool {
        if self.delivery_queue.replay_dead_letter_in_store(id) {
            self.delivery_queue.recover_from_store().await;
            return true;
        }
        self.delivery_queue.replay_dead_letter_in_memory(id).await
    }

    pub async fn is_registered(&self, name: &str) -> bool {
        let channels = self.channels.lock().await;
        channels.contains_key(name)
    }
}

impl Default for ChannelRouter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ironclad_core::IroncladError;
    use ironclad_db::Database;

    struct MockAdapter {
        name: String,
        message: Option<InboundMessage>,
    }

    impl MockAdapter {
        fn new(name: &str) -> Self {
            Self {
                name: name.into(),
                message: None,
            }
        }
        fn with_message(name: &str, content: &str) -> Self {
            Self {
                name: name.into(),
                message: Some(InboundMessage {
                    id: "mock-1".into(),
                    platform: name.into(),
                    sender_id: "user-1".into(),
                    content: content.into(),
                    timestamp: Utc::now(),
                    metadata: None,
                }),
            }
        }
    }

    #[async_trait]
    impl ChannelAdapter for MockAdapter {
        fn platform_name(&self) -> &str {
            &self.name
        }
        async fn recv(&self) -> Result<Option<InboundMessage>> {
            Ok(self.message.clone())
        }
        async fn send(&self, _msg: OutboundMessage) -> Result<()> {
            Ok(())
        }
    }

    struct PermanentFailAdapter {
        name: String,
    }

    #[async_trait]
    impl ChannelAdapter for PermanentFailAdapter {
        fn platform_name(&self) -> &str {
            &self.name
        }

        async fn recv(&self) -> Result<Option<InboundMessage>> {
            Ok(None)
        }

        async fn send(&self, _msg: OutboundMessage) -> Result<()> {
            Err(IroncladError::Network(
                "Telegram API 403 Forbidden: bot was blocked by the user".into(),
            ))
        }
    }

    #[tokio::test]
    async fn register_and_count() {
        let router = ChannelRouter::new();
        assert_eq!(router.channel_count().await, 0);
        router.register(Arc::new(MockAdapter::new("test"))).await;
        assert_eq!(router.channel_count().await, 1);
        assert!(router.is_registered("test").await);
        assert!(!router.is_registered("other").await);
    }

    #[tokio::test]
    async fn poll_all_receives_messages() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::with_message("ch1", "hello")))
            .await;
        router.register(Arc::new(MockAdapter::new("ch2"))).await;

        let msgs = router.poll_all().await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].0, "ch1");
        assert_eq!(msgs[0].1.content, "hello");
    }

    #[tokio::test]
    async fn send_to_known_channel() {
        let router = ChannelRouter::new();
        router.register(Arc::new(MockAdapter::new("test"))).await;
        let msg = OutboundMessage {
            content: "hi".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        router.send_to("test", msg).await.unwrap();
        let statuses = router.channel_status().await;
        assert_eq!(statuses[0].messages_sent, 1);
    }

    #[tokio::test]
    async fn send_to_unknown_channel_fails() {
        let router = ChannelRouter::new();
        let msg = OutboundMessage {
            content: "hi".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        let result = router.send_to("nonexistent", msg).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn channel_names_list() {
        let router = ChannelRouter::new();
        router.register(Arc::new(MockAdapter::new("a"))).await;
        router.register(Arc::new(MockAdapter::new("b"))).await;
        let mut names = router.channel_names().await;
        names.sort();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[tokio::test]
    async fn send_reply_convenience() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::new("telegram")))
            .await;
        router
            .send_reply("telegram", "chat123", "hello".into())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn poll_updates_status() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::with_message("ch", "msg")))
            .await;
        router.poll_all().await;
        let statuses = router.channel_status().await;
        assert_eq!(statuses[0].messages_received, 1);
        assert!(statuses[0].last_activity.is_some());
    }

    #[tokio::test]
    async fn record_received_updates_status() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::new("telegram")))
            .await;
        router.record_received("telegram").await;
        let statuses = router.channel_status().await;
        assert_eq!(statuses[0].messages_received, 1);
        assert!(statuses[0].last_activity.is_some());
    }

    #[tokio::test]
    async fn record_processing_error_updates_status() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::new("telegram")))
            .await;
        router
            .record_processing_error("telegram", "pipeline failed".into())
            .await;
        let statuses = router.channel_status().await;
        assert_eq!(statuses[0].last_error.as_deref(), Some("pipeline failed"));
        assert!(statuses[0].last_activity.is_some());
    }

    #[tokio::test]
    async fn with_store_recovers_queue_state() {
        let db = Database::new(":memory:").expect("db");
        let router = ChannelRouter::with_store(db.clone()).await;
        let msg = OutboundMessage {
            content: "queued".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        router
            .delivery_queue()
            .enqueue("telegram".into(), msg)
            .await;

        let recovered = ChannelRouter::with_store(db).await;
        assert_eq!(recovered.delivery_queue().queue_size().await, 1);
    }

    #[tokio::test]
    async fn send_to_permanent_error_does_not_enqueue_retry() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(PermanentFailAdapter {
                name: "telegram".into(),
            }))
            .await;

        let msg = OutboundMessage {
            content: "hello".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };

        let result = router.send_to("telegram", msg).await;
        assert!(
            result.is_ok(),
            "router should swallow send failure into status"
        );
        assert_eq!(
            router.delivery_queue().queue_size().await,
            0,
            "permanent errors must not be queued for retry"
        );
    }

    struct TransientFailAdapter {
        name: String,
    }

    #[async_trait]
    impl ChannelAdapter for TransientFailAdapter {
        fn platform_name(&self) -> &str {
            &self.name
        }

        async fn recv(&self) -> Result<Option<InboundMessage>> {
            Ok(None)
        }

        async fn send(&self, _msg: OutboundMessage) -> Result<()> {
            Err(IroncladError::Network("connection timeout".into()))
        }
    }

    struct RecvErrorAdapter;

    #[async_trait]
    impl ChannelAdapter for RecvErrorAdapter {
        fn platform_name(&self) -> &str {
            "error_channel"
        }

        async fn recv(&self) -> Result<Option<InboundMessage>> {
            Err(IroncladError::Network("recv failed".into()))
        }

        async fn send(&self, _msg: OutboundMessage) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn send_to_transient_error_enqueues_retry() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(TransientFailAdapter {
                name: "telegram".into(),
            }))
            .await;

        let msg = OutboundMessage {
            content: "retry me".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        router.send_to("telegram", msg).await.unwrap();
        assert_eq!(
            router.delivery_queue().queue_size().await,
            1,
            "transient errors should be queued for retry"
        );
    }

    #[tokio::test]
    async fn send_to_transient_error_records_last_error() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(TransientFailAdapter { name: "ch".into() }))
            .await;

        let msg = OutboundMessage {
            content: "fail".into(),
            recipient_id: "r1".into(),
            metadata: None,
        };
        router.send_to("ch", msg).await.unwrap();
        let statuses = router.channel_status().await;
        assert!(statuses[0].last_error.is_some());
        assert!(statuses[0].last_error.as_ref().unwrap().contains("timeout"));
    }

    #[tokio::test]
    async fn poll_all_records_recv_error() {
        let router = ChannelRouter::new();
        router.register(Arc::new(RecvErrorAdapter)).await;
        let msgs = router.poll_all().await;
        assert!(msgs.is_empty());
        let statuses = router.channel_status().await;
        assert!(statuses[0].last_error.is_some());
    }

    #[tokio::test]
    async fn default_router() {
        let router = ChannelRouter::default();
        assert_eq!(router.channel_count().await, 0);
    }

    #[tokio::test]
    async fn channel_status_fields() {
        let status = ChannelStatus {
            name: "test".into(),
            connected: true,
            messages_received: 5,
            messages_sent: 3,
            last_error: None,
            last_activity: Some(Utc::now()),
        };
        assert_eq!(status.name, "test");
        assert!(status.connected);
        assert_eq!(status.messages_received, 5);
        assert_eq!(status.messages_sent, 3);
        assert!(status.last_error.is_none());
        assert!(status.last_activity.is_some());
    }

    #[tokio::test]
    async fn channel_status_serde() {
        let status = ChannelStatus {
            name: "telegram".into(),
            connected: true,
            messages_received: 10,
            messages_sent: 5,
            last_error: Some("timeout".into()),
            last_activity: None,
        };
        let json = serde_json::to_string(&status).unwrap();
        let back: ChannelStatus = serde_json::from_str(&json).unwrap();
        assert_eq!(back.name, "telegram");
        assert_eq!(back.messages_received, 10);
        assert_eq!(back.last_error.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn record_received_nonexistent_channel_noop() {
        let router = ChannelRouter::new();
        // Should not panic
        router.record_received("nonexistent").await;
    }

    #[tokio::test]
    async fn record_processing_error_nonexistent_channel_noop() {
        let router = ChannelRouter::new();
        // Should not panic
        router
            .record_processing_error("nonexistent", "err".into())
            .await;
    }

    #[tokio::test]
    async fn drain_retry_queue_empty_is_noop() {
        let router = ChannelRouter::new();
        router.drain_retry_queue().await;
        // No assertions needed; just verify no panic
    }

    #[tokio::test]
    async fn drain_retry_queue_with_missing_channel_dead_letters() {
        let router = ChannelRouter::new();
        // Enqueue something for a channel that's not registered
        router
            .delivery_queue()
            .enqueue(
                "gone_channel".into(),
                OutboundMessage {
                    content: "orphan".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;
        router.drain_retry_queue().await;
        // The item should be dead-lettered since channel is gone
    }

    #[tokio::test]
    async fn dead_letters_empty() {
        let router = ChannelRouter::new();
        let dead = router.dead_letters(10).await;
        assert!(dead.is_empty());
    }

    #[tokio::test]
    async fn replay_dead_letter_nonexistent_returns_false() {
        let router = ChannelRouter::new();
        let replayed = router.replay_dead_letter("nonexistent-id").await;
        assert!(!replayed);
    }

    #[tokio::test]
    async fn is_registered_after_register() {
        let router = ChannelRouter::new();
        assert!(!router.is_registered("test").await);
        router.register(Arc::new(MockAdapter::new("test"))).await;
        assert!(router.is_registered("test").await);
    }

    #[tokio::test]
    async fn send_reply_to_nonexistent_channel() {
        let router = ChannelRouter::new();
        let result = router
            .send_reply("nonexistent", "user1", "hello".into())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn poll_all_clears_error_on_success() {
        let router = ChannelRouter::new();
        router
            .register(Arc::new(MockAdapter::with_message("ch", "hello")))
            .await;
        // First, set an error
        router
            .record_processing_error("ch", "previous error".into())
            .await;
        // Poll should clear error on success
        router.poll_all().await;
        let statuses = router.channel_status().await;
        assert!(
            statuses[0].last_error.is_none(),
            "error should be cleared after successful recv"
        );
    }

    #[tokio::test]
    async fn delivery_queue_accessor() {
        let router = ChannelRouter::new();
        assert_eq!(router.delivery_queue().queue_size().await, 0);
    }

    #[tokio::test]
    async fn drain_retry_queue_successful_delivery() {
        let router = ChannelRouter::new();
        // Register a mock adapter that succeeds
        router.register(Arc::new(MockAdapter::new("test_ch"))).await;

        // Enqueue a delivery item
        router
            .delivery_queue()
            .enqueue(
                "test_ch".into(),
                OutboundMessage {
                    content: "retry me".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;

        // Drain should succeed
        router.drain_retry_queue().await;

        // Queue should be empty after successful delivery
        assert_eq!(router.delivery_queue().queue_size().await, 0);

        // Verify messages_sent was incremented
        let statuses = router.channel_status().await;
        let ch_status = statuses.iter().find(|s| s.name == "test_ch").unwrap();
        assert!(ch_status.messages_sent > 0);
        assert!(ch_status.last_error.is_none());
    }

    #[tokio::test]
    async fn drain_retry_queue_transient_failure() {
        let router = ChannelRouter::new();
        // Register an adapter that fails with a transient error
        router
            .register(Arc::new(TransientFailAdapter {
                name: "fail_ch".into(),
            }))
            .await;

        router
            .delivery_queue()
            .enqueue(
                "fail_ch".into(),
                OutboundMessage {
                    content: "will fail".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;

        router.drain_retry_queue().await;

        // Verify last_error was set
        let statuses = router.channel_status().await;
        let ch_status = statuses.iter().find(|s| s.name == "fail_ch").unwrap();
        assert!(ch_status.last_error.is_some());
    }

    #[tokio::test]
    async fn drain_retry_queue_permanent_failure_dead_letters() {
        let router = ChannelRouter::new();
        // Register an adapter that fails with a permanent error
        router
            .register(Arc::new(PermanentFailAdapter {
                name: "perm_ch".into(),
            }))
            .await;

        router
            .delivery_queue()
            .enqueue(
                "perm_ch".into(),
                OutboundMessage {
                    content: "permanent fail".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;

        router.drain_retry_queue().await;

        // Verify last_error was set
        let statuses = router.channel_status().await;
        let ch_status = statuses.iter().find(|s| s.name == "perm_ch").unwrap();
        assert!(ch_status.last_error.is_some());
    }

    #[tokio::test]
    async fn dead_letters_returns_from_memory() {
        let router = ChannelRouter::new();
        // Enqueue then force to dead letter
        router
            .delivery_queue()
            .enqueue(
                "ch".into(),
                OutboundMessage {
                    content: "dead".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;
        let item = router.delivery_queue().next_ready().await.unwrap();
        router
            .delivery_queue()
            .requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;

        let dead = router.dead_letters(10).await;
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].content, "dead");
    }

    #[tokio::test]
    async fn replay_dead_letter_from_memory() {
        let router = ChannelRouter::new();
        router
            .delivery_queue()
            .enqueue(
                "ch".into(),
                OutboundMessage {
                    content: "replay".into(),
                    recipient_id: "r1".into(),
                    metadata: None,
                },
            )
            .await;
        let item = router.delivery_queue().next_ready().await.unwrap();
        let item_id = item.id.clone();
        router
            .delivery_queue()
            .requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;

        let replayed = router.replay_dead_letter(&item_id).await;
        assert!(replayed);
    }

    #[tokio::test]
    async fn channel_names() {
        let router = ChannelRouter::new();
        router.register(Arc::new(MockAdapter::new("alpha"))).await;
        router.register(Arc::new(MockAdapter::new("beta"))).await;
        let mut names = router.channel_names().await;
        names.sort();
        assert_eq!(names, vec!["alpha", "beta"]);
    }
}
