use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tracing::{debug, warn};
use uuid::Uuid;

use crate::OutboundMessage;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeliveryStatus {
    Pending,
    InFlight,
    Delivered,
    Failed,
    DeadLetter,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeliveryItem {
    pub id: String,
    pub channel: String,
    pub recipient_id: String,
    pub content: String,
    pub status: DeliveryStatus,
    pub attempts: u32,
    pub max_attempts: u32,
    pub next_retry_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
    pub last_error: Option<String>,
}

impl DeliveryItem {
    pub fn new(channel: String, msg: OutboundMessage) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            channel,
            recipient_id: msg.recipient_id,
            content: msg.content,
            status: DeliveryStatus::Pending,
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            created_at: Utc::now(),
            last_error: None,
        }
    }

    pub fn backoff_delay(attempt: u32) -> Duration {
        match attempt {
            0 => Duration::seconds(0),
            1 => Duration::seconds(1),
            2 => Duration::seconds(5),
            3 => Duration::seconds(30),
            4 => Duration::minutes(5),
            _ => Duration::minutes(15),
        }
    }

    pub fn mark_failed(&mut self, error: String) {
        self.attempts += 1;
        self.last_error = Some(error);
        if self.attempts >= self.max_attempts {
            self.status = DeliveryStatus::DeadLetter;
        } else {
            self.status = DeliveryStatus::Pending;
            self.next_retry_at = Utc::now() + Self::backoff_delay(self.attempts);
        }
    }

    pub fn mark_delivered(&mut self) {
        self.status = DeliveryStatus::Delivered;
        self.attempts += 1;
    }

    pub fn is_ready(&self) -> bool {
        self.status == DeliveryStatus::Pending && Utc::now() >= self.next_retry_at
    }
}

pub struct DeliveryQueue {
    pub(crate) items: Arc<Mutex<VecDeque<DeliveryItem>>>,
    dead_letters: Arc<Mutex<Vec<DeliveryItem>>>,
}

impl DeliveryQueue {
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(VecDeque::new())),
            dead_letters: Arc::new(Mutex::new(Vec::new())),
        }
    }

    pub async fn enqueue(&self, channel: String, msg: OutboundMessage) -> String {
        let item = DeliveryItem::new(channel, msg);
        let id = item.id.clone();
        debug!(id = %id, "enqueuing delivery item");
        let mut items = self.items.lock().await;
        items.push_back(item);
        id
    }

    pub async fn next_ready(&self) -> Option<DeliveryItem> {
        let mut items = self.items.lock().await;
        let pos = items.iter().position(|item| item.is_ready())?;
        let mut item = items.remove(pos)?;
        item.status = DeliveryStatus::InFlight;
        Some(item)
    }

    pub async fn mark_success(&self, id: &str) {
        debug!(id, "delivery succeeded");
    }

    pub async fn requeue_failed(&self, mut item: DeliveryItem, error: String) {
        item.mark_failed(error.clone());
        if item.status == DeliveryStatus::DeadLetter {
            warn!(id = %item.id, attempts = item.attempts, "moving to dead letter");
            let mut dead = self.dead_letters.lock().await;
            dead.push(item);
        } else {
            debug!(id = %item.id, attempt = item.attempts, "requeuing with backoff");
            let mut items = self.items.lock().await;
            items.push_back(item);
        }
    }

    pub async fn pending_count(&self) -> usize {
        let items = self.items.lock().await;
        items
            .iter()
            .filter(|i| i.status == DeliveryStatus::Pending)
            .count()
    }

    pub async fn dead_letter_count(&self) -> usize {
        let dead = self.dead_letters.lock().await;
        dead.len()
    }

    pub async fn queue_size(&self) -> usize {
        let items = self.items.lock().await;
        items.len()
    }

    pub async fn dead_letters(&self) -> Vec<DeliveryItem> {
        let dead = self.dead_letters.lock().await;
        dead.clone()
    }
}

impl Default for DeliveryQueue {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_msg(content: &str) -> OutboundMessage {
        OutboundMessage {
            content: content.into(),
            recipient_id: "r1".into(),
            metadata: None,
        }
    }

    #[tokio::test]
    async fn enqueue_and_dequeue() {
        let q = DeliveryQueue::new();
        let id = q.enqueue("telegram".into(), test_msg("hello")).await;
        assert!(!id.is_empty());
        assert_eq!(q.queue_size().await, 1);

        let item = q.next_ready().await;
        assert!(item.is_some());
        let item = item.unwrap();
        assert_eq!(item.content, "hello");
        assert_eq!(item.channel, "telegram");
        assert_eq!(item.status, DeliveryStatus::InFlight);
    }

    #[tokio::test]
    async fn empty_queue_returns_none() {
        let q = DeliveryQueue::new();
        assert!(q.next_ready().await.is_none());
    }

    #[tokio::test]
    async fn requeue_with_backoff() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg")).await;
        let item = q.next_ready().await.unwrap();
        q.requeue_failed(item, "timeout".into()).await;
        assert_eq!(q.queue_size().await, 1);
        assert_eq!(q.dead_letter_count().await, 0);
    }

    #[tokio::test]
    async fn dead_letter_after_max_attempts() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg")).await;

        for _i in 0..5 {
            // Force item to be ready by manipulating next_retry_at
            {
                let mut items = q.items.lock().await;
                if let Some(item) = items.front_mut() {
                    item.next_retry_at = Utc::now() - Duration::seconds(1);
                    item.status = DeliveryStatus::Pending;
                }
            }
            if let Some(item) = q.next_ready().await {
                q.requeue_failed(item, "error".to_string()).await;
            }
        }

        assert_eq!(q.dead_letter_count().await, 1);
        assert_eq!(q.queue_size().await, 0);
    }

    #[test]
    fn backoff_delays() {
        assert_eq!(DeliveryItem::backoff_delay(0), Duration::seconds(0));
        assert_eq!(DeliveryItem::backoff_delay(1), Duration::seconds(1));
        assert_eq!(DeliveryItem::backoff_delay(2), Duration::seconds(5));
        assert_eq!(DeliveryItem::backoff_delay(3), Duration::seconds(30));
        assert_eq!(DeliveryItem::backoff_delay(4), Duration::minutes(5));
        assert_eq!(DeliveryItem::backoff_delay(5), Duration::minutes(15));
    }

    #[test]
    fn delivery_item_new() {
        let item = DeliveryItem::new("tg".into(), test_msg("hi"));
        assert_eq!(item.channel, "tg");
        assert_eq!(item.content, "hi");
        assert_eq!(item.status, DeliveryStatus::Pending);
        assert_eq!(item.attempts, 0);
        assert_eq!(item.max_attempts, 5);
    }

    #[test]
    fn mark_delivered() {
        let mut item = DeliveryItem::new("tg".into(), test_msg("x"));
        item.mark_delivered();
        assert_eq!(item.status, DeliveryStatus::Delivered);
        assert_eq!(item.attempts, 1);
    }
}
