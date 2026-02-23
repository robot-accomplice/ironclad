use std::collections::HashMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, error, warn};

use ironclad_core::{IroncladError, Result};

use crate::delivery::DeliveryQueue;
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
                warn!(
                    channel = %channel_name,
                    error = %e,
                    "send failed, queuing for retry"
                );
                let mut channels = self.channels.lock().await;
                if let Some(entry) = channels.get_mut(channel_name) {
                    entry.status.last_error = Some(e.to_string());
                }
                self.delivery_queue
                    .enqueue(channel_name.to_string(), queued_msg)
                    .await;
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
                    warn!(
                        id = %item.id,
                        channel = %item.channel,
                        error = %e,
                        attempt = item.attempts + 1,
                        "retry failed, requeuing"
                    );
                    let mut channels = self.channels.lock().await;
                    if let Some(entry) = channels.get_mut(&item.channel) {
                        entry.status.last_error = Some(e.to_string());
                    }
                    self.delivery_queue
                        .requeue_failed(item, e.to_string())
                        .await;
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
}
