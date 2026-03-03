//! Lock-free atomic port allocator for parallel test sandboxes.
//!
//! Each call to [`allocate_port`] returns a unique, bind-verified port
//! in the ephemeral range (49152–64000). The atomic counter guarantees
//! no two tests racing on different threads will receive the same port.

use std::sync::atomic::{AtomicU16, Ordering};

/// Starting port for allocation. Well above the registered port range
/// to avoid conflicts with running services.
const PORT_RANGE_START: u16 = 49152;
const PORT_RANGE_END: u16 = 64000;

static NEXT_PORT: AtomicU16 = AtomicU16::new(PORT_RANGE_START);

/// Allocate a unique port that is confirmed available via a TCP bind check.
///
/// The atomic counter ensures no two concurrent callers receive the same port.
/// A quick `TcpListener::bind` verifies the OS hasn't already assigned it.
pub fn allocate_port() -> u16 {
    loop {
        let port = NEXT_PORT.fetch_add(1, Ordering::Relaxed);
        if port >= PORT_RANGE_END {
            // Wrap around (unlikely in practice — would need 15k+ parallel tests)
            NEXT_PORT.store(PORT_RANGE_START, Ordering::Relaxed);
            continue;
        }
        // Verify the port is actually available
        if std::net::TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn allocates_unique_ports() {
        let mut seen = HashSet::new();
        for _ in 0..20 {
            let port = allocate_port();
            assert!(port >= PORT_RANGE_START);
            assert!(port < PORT_RANGE_END);
            assert!(seen.insert(port), "duplicate port: {port}");
        }
    }
}
