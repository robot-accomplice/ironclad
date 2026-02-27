use ironclad_core::config::SessionConfig;
use ironclad_db::Database;
use ironclad_llm::format::UnifiedMessage;

pub struct SessionGovernor {
    config: SessionConfig,
}

impl SessionGovernor {
    pub fn new(config: SessionConfig) -> Self {
        Self { config }
    }

    /// Run a single maintenance tick: expire stale sessions based on TTL.
    /// Returns the number of sessions expired.
    pub fn tick(&self, db: &Database) -> ironclad_core::Result<usize> {
        let stale =
            ironclad_db::sessions::list_stale_active_session_ids(db, self.config.ttl_seconds)?;
        for session_id in &stale {
            self.compact_before_archive(db, session_id).ok();
            ironclad_db::sessions::set_session_status(
                db,
                session_id,
                ironclad_db::sessions::SessionStatus::Expired,
            )
            .ok();
        }
        Ok(stale.len())
    }

    /// Spawn a new scoped session for the given agent, returning the session id.
    pub fn spawn(
        &self,
        db: &Database,
        agent_id: &str,
        scope: Option<&ironclad_db::sessions::SessionScope>,
    ) -> ironclad_core::Result<String> {
        ironclad_db::sessions::find_or_create(db, agent_id, scope)
    }

    /// Rotate active agent-scope sessions by archiving them and creating a new
    /// active session for the same agent.
    pub fn rotate_agent_scope_sessions(
        &self,
        db: &Database,
        agent_id: &str,
    ) -> ironclad_core::Result<usize> {
        let sessions = ironclad_db::sessions::list_active_sessions(db, Some(agent_id))?;
        let agent_scoped: Vec<_> = sessions
            .into_iter()
            .filter(|s| s.scope_key.as_deref() == Some("agent"))
            .collect();
        for s in &agent_scoped {
            self.compact_before_archive(db, &s.id).ok();
        }
        if agent_scoped.is_empty() {
            return Ok(0);
        }
        let _ = ironclad_db::sessions::rotate_agent_session(db, agent_id)?;
        Ok(1)
    }

    fn compact_before_archive(&self, db: &Database, session_id: &str) -> ironclad_core::Result<()> {
        let msgs = ironclad_db::sessions::list_messages(db, session_id, Some(20))?;
        if msgs.len() < 4 {
            return Ok(());
        }
        let keep_recent = 4usize;
        let trim_end = msgs.len().saturating_sub(keep_recent);
        let trimmed: Vec<UnifiedMessage> = msgs[..trim_end]
            .iter()
            .map(|m| UnifiedMessage {
                role: m.role.clone(),
                content: m.content.clone(),
                parts: None,
            })
            .collect();
        if trimmed.is_empty() {
            return Ok(());
        }
        let prompt = crate::context::build_compaction_prompt(&trimmed);
        let digest = format!(
            "[Conversation Summary Draft]\n{}",
            prompt.chars().take(2_000).collect::<String>()
        );
        ironclad_db::sessions::append_message(db, session_id, "system", &digest)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    #[test]
    fn governor_tick_no_sessions() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let expired = gov.tick(&db).unwrap();
        assert_eq!(expired, 0);
    }

    #[test]
    fn governor_spawn_creates_session() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = gov.spawn(&db, "gov-agent", None).unwrap();
        assert!(!sid.is_empty());

        let sid2 = gov.spawn(&db, "gov-agent", None).unwrap();
        assert_eq!(sid, sid2, "same agent should reuse session");
    }

    #[test]
    fn governor_spawn_with_scope() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();

        let scope = ironclad_db::sessions::SessionScope::Peer {
            peer_id: "alice".into(),
            channel: "telegram".into(),
        };
        let sid_scoped = gov.spawn(&db, "gov-agent", Some(&scope)).unwrap();
        let sid_plain = gov.spawn(&db, "gov-agent", None).unwrap();
        assert_ne!(sid_scoped, sid_plain);
    }

    #[test]
    fn rotate_agent_scope_sessions_keeps_single_active_session() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid1 = ironclad_db::sessions::create_new(&db, "gov-rotate", None).unwrap();

        let rotated = gov.rotate_agent_scope_sessions(&db, "gov-rotate").unwrap();
        assert_eq!(rotated, 1);

        let active = ironclad_db::sessions::list_active_sessions(&db, Some("gov-rotate")).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].scope_key.as_deref(), Some("agent"));
        assert_ne!(active[0].id, sid1);

        let archived = ironclad_db::sessions::get_session(&db, &sid1)
            .unwrap()
            .unwrap();
        assert_eq!(archived.status, "archived");
    }

    // ── compact_before_archive tests (BUG-084) ─────────────────────

    #[test]
    fn compact_before_archive_fewer_than_4_messages_is_noop() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "compact-few", None).unwrap();

        // Add only 2 messages (< 4 threshold)
        ironclad_db::sessions::append_message(&db, &sid, "user", "hello").unwrap();
        ironclad_db::sessions::append_message(&db, &sid, "assistant", "hi there").unwrap();

        gov.compact_before_archive(&db, &sid).unwrap();

        // No extra system message should be appended
        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        assert_eq!(msgs.len(), 2);
        assert!(!msgs.iter().any(|m| m.content.contains("[Conversation Summary Draft]")));
    }

    #[test]
    fn compact_before_archive_with_enough_messages_appends_digest() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "compact-enough", None).unwrap();

        // Add 6 messages (>= 4 threshold)
        for i in 0..6 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ironclad_db::sessions::append_message(
                &db,
                &sid,
                role,
                &format!("message number {i}"),
            )
            .unwrap();
        }

        gov.compact_before_archive(&db, &sid).unwrap();

        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        // Should have original 6 + 1 compaction system message = 7
        assert_eq!(msgs.len(), 7);
        let last = msgs.last().unwrap();
        assert_eq!(last.role, "system");
        assert!(
            last.content.contains("[Conversation Summary Draft]"),
            "expected summary draft header"
        );
        assert!(
            last.content.contains("Summarize"),
            "expected summarize instruction"
        );
    }

    #[test]
    fn compact_before_archive_trims_old_keeps_recent_4() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "compact-trim", None).unwrap();

        // Add 8 messages
        for i in 0..8 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ironclad_db::sessions::append_message(
                &db,
                &sid,
                role,
                &format!("content-{i}"),
            )
            .unwrap();
        }

        gov.compact_before_archive(&db, &sid).unwrap();

        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        let summary_msg = msgs.iter().find(|m| m.content.contains("[Conversation Summary Draft]")).unwrap();

        // The summary should include content from trimmed messages (0..4) but
        // not from the kept recent 4 (4..8)
        assert!(
            summary_msg.content.contains("content-0"),
            "summary should include trimmed message 0"
        );
        assert!(
            summary_msg.content.contains("content-3"),
            "summary should include trimmed message 3"
        );
    }

    #[test]
    fn compact_before_archive_exactly_4_messages_is_noop() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "compact-exact", None).unwrap();

        // Add exactly 4 messages — trimmed slice would be empty
        for i in 0..4 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ironclad_db::sessions::append_message(
                &db,
                &sid,
                role,
                &format!("msg-{i}"),
            )
            .unwrap();
        }

        gov.compact_before_archive(&db, &sid).unwrap();

        // keep_recent = 4, trim_end = 4 - 4 = 0, trimmed slice is empty -> early return
        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        assert_eq!(msgs.len(), 4);
    }

    #[test]
    fn tick_expires_stale_sessions_with_compaction() {
        let gov = SessionGovernor::new(SessionConfig {
            ttl_seconds: 0, // immediate expiry
            ..SessionConfig::default()
        });
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "stale-agent", None).unwrap();

        // Add enough messages to trigger compaction
        for i in 0..6 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ironclad_db::sessions::append_message(
                &db,
                &sid,
                role,
                &format!("stale-msg-{i}"),
            )
            .unwrap();
        }

        // Allow the session to become stale
        std::thread::sleep(std::time::Duration::from_millis(50));

        let expired = gov.tick(&db).unwrap();
        assert_eq!(expired, 1);

        // Check the session is now expired
        let session = ironclad_db::sessions::get_session(&db, &sid).unwrap().unwrap();
        assert_eq!(session.status, "expired");

        // Compaction should have run — check for summary message
        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        assert!(
            msgs.iter().any(|m| m.content.contains("[Conversation Summary Draft]")),
            "compaction should have appended a summary"
        );
    }

    #[test]
    fn rotate_with_no_sessions_returns_zero() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let rotated = gov.rotate_agent_scope_sessions(&db, "nonexistent-agent").unwrap();
        assert_eq!(rotated, 0);
    }
}
