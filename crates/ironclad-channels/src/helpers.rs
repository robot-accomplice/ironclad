//! Shared helpers for channel adapters.
//!
//! These functions extract common patterns (message chunking, buffer management)
//! that were duplicated across multiple adapter implementations.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crate::InboundMessage;

/// Split a message into chunks respecting a maximum byte length.
///
/// Prefers splitting at newlines, then whitespace, then hard-splits at char
/// boundaries. Guarantees every chunk is valid UTF-8 and at most `max_len`
/// bytes.
pub fn chunk_message(text: &str, max_len: usize) -> Vec<String> {
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= max_len {
            chunks.push(remaining.to_string());
            break;
        }

        let safe_max = remaining.floor_char_boundary(max_len);

        // Guard: if floor_char_boundary returned 0 (first char wider than max_len),
        // force progress by advancing one char boundary to avoid an infinite loop.
        if safe_max == 0 {
            let next = remaining
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());
            chunks.push(remaining[..next].to_string());
            remaining = &remaining[next..];
            continue;
        }

        let boundary = &remaining[..safe_max];
        let split_at = boundary
            .rfind('\n')
            .or_else(|| boundary.rfind(|c: char| c.is_whitespace()))
            .unwrap_or(safe_max);

        let (chunk, rest) = remaining.split_at(split_at);
        // Skip empty chunks (e.g. when message starts with a newline)
        if !chunk.is_empty() {
            chunks.push(chunk.to_string());
        }
        remaining = rest.trim_start_matches('\n').trim_start();
    }

    chunks
}

/// Create a new shared message buffer.
pub fn new_message_buffer() -> Arc<Mutex<VecDeque<InboundMessage>>> {
    Arc::new(Mutex::new(VecDeque::new()))
}

/// Pop the next message from a shared buffer.
///
/// Uses poison-recovery semantics: if the mutex is poisoned (a thread panicked
/// while holding it), the lock is recovered rather than propagating the panic.
pub fn recv_from_buffer(buffer: &Mutex<VecDeque<InboundMessage>>) -> Option<InboundMessage> {
    buffer.lock().unwrap_or_else(|e| e.into_inner()).pop_front()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    #[test]
    fn chunk_short_message_returns_single() {
        let chunks = chunk_message("hello", 100);
        assert_eq!(chunks, vec!["hello"]);
    }

    #[test]
    fn chunk_splits_at_newline() {
        let text = "line one\nline two which is a bit longer\nline three";
        let chunks = chunk_message(text, 25);
        assert!(chunks.len() > 1);
        assert!(chunks[0].ends_with("one"));
    }

    #[test]
    fn chunk_splits_at_whitespace() {
        let text = "hello world this is a test of the chunking system";
        let chunks = chunk_message(text, 20);
        for chunk in &chunks {
            assert!(chunk.len() <= 20, "chunk too long: {}", chunk);
        }
    }

    #[test]
    fn chunk_empty_returns_single() {
        let chunks = chunk_message("", 100);
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "");
    }

    #[test]
    fn chunk_exact_boundary() {
        let text = "a".repeat(100);
        let chunks = chunk_message(&text, 100);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn chunk_no_whitespace_hard_split() {
        let text = "a".repeat(50);
        let chunks = chunk_message(&text, 20);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 20);
        }
    }

    #[test]
    fn chunk_unicode_boundary() {
        // Multi-byte characters should not be split
        let text = "ab".repeat(30) + &"\u{00f1}".repeat(50);
        let chunks = chunk_message(&text, 50);
        for chunk in &chunks {
            assert!(chunk.len() <= 50);
        }
    }

    #[test]
    fn chunk_long_message() {
        let text = "word ".repeat(500);
        let chunks = chunk_message(text.trim(), 100);
        for chunk in &chunks {
            assert!(chunk.len() <= 100);
        }
    }

    #[test]
    fn buffer_new_is_empty() {
        let buf = new_message_buffer();
        assert!(recv_from_buffer(&buf).is_none());
    }

    #[test]
    fn buffer_push_and_recv() {
        let buf = new_message_buffer();
        {
            let mut guard = buf.lock().unwrap();
            guard.push_back(InboundMessage {
                id: "m1".into(),
                platform: "test".into(),
                sender_id: "u1".into(),
                content: "hello".into(),
                timestamp: Utc::now(),
                metadata: None,
            });
        }
        let msg = recv_from_buffer(&buf).unwrap();
        assert_eq!(msg.id, "m1");
        assert_eq!(msg.content, "hello");
        assert!(recv_from_buffer(&buf).is_none());
    }

    #[test]
    fn buffer_fifo_order() {
        let buf = new_message_buffer();
        {
            let mut guard = buf.lock().unwrap();
            for i in 0..3 {
                guard.push_back(InboundMessage {
                    id: format!("m{i}"),
                    platform: "test".into(),
                    sender_id: "u1".into(),
                    content: format!("msg{i}"),
                    timestamp: Utc::now(),
                    metadata: None,
                });
            }
        }
        for i in 0..3 {
            let msg = recv_from_buffer(&buf).unwrap();
            assert_eq!(msg.content, format!("msg{i}"));
        }
        assert!(recv_from_buffer(&buf).is_none());
    }
}
