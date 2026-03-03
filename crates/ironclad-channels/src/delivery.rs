use std::collections::VecDeque;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use ironclad_db::Database;
use ironclad_db::delivery_queue as dq_store;
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
    pub idempotency_key: String,
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
            idempotency_key: Uuid::new_v4().to_string(),
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
        self.last_error = Some(error.clone());
        if self.attempts >= self.max_attempts || Self::is_permanent_error(&error) {
            self.status = DeliveryStatus::DeadLetter;
        } else {
            self.status = DeliveryStatus::Pending;
            self.next_retry_at = Utc::now() + Self::backoff_delay(self.attempts);
        }
    }

    /// HTTP 4xx client errors that will never succeed on retry.
    pub fn is_permanent_error(error: &str) -> bool {
        let permanent_patterns = [
            "403 Forbidden",
            "401 Unauthorized",
            "400 Bad Request",
            "blocked by the user",
            "bot was blocked",
            "chat not found",
            "user is deactivated",
            "bot was kicked",
            "PEER_ID_INVALID",
        ];
        permanent_patterns.iter().any(|p| error.contains(p))
    }

    pub fn mark_delivered(&mut self) {
        self.status = DeliveryStatus::Delivered;
        self.attempts += 1;
    }

    pub fn is_ready(&self) -> bool {
        self.status == DeliveryStatus::Pending && Utc::now() >= self.next_retry_at
    }

    fn to_store_status(&self) -> &'static str {
        match self.status {
            DeliveryStatus::Pending => "pending",
            DeliveryStatus::InFlight => "in_flight",
            DeliveryStatus::Delivered => "delivered",
            DeliveryStatus::Failed => "failed",
            DeliveryStatus::DeadLetter => "dead_letter",
        }
    }

    fn from_store_status(status: &str) -> DeliveryStatus {
        match status {
            "in_flight" => DeliveryStatus::InFlight,
            "delivered" => DeliveryStatus::Delivered,
            "failed" => DeliveryStatus::Failed,
            "dead_letter" => DeliveryStatus::DeadLetter,
            _ => DeliveryStatus::Pending,
        }
    }

    fn to_record(&self) -> dq_store::DeliveryQueueRecord {
        dq_store::DeliveryQueueRecord {
            id: self.id.clone(),
            channel: self.channel.clone(),
            recipient_id: self.recipient_id.clone(),
            content: self.content.clone(),
            status: self.to_store_status().to_string(),
            attempts: self.attempts,
            max_attempts: self.max_attempts,
            next_retry_at: self.next_retry_at,
            last_error: self.last_error.clone(),
            idempotency_key: self.idempotency_key.clone(),
            created_at: self.created_at,
        }
    }

    fn from_record(record: dq_store::DeliveryQueueRecord) -> Self {
        let idempotency_key = if record.idempotency_key.is_empty() {
            record.id.clone()
        } else {
            record.idempotency_key.clone()
        };
        Self {
            id: record.id,
            channel: record.channel,
            recipient_id: record.recipient_id,
            content: record.content,
            idempotency_key,
            status: Self::from_store_status(&record.status),
            attempts: record.attempts,
            max_attempts: record.max_attempts,
            next_retry_at: record.next_retry_at,
            created_at: record.created_at,
            last_error: record.last_error,
        }
    }
}

pub struct DeliveryQueue {
    pub(crate) items: Arc<Mutex<VecDeque<DeliveryItem>>>,
    dead_letters: Arc<Mutex<Vec<DeliveryItem>>>,
    store: Option<Database>,
}

impl DeliveryQueue {
    pub fn new() -> Self {
        Self {
            items: Arc::new(Mutex::new(VecDeque::new())),
            dead_letters: Arc::new(Mutex::new(Vec::new())),
            store: None,
        }
    }

    pub fn with_store(store: Database) -> Self {
        Self {
            items: Arc::new(Mutex::new(VecDeque::new())),
            dead_letters: Arc::new(Mutex::new(Vec::new())),
            store: Some(store),
        }
    }

    fn persist_item(&self, item: &DeliveryItem) {
        if let Some(db) = &self.store
            && let Err(e) = dq_store::upsert_delivery_item(db, &item.to_record())
        {
            warn!(id = %item.id, error = %e, "failed to persist delivery item");
        }
    }

    pub async fn recover_from_store(&self) {
        let Some(db) = &self.store else {
            return;
        };
        match dq_store::list_recoverable(db, 2000) {
            Ok(records) => {
                if records.is_empty() {
                    return;
                }
                let recovered = records
                    .into_iter()
                    .map(DeliveryItem::from_record)
                    .collect::<VecDeque<_>>();
                let mut items = self.items.lock().await;
                let count = recovered.len();
                *items = recovered;
                debug!(count, "recovered delivery queue items from database");
            }
            Err(e) => warn!(error = %e, "failed to recover delivery queue from database"),
        }
    }

    pub async fn enqueue(&self, channel: String, msg: OutboundMessage) -> String {
        let item = DeliveryItem::new(channel, msg);
        let id = item.id.clone();
        debug!(id = %id, "enqueuing delivery item");
        let mut items = self.items.lock().await;
        items.push_back(item);
        if let Some(last) = items.back() {
            self.persist_item(last);
        }
        id
    }

    pub async fn next_ready(&self) -> Option<DeliveryItem> {
        let mut items = self.items.lock().await;
        let pos = items.iter().position(|item| item.is_ready())?;
        let mut item = items.remove(pos)?;
        item.status = DeliveryStatus::InFlight;
        self.persist_item(&item);
        if let Some(db) = &self.store
            && let Err(e) = dq_store::mark_in_flight(db, &item.id)
        {
            warn!(id = %item.id, error = %e, "failed to mark item as in-flight");
        }
        Some(item)
    }

    pub async fn mark_success(&self, id: &str) {
        debug!(id, "delivery succeeded");
        if let Some(db) = &self.store
            && let Err(e) = dq_store::mark_delivered(db, id)
        {
            warn!(id, error = %e, "failed to mark delivery as delivered");
        }
    }

    pub async fn requeue_failed(&self, mut item: DeliveryItem, error: String) {
        item.mark_failed(error.clone());
        if item.status == DeliveryStatus::DeadLetter {
            warn!(id = %item.id, attempts = item.attempts, "moving to dead letter");
            let mut dead = self.dead_letters.lock().await;
            dead.push(item);
            if let Some(last) = dead.last() {
                self.persist_item(last);
            }
        } else {
            debug!(id = %item.id, attempt = item.attempts, "requeuing with backoff");
            let mut items = self.items.lock().await;
            items.push_back(item);
            if let Some(last) = items.back() {
                self.persist_item(last);
            }
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

    pub fn dead_letters_from_store(&self, max_items: usize) -> Vec<DeliveryItem> {
        let Some(db) = &self.store else {
            return Vec::new();
        };
        dq_store::list_dead_letters(db, max_items)
            .inspect_err(|e| warn!(error = %e, "failed to load dead letters from store"))
            .map(|rows| rows.into_iter().map(DeliveryItem::from_record).collect())
            .unwrap_or_default()
    }

    pub fn replay_dead_letter_in_store(&self, id: &str) -> bool {
        let Some(db) = &self.store else {
            return false;
        };
        dq_store::replay_dead_letter(db, id).unwrap_or(false)
    }

    pub async fn replay_dead_letter_in_memory(&self, id: &str) -> bool {
        let mut dead = self.dead_letters.lock().await;
        if let Some(pos) = dead.iter().position(|item| item.id == id)
            && let Some(mut item) = dead.get(pos).cloned()
        {
            // Hold both locks to avoid the item existing in neither collection
            let mut items = self.items.lock().await;
            dead.remove(pos);
            item.status = DeliveryStatus::Pending;
            item.next_retry_at = Utc::now();
            items.push_back(item);
            return true;
        }
        false
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
    use ironclad_db::Database;

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
        assert!(!item.idempotency_key.is_empty());
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

    #[test]
    fn permanent_error_detected() {
        assert!(DeliveryItem::is_permanent_error(
            "Telegram API 403 Forbidden: Forbidden: bot was blocked by the user"
        ));
        assert!(DeliveryItem::is_permanent_error("401 Unauthorized"));
        assert!(DeliveryItem::is_permanent_error(
            "400 Bad Request: chat not found"
        ));
        assert!(DeliveryItem::is_permanent_error("user is deactivated"));
        assert!(DeliveryItem::is_permanent_error(
            "bot was kicked from the group"
        ));
        assert!(DeliveryItem::is_permanent_error("PEER_ID_INVALID"));
    }

    #[test]
    fn transient_error_not_permanent() {
        assert!(!DeliveryItem::is_permanent_error(
            "rate limited, retry after 5s"
        ));
        assert!(!DeliveryItem::is_permanent_error("network timeout"));
        assert!(!DeliveryItem::is_permanent_error(
            "500 Internal Server Error"
        ));
        assert!(!DeliveryItem::is_permanent_error("connection reset"));
    }

    #[test]
    fn mark_failed_permanent_dead_letters_immediately() {
        let mut item = DeliveryItem::new("tg".into(), test_msg("hi"));
        item.mark_failed("Telegram API 403 Forbidden: bot was blocked by the user".into());
        assert_eq!(item.status, DeliveryStatus::DeadLetter);
        assert_eq!(item.attempts, 1);
    }

    #[tokio::test]
    async fn permanent_error_dead_letters_on_first_requeue() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg")).await;
        let item = q.next_ready().await.unwrap();
        q.requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;
        assert_eq!(q.dead_letter_count().await, 1);
        assert_eq!(q.queue_size().await, 0);
    }

    #[tokio::test]
    async fn store_backed_queue_recovers_items() {
        let db = Database::new(":memory:").expect("db");
        let q = DeliveryQueue::with_store(db.clone());
        let id = q.enqueue("telegram".into(), test_msg("persist")).await;
        assert!(!id.is_empty());

        let recovered = DeliveryQueue::with_store(db);
        recovered.recover_from_store().await;
        assert_eq!(recovered.queue_size().await, 1);
        let item = recovered.next_ready().await.expect("recovered item");
        assert_eq!(item.content, "persist");
        recovered.mark_success(&item.id).await;
    }

    #[test]
    fn to_store_status_mappings() {
        let mut item = DeliveryItem::new("ch".into(), test_msg("hi"));
        assert_eq!(item.to_store_status(), "pending");

        item.status = DeliveryStatus::InFlight;
        assert_eq!(item.to_store_status(), "in_flight");

        item.status = DeliveryStatus::Delivered;
        assert_eq!(item.to_store_status(), "delivered");

        item.status = DeliveryStatus::Failed;
        assert_eq!(item.to_store_status(), "failed");

        item.status = DeliveryStatus::DeadLetter;
        assert_eq!(item.to_store_status(), "dead_letter");
    }

    #[test]
    fn from_store_status_mappings() {
        assert_eq!(
            DeliveryItem::from_store_status("pending"),
            DeliveryStatus::Pending
        );
        assert_eq!(
            DeliveryItem::from_store_status("in_flight"),
            DeliveryStatus::InFlight
        );
        assert_eq!(
            DeliveryItem::from_store_status("delivered"),
            DeliveryStatus::Delivered
        );
        assert_eq!(
            DeliveryItem::from_store_status("failed"),
            DeliveryStatus::Failed
        );
        assert_eq!(
            DeliveryItem::from_store_status("dead_letter"),
            DeliveryStatus::DeadLetter
        );
        // Unknown defaults to Pending
        assert_eq!(
            DeliveryItem::from_store_status("unknown"),
            DeliveryStatus::Pending
        );
        assert_eq!(DeliveryItem::from_store_status(""), DeliveryStatus::Pending);
    }

    #[test]
    fn to_record_and_from_record_roundtrip() {
        let item = DeliveryItem::new("telegram".into(), test_msg("roundtrip"));
        let record = item.to_record();
        assert_eq!(record.channel, "telegram");
        assert_eq!(record.content, "roundtrip");
        assert_eq!(record.status, "pending");
        assert_eq!(record.attempts, 0);
        assert_eq!(record.max_attempts, 5);

        let recovered = DeliveryItem::from_record(record);
        assert_eq!(recovered.channel, "telegram");
        assert_eq!(recovered.content, "roundtrip");
        assert_eq!(recovered.status, DeliveryStatus::Pending);
        assert_eq!(recovered.idempotency_key, item.idempotency_key);
    }

    #[test]
    fn from_record_empty_idempotency_key_falls_back_to_id() {
        let record = dq_store::DeliveryQueueRecord {
            id: "rec-1".into(),
            channel: "ch".into(),
            recipient_id: "r1".into(),
            content: "msg".into(),
            status: "pending".into(),
            attempts: 0,
            max_attempts: 5,
            next_retry_at: Utc::now(),
            last_error: None,
            idempotency_key: "".into(),
            created_at: Utc::now(),
        };
        let item = DeliveryItem::from_record(record);
        assert_eq!(item.idempotency_key, "rec-1");
    }

    #[test]
    fn mark_failed_transient_requeues() {
        let mut item = DeliveryItem::new("ch".into(), test_msg("hi"));
        item.mark_failed("timeout".into());
        assert_eq!(item.status, DeliveryStatus::Pending);
        assert_eq!(item.attempts, 1);
        assert!(item.last_error.as_ref().unwrap().contains("timeout"));
    }

    #[test]
    fn mark_failed_max_attempts_dead_letters() {
        let mut item = DeliveryItem::new("ch".into(), test_msg("hi"));
        item.max_attempts = 2;
        item.mark_failed("err1".into());
        assert_eq!(item.status, DeliveryStatus::Pending);
        item.mark_failed("err2".into());
        assert_eq!(item.status, DeliveryStatus::DeadLetter);
    }

    #[test]
    fn is_ready_checks_status_and_time() {
        let mut item = DeliveryItem::new("ch".into(), test_msg("hi"));
        // Pending + next_retry_at in past = ready
        item.next_retry_at = Utc::now() - Duration::seconds(1);
        assert!(item.is_ready());

        // InFlight should not be ready
        item.status = DeliveryStatus::InFlight;
        assert!(!item.is_ready());

        // Pending but next_retry_at in future = not ready
        item.status = DeliveryStatus::Pending;
        item.next_retry_at = Utc::now() + Duration::hours(1);
        assert!(!item.is_ready());
    }

    #[test]
    fn delivery_status_serde_roundtrip() {
        let statuses = vec![
            DeliveryStatus::Pending,
            DeliveryStatus::InFlight,
            DeliveryStatus::Delivered,
            DeliveryStatus::Failed,
            DeliveryStatus::DeadLetter,
        ];
        for status in statuses {
            let json = serde_json::to_string(&status).unwrap();
            let decoded: DeliveryStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, status);
        }
    }

    #[test]
    fn delivery_item_clone_and_debug() {
        let item = DeliveryItem::new("ch".into(), test_msg("hi"));
        let cloned = item.clone();
        assert_eq!(cloned.id, item.id);
        assert_eq!(cloned.content, item.content);
        // Debug should not panic
        let _ = format!("{:?}", item);
    }

    #[tokio::test]
    async fn pending_count_filters_correctly() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg1")).await;
        q.enqueue("ch".into(), test_msg("msg2")).await;
        assert_eq!(q.pending_count().await, 2);

        // Take one out (now InFlight)
        q.next_ready().await;
        assert_eq!(q.pending_count().await, 1);
    }

    #[tokio::test]
    async fn dead_letters_accessor() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg")).await;
        let item = q.next_ready().await.unwrap();
        // Force dead letter with permanent error
        q.requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;
        let dead = q.dead_letters().await;
        assert_eq!(dead.len(), 1);
        assert_eq!(dead[0].content, "msg");
    }

    #[tokio::test]
    async fn replay_dead_letter_in_memory_success() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("replay_me")).await;
        let item = q.next_ready().await.unwrap();
        let item_id = item.id.clone();
        q.requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;
        assert_eq!(q.dead_letter_count().await, 1);
        assert_eq!(q.queue_size().await, 0);

        // Replay it
        let replayed = q.replay_dead_letter_in_memory(&item_id).await;
        assert!(replayed);
        assert_eq!(q.dead_letter_count().await, 0);
        assert_eq!(q.queue_size().await, 1);
    }

    #[tokio::test]
    async fn replay_dead_letter_in_memory_nonexistent() {
        let q = DeliveryQueue::new();
        assert!(!q.replay_dead_letter_in_memory("nonexistent").await);
    }

    #[test]
    fn dead_letters_from_store_no_store() {
        let q = DeliveryQueue::new();
        assert!(q.dead_letters_from_store(100).is_empty());
    }

    #[test]
    fn replay_dead_letter_in_store_no_store() {
        let q = DeliveryQueue::new();
        assert!(!q.replay_dead_letter_in_store("id1"));
    }

    #[test]
    fn delivery_queue_default() {
        let q = DeliveryQueue::default();
        let rt = tokio::runtime::Runtime::new().unwrap();
        assert_eq!(rt.block_on(q.queue_size()), 0);
    }

    #[tokio::test]
    async fn mark_success_without_store() {
        let q = DeliveryQueue::new();
        q.enqueue("ch".into(), test_msg("msg")).await;
        let item = q.next_ready().await.unwrap();
        // Should not panic even without store
        q.mark_success(&item.id).await;
    }

    #[test]
    fn backoff_delay_high_attempt() {
        // Attempts beyond 5 should all be 15 minutes
        assert_eq!(DeliveryItem::backoff_delay(6), Duration::minutes(15));
        assert_eq!(DeliveryItem::backoff_delay(10), Duration::minutes(15));
        assert_eq!(DeliveryItem::backoff_delay(100), Duration::minutes(15));
    }

    #[test]
    fn persist_item_without_store_is_noop() {
        let q = DeliveryQueue::new();
        let item = DeliveryItem::new("ch".into(), test_msg("hi"));
        // Should not panic
        q.persist_item(&item);
    }

    #[tokio::test]
    async fn recover_from_store_without_store_is_noop() {
        let q = DeliveryQueue::new();
        // Should not panic
        q.recover_from_store().await;
    }

    #[tokio::test]
    async fn store_backed_dead_letters_from_store() {
        let db = Database::new(":memory:").expect("db");
        let q = DeliveryQueue::with_store(db);
        q.enqueue("ch".into(), test_msg("dead_store")).await;
        let item = q.next_ready().await.unwrap();
        q.requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;

        let dead = q.dead_letters_from_store(10);
        assert!(!dead.is_empty());
        assert_eq!(dead[0].content, "dead_store");
    }

    #[tokio::test]
    async fn store_backed_replay_dead_letter() {
        let db = Database::new(":memory:").expect("db");
        let q = DeliveryQueue::with_store(db);
        q.enqueue("ch".into(), test_msg("replay_store")).await;
        let item = q.next_ready().await.unwrap();
        let item_id = item.id.clone();
        q.requeue_failed(item, "403 Forbidden: bot was blocked by the user".into())
            .await;

        let replayed = q.replay_dead_letter_in_store(&item_id);
        assert!(replayed);
    }

    #[tokio::test]
    async fn store_backed_next_ready_marks_in_flight() {
        let db = Database::new(":memory:").expect("db");
        let q = DeliveryQueue::with_store(db);
        q.enqueue("ch".into(), test_msg("flight")).await;
        let item = q.next_ready().await.unwrap();
        assert_eq!(item.status, DeliveryStatus::InFlight);
    }

    #[tokio::test]
    async fn store_backed_mark_success() {
        let db = Database::new(":memory:").expect("db");
        let q = DeliveryQueue::with_store(db);
        q.enqueue("ch".into(), test_msg("success")).await;
        let item = q.next_ready().await.unwrap();
        // Should not panic with store
        q.mark_success(&item.id).await;
    }

    // ── property-based tests (v0.8.0 stabilization) ────────────────────

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn proptest_backoff_delay_is_monotonic(a in 0u32..10, b in 0u32..10) {
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            prop_assert!(
                DeliveryItem::backoff_delay(hi) >= DeliveryItem::backoff_delay(lo),
                "backoff_delay({}) < backoff_delay({})", hi, lo
            );
        }

        #[test]
        fn proptest_backoff_delay_zero_is_zero(_seed in 0u32..100) {
            let delay = DeliveryItem::backoff_delay(0);
            prop_assert_eq!(delay, Duration::seconds(0),
                "backoff_delay(0) should always be zero");
        }

        #[test]
        fn proptest_is_permanent_error_false_for_empty(_seed in 0u32..100) {
            prop_assert!(!DeliveryItem::is_permanent_error(""),
                "empty string should not be a permanent error");
        }

        #[test]
        fn proptest_is_permanent_error_false_for_transient(
            error in "(timeout|network error|rate limited|connection reset|500 Internal Server Error|502 Bad Gateway|503 Service Unavailable)"
        ) {
            prop_assert!(
                !DeliveryItem::is_permanent_error(&error),
                "transient error {:?} should not be permanent", error
            );
        }
    }
}
