use ironclad_core::config::SessionConfig;
use ironclad_db::Database;

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
        ironclad_db::sessions::expire_stale_sessions(db, self.config.ttl_seconds)
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
}
