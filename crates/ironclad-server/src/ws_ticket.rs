//! Short-lived, single-use tickets for WebSocket authentication.
//!
//! Replaces the old `?token=<api_key>` pattern that leaked persistent
//! credentials into proxy/CDN/browser logs.
//!
//! ## Flow
//!
//! 1. Client `POST /api/ws-ticket` with `x-api-key` header → `{"ticket":"wst_…","expires_in":30}`
//! 2. Client connects to `/ws?ticket=wst_…` within 30 seconds
//! 3. Server validates (exists, not expired, not already used), consumes, upgrades

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use rand::RngCore;

/// Time-to-live for issued tickets.
const TICKET_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// Maximum outstanding tickets before forced cleanup.
const MAX_OUTSTANDING: usize = 1000;

/// Minimum interval between lazy cleanup sweeps.
const CLEANUP_INTERVAL: std::time::Duration = std::time::Duration::from_secs(60);

struct TicketEntry {
    issued_at: Instant,
}

/// In-memory store for short-lived WebSocket upgrade tickets.
///
/// Tickets are 30-second TTL, single-use, and stored only in memory —
/// a server restart invalidates all outstanding tickets (acceptable
/// because WS connections are also lost on restart).
#[derive(Clone)]
pub struct TicketStore {
    inner: Arc<Mutex<TicketStoreInner>>,
}

struct TicketStoreInner {
    tickets: HashMap<String, TicketEntry>,
    last_cleanup: Instant,
}

impl Default for TicketStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TicketStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TicketStoreInner {
                tickets: HashMap::new(),
                last_cleanup: Instant::now(),
            })),
        }
    }

    /// Issue a new ticket. Returns the ticket string (e.g. `wst_<64 hex chars>`).
    pub fn issue(&self) -> String {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        let ticket = format!("wst_{}", hex::encode(bytes));

        let mut inner = self.inner.lock().expect("ticket store lock poisoned");

        // Lazy cleanup: evict expired tickets periodically or when store is large
        if inner.tickets.len() >= MAX_OUTSTANDING
            || inner.last_cleanup.elapsed() >= CLEANUP_INTERVAL
        {
            let now = Instant::now();
            inner
                .tickets
                .retain(|_, e| now.duration_since(e.issued_at) < TICKET_TTL);
            inner.last_cleanup = now;
        }

        inner.tickets.insert(
            ticket.clone(),
            TicketEntry {
                issued_at: Instant::now(),
            },
        );
        ticket
    }

    /// Attempt to redeem a ticket. Returns `true` if the ticket was valid,
    /// not expired, and not previously used. The ticket is consumed atomically.
    pub fn redeem(&self, ticket: &str) -> bool {
        let mut inner = self.inner.lock().expect("ticket store lock poisoned");
        match inner.tickets.remove(ticket) {
            Some(entry) => entry.issued_at.elapsed() < TICKET_TTL,
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;
    use std::time::Duration;

    #[test]
    fn issue_format() {
        let store = TicketStore::new();
        let ticket = store.issue();
        assert!(ticket.starts_with("wst_"), "ticket should have wst_ prefix");
        // wst_ (4 chars) + 64 hex chars = 68 total
        assert_eq!(ticket.len(), 68, "ticket should be 68 chars total");
        // Verify the hex portion is valid hex
        assert!(
            hex::decode(&ticket[4..]).is_ok(),
            "suffix should be valid hex"
        );
    }

    #[test]
    fn issue_unique() {
        let store = TicketStore::new();
        let t1 = store.issue();
        let t2 = store.issue();
        assert_ne!(t1, t2, "tickets should be unique");
    }

    #[test]
    fn redeem_valid() {
        let store = TicketStore::new();
        let ticket = store.issue();
        assert!(store.redeem(&ticket), "valid ticket should redeem");
    }

    #[test]
    fn redeem_invalid() {
        let store = TicketStore::new();
        assert!(
            !store.redeem("wst_0000000000000000000000000000000000000000000000000000000000000000"),
            "unknown ticket should not redeem"
        );
    }

    #[test]
    fn redeem_single_use() {
        let store = TicketStore::new();
        let ticket = store.issue();
        assert!(store.redeem(&ticket), "first redeem should succeed");
        assert!(!store.redeem(&ticket), "second redeem should fail");
    }

    #[test]
    fn redeem_expired() {
        // We can't easily fast-forward Instant, so we test via a manual entry
        let store = TicketStore::new();
        {
            let mut inner = store.inner.lock().unwrap();
            inner.tickets.insert(
                "wst_expired".to_string(),
                TicketEntry {
                    issued_at: Instant::now() - Duration::from_secs(60),
                },
            );
        }
        assert!(
            !store.redeem("wst_expired"),
            "expired ticket should not redeem"
        );
    }

    #[test]
    fn cleanup_evicts_expired() {
        let store = TicketStore::new();
        // Insert an expired entry
        {
            let mut inner = store.inner.lock().unwrap();
            inner.tickets.insert(
                "wst_old".to_string(),
                TicketEntry {
                    issued_at: Instant::now() - Duration::from_secs(60),
                },
            );
            // Force cleanup on next issue by setting last_cleanup far in the past
            inner.last_cleanup = Instant::now() - Duration::from_secs(120);
        }
        // Issue a new ticket to trigger cleanup
        let _new = store.issue();
        let inner = store.inner.lock().unwrap();
        assert!(
            !inner.tickets.contains_key("wst_old"),
            "expired ticket should be cleaned up"
        );
    }

    #[test]
    fn empty_string_not_redeemable() {
        let store = TicketStore::new();
        assert!(!store.redeem(""), "empty string should not redeem");
    }

    #[test]
    fn concurrent_issue_and_redeem() {
        let store = TicketStore::new();
        let tickets: Vec<String> = (0..100).map(|_| store.issue()).collect();

        let store_clone = store.clone();
        let tickets_clone = tickets.clone();
        let handle = thread::spawn(move || {
            tickets_clone
                .iter()
                .filter(|t| store_clone.redeem(t))
                .count()
        });

        let count_main = tickets.iter().filter(|t| store.redeem(t)).count();
        let count_thread = handle.join().unwrap();

        // Each ticket should be redeemed exactly once across both threads
        assert_eq!(
            count_main + count_thread,
            100,
            "all tickets should be redeemed exactly once"
        );
    }
}
