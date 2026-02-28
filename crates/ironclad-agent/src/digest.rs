use ironclad_core::config::DigestConfig;
use ironclad_db::Database;
use ironclad_db::sessions::{self, Session};
use tracing::{debug, info};

/// A generated summary of a session's key events and outcomes.
#[derive(Debug, Clone)]
pub struct EpisodicDigest {
    pub session_id: String,
    pub agent_id: String,
    pub summary: String,
    pub key_topics: Vec<String>,
    pub turn_count: i64,
    pub importance: i32,
}

impl EpisodicDigest {
    /// Generate a digest from a session's message history.
    pub fn from_session(db: &Database, session: &Session) -> Option<Self> {
        let messages = sessions::list_messages(db, &session.id, None)
            .inspect_err(|e| tracing::warn!(error = %e, session_id = %session.id, "failed to list messages for digest"))
            .ok()?;
        if messages.is_empty() {
            return None;
        }

        let mut topics = Vec::new();
        let mut summary_parts = Vec::new();
        let turn_count = messages.len() as i64;

        for msg in &messages {
            if msg.role == "user" {
                let first_line = msg.content.lines().next().unwrap_or("").trim();
                if !first_line.is_empty() && first_line.len() < 200 {
                    topics.push(first_line.to_string());
                }
            }
        }
        topics.truncate(5);

        if let Some(first_user) = messages.iter().find(|m| m.role == "user") {
            let truncated = truncate_str(&first_user.content, 200);
            summary_parts.push(format!("Started with: {truncated}"));
        }
        if let Some(last_assistant) = messages.iter().rev().find(|m| m.role == "assistant") {
            let truncated = truncate_str(&last_assistant.content, 200);
            summary_parts.push(format!("Concluded with: {truncated}"));
        }
        summary_parts.push(format!("Total turns: {turn_count}"));

        let importance = calculate_importance(turn_count, topics.len());

        Some(EpisodicDigest {
            session_id: session.id.clone(),
            agent_id: session.agent_id.clone(),
            summary: summary_parts.join(". "),
            key_topics: topics,
            turn_count,
            importance,
        })
    }

    /// Store this digest in episodic memory.
    pub fn persist(&self, db: &Database) -> ironclad_core::Result<String> {
        let content = format!(
            "[Session Digest] {}\nTopics: {}\nTurns: {}",
            self.summary,
            self.key_topics.join(", "),
            self.turn_count,
        );
        ironclad_db::memory::store_episodic(db, "digest", &content, self.importance)
    }
}

/// Calculate importance based on session engagement.
fn calculate_importance(turn_count: i64, topic_count: usize) -> i32 {
    let base = 5i32;
    let turn_bonus = (turn_count as i32 / 5).min(3);
    let topic_bonus = (topic_count as i32).min(2);
    (base + turn_bonus + topic_bonus).min(10)
}

/// Apply exponential decay to a digest's importance based on age.
pub fn decay_importance(original_importance: i32, age_days: f64, half_life_days: f64) -> i32 {
    if half_life_days <= 0.0 {
        return original_importance;
    }
    let decay_factor = (0.5_f64).powf(age_days / half_life_days);
    let decayed = (original_importance as f64 * decay_factor).round() as i32;
    decayed.max(1)
}

fn truncate_str(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        let boundary = s
            .char_indices()
            .take_while(|&(i, _)| i < max_len)
            .last()
            .map(|(i, c)| i + c.len_utf8())
            .unwrap_or(0);
        s[..boundary].to_string()
    }
}

/// Generate and persist a digest for a session that is being archived/expired.
pub fn digest_on_close(db: &Database, config: &DigestConfig, session: &Session) {
    if !config.enabled {
        debug!(session_id = %session.id, "digest generation disabled");
        return;
    }

    match EpisodicDigest::from_session(db, session) {
        Some(digest) => match digest.persist(db) {
            Ok(id) => info!(
                digest_id = %id,
                session_id = %session.id,
                topics = ?digest.key_topics,
                importance = digest.importance,
                "stored episodic digest"
            ),
            Err(e) => tracing::error!(error = %e, "failed to persist digest"),
        },
        None => debug!(session_id = %session.id, "no content to digest"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn empty_session_produces_no_digest() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        let session = sessions::get_session(&db, &sid).unwrap().unwrap();
        let digest = EpisodicDigest::from_session(&db, &session);
        assert!(digest.is_none());
    }

    #[test]
    fn session_with_messages_produces_digest() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        sessions::append_message(&db, &sid, "user", "How do I sort a vector in Rust?").unwrap();
        sessions::append_message(&db, &sid, "assistant", "Use vec.sort() or vec.sort_by()")
            .unwrap();

        let session = sessions::get_session(&db, &sid).unwrap().unwrap();
        let digest = EpisodicDigest::from_session(&db, &session).unwrap();
        assert_eq!(digest.session_id, sid);
        assert!(!digest.summary.is_empty());
        assert!(digest.summary.contains("sort"));
        assert_eq!(digest.turn_count, 2);
        assert!(!digest.key_topics.is_empty());
    }

    #[test]
    fn digest_persist_stores_in_episodic_memory() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        sessions::append_message(&db, &sid, "user", "Tell me about Rust").unwrap();
        sessions::append_message(&db, &sid, "assistant", "Rust is a systems language").unwrap();

        let session = sessions::get_session(&db, &sid).unwrap().unwrap();
        let digest = EpisodicDigest::from_session(&db, &session).unwrap();
        let id = digest.persist(&db).unwrap();
        assert!(!id.is_empty());

        let entries = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        let found = entries
            .iter()
            .any(|e| e.content.contains("[Session Digest]"));
        assert!(found, "digest should be stored in episodic memory");
    }

    #[test]
    fn calculate_importance_base() {
        assert_eq!(calculate_importance(1, 0), 5);
        assert_eq!(calculate_importance(5, 1), 7);
        assert_eq!(calculate_importance(20, 5), 10);
    }

    #[test]
    fn decay_importance_halves_at_half_life() {
        assert_eq!(decay_importance(10, 7.0, 7.0), 5);
    }

    #[test]
    fn decay_importance_zero_age_no_change() {
        assert_eq!(decay_importance(8, 0.0, 7.0), 8);
    }

    #[test]
    fn decay_importance_never_below_one() {
        assert_eq!(decay_importance(2, 100.0, 7.0), 1);
    }

    #[test]
    fn decay_importance_zero_half_life_no_decay() {
        assert_eq!(decay_importance(8, 30.0, 0.0), 8);
    }

    #[test]
    fn truncate_str_short() {
        assert_eq!(truncate_str("hello", 10), "hello");
    }

    #[test]
    fn truncate_str_long() {
        let long = "a".repeat(300);
        assert!(truncate_str(&long, 200).len() <= 200);
    }

    #[test]
    fn digest_on_close_disabled() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        sessions::append_message(&db, &sid, "user", "hello").unwrap();
        let session = sessions::get_session(&db, &sid).unwrap().unwrap();

        let config = DigestConfig {
            enabled: false,
            max_tokens: 512,
            decay_half_life_days: 7,
        };
        digest_on_close(&db, &config, &session);

        let entries = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        let has_digest = entries
            .iter()
            .any(|e| e.content.contains("[Session Digest]"));
        assert!(!has_digest);
    }

    #[test]
    fn digest_on_close_enabled() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        sessions::append_message(&db, &sid, "user", "hello").unwrap();
        sessions::append_message(&db, &sid, "assistant", "hi!").unwrap();
        let session = sessions::get_session(&db, &sid).unwrap().unwrap();

        let config = DigestConfig::default();
        digest_on_close(&db, &config, &session);

        let entries = ironclad_db::memory::retrieve_episodic(&db, 10).unwrap();
        let has_digest = entries
            .iter()
            .any(|e| e.content.contains("[Session Digest]"));
        assert!(has_digest);
    }

    #[test]
    fn topics_limited_to_five() {
        let db = test_db();
        let sid = sessions::find_or_create(&db, "agent-1", None).unwrap();
        for i in 0..10 {
            sessions::append_message(&db, &sid, "user", &format!("Topic {i}")).unwrap();
            sessions::append_message(&db, &sid, "assistant", "response").unwrap();
        }
        let session = sessions::get_session(&db, &sid).unwrap().unwrap();
        let digest = EpisodicDigest::from_session(&db, &session).unwrap();
        assert!(digest.key_topics.len() <= 5);
    }
}
