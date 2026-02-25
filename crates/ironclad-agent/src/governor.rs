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
        Ok(agent_scoped.len())
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

    #[test]
    fn rotate_agent_scope_sessions_reports_archived_count() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        db.conn()
            .execute("DROP INDEX idx_sessions_active_scope_unique", [])
            .unwrap();
        let sid1 = ironclad_db::sessions::create_new(&db, "gov-rotate-count", None).unwrap();
        let sid2 = ironclad_db::sessions::create_new(&db, "gov-rotate-count", None).unwrap();
        let sid3 = ironclad_db::sessions::create_new(&db, "gov-rotate-count", None).unwrap();

        let rotated = gov
            .rotate_agent_scope_sessions(&db, "gov-rotate-count")
            .unwrap();
        assert_eq!(rotated, 3);

        for sid in [sid1, sid2, sid3] {
            let archived = ironclad_db::sessions::get_session(&db, &sid)
                .unwrap()
                .unwrap();
            assert_eq!(archived.status, "archived");
        }

        let active =
            ironclad_db::sessions::list_active_sessions(&db, Some("gov-rotate-count")).unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].scope_key.as_deref(), Some("agent"));
    }
}
