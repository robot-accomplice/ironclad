use std::sync::Arc;
use ironclad_core::config::SessionConfig;
use ironclad_db::Database;
use tracing::{debug, info};

pub struct SessionGovernor {
    config: SessionConfig,
    db: Database,
}

impl SessionGovernor {
    pub fn new(config: SessionConfig, db: Database) -> Self {
        Self { config, db }
    }

    /// Run one cycle of session maintenance: expire stale sessions.
    pub fn tick(&self) -> Vec<String> {
        match ironclad_db::sessions::expire_stale_sessions(&self.db, self.config.ttl_seconds) {
            Ok(expired) => {
                if !expired.is_empty() {
                    info!(count = expired.len(), "expired stale sessions");
                }
                expired
            }
            Err(e) => {
                tracing::error!(error = %e, "session governor tick failed");
                vec![]
            }
        }
    }

    /// Spawn a background task that runs `tick()` at a regular interval.
    pub fn spawn(self: Arc<Self>, interval: std::time::Duration) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                debug!("session governor: running maintenance cycle");
                self.tick();
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_governor(ttl: u64) -> SessionGovernor {
        let db = Database::new(":memory:").unwrap();
        let config = SessionConfig {
            ttl_seconds: ttl,
            scope_mode: "agent".into(),
            reset_schedule: None,
        };
        SessionGovernor::new(config, db)
    }

    #[test]
    fn tick_no_sessions_no_errors() {
        let gov = test_governor(3600);
        let expired = gov.tick();
        assert!(expired.is_empty());
    }

    #[test]
    fn tick_does_not_expire_fresh_sessions() {
        let gov = test_governor(3600);
        let _sid = ironclad_db::sessions::find_or_create(&gov.db, "agent-1", None).unwrap();
        let expired = gov.tick();
        assert!(expired.is_empty());
    }

    #[test]
    fn tick_expires_old_sessions() {
        let gov = test_governor(1);
        let sid = ironclad_db::sessions::find_or_create(&gov.db, "agent-1", None).unwrap();
        // Backdate updated_at so it's clearly older than the TTL
        gov.db.conn().execute(
            "UPDATE sessions SET updated_at = datetime('now', '-10 seconds') WHERE id = ?1",
            [&sid],
        ).unwrap();
        let expired = gov.tick();
        assert_eq!(expired.len(), 1);
        assert_eq!(expired[0], sid);

        let session = ironclad_db::sessions::get_session(&gov.db, &sid).unwrap().unwrap();
        assert_eq!(session.status, "expired");
    }
}
