use chrono::{DateTime, Utc};
use ironclad_core::config::{DigestConfig, LearningConfig, SessionConfig};
use ironclad_db::Database;
use ironclad_llm::format::UnifiedMessage;
use std::path::PathBuf;

pub struct SessionGovernor {
    config: SessionConfig,
    digest_config: DigestConfig,
    learning_config: LearningConfig,
    skills_dir: Option<PathBuf>,
}

impl SessionGovernor {
    pub fn new(config: SessionConfig) -> Self {
        Self {
            config,
            digest_config: DigestConfig::default(),
            learning_config: LearningConfig::default(),
            skills_dir: None,
        }
    }

    pub fn with_digest(mut self, digest_config: DigestConfig) -> Self {
        self.digest_config = digest_config;
        self
    }

    pub fn with_learning(mut self, learning_config: LearningConfig, skills_dir: PathBuf) -> Self {
        self.learning_config = learning_config;
        self.skills_dir = Some(skills_dir);
        self
    }

    /// Run a single maintenance tick: expire stale sessions based on TTL.
    /// Returns the number of sessions actually expired.
    pub fn tick(&self, db: &Database) -> ironclad_core::Result<usize> {
        let stale =
            ironclad_db::sessions::list_stale_active_session_ids(db, self.config.ttl_seconds)?;
        let mut expired = 0usize;
        for session_id in &stale {
            if let Err(e) = self.compact_before_archive(db, session_id) {
                tracing::warn!(error = %e, session_id = %session_id, "compaction failed before archive, proceeding with expiry");
            }
            // Generate episodic digest before the session status changes
            if let Ok(Some(session)) = ironclad_db::sessions::get_session(db, session_id) {
                crate::digest::digest_on_close(db, &self.digest_config, &session);
                if let Some(ref skills_dir) = self.skills_dir {
                    crate::learning::learn_on_close(
                        db,
                        &self.learning_config,
                        &session,
                        skills_dir,
                    );
                }
            }
            // Clean up checkpoints before expiry
            if let Err(e) = ironclad_db::checkpoint::clear_checkpoints(db, session_id) {
                tracing::warn!(error = %e, session_id = %session_id, "failed to clear checkpoints");
            }
            if let Err(e) = ironclad_db::sessions::set_session_status(
                db,
                session_id,
                ironclad_db::sessions::SessionStatus::Expired,
            ) {
                tracing::error!(error = %e, session_id = %session_id, "failed to expire session");
                continue;
            }
            expired += 1;
        }
        if let Err(e) = self.decay_episodic_importance(db) {
            tracing::warn!(error = %e, "episodic importance decay failed during governor tick");
        }
        if let Err(e) = self.adjust_learned_skill_priorities(db) {
            tracing::warn!(error = %e, "learned skill priority adjustment failed during governor tick");
        }
        Ok(expired)
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
            if let Err(e) = self.compact_before_archive(db, &s.id) {
                tracing::warn!(error = %e, session_id = %s.id, "compaction failed before rotation");
            }
            crate::digest::digest_on_close(db, &self.digest_config, s);
            if let Some(ref skills_dir) = self.skills_dir {
                crate::learning::learn_on_close(db, &self.learning_config, s, skills_dir);
            }
            if let Err(e) = ironclad_db::checkpoint::clear_checkpoints(db, &s.id) {
                tracing::warn!(error = %e, session_id = %s.id, "failed to clear checkpoints on rotation");
            }
        }
        let archived = agent_scoped.len();
        if archived == 0 {
            return Ok(0);
        }
        let _ = ironclad_db::sessions::rotate_agent_session(db, agent_id)?;
        Ok(archived)
    }

    fn compact_before_archive(&self, db: &Database, session_id: &str) -> ironclad_core::Result<()> {
        let msgs = ironclad_db::sessions::list_messages(db, session_id, None)?;
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

        // Progressive compaction: pick the least aggressive stage that fits ~500 tokens.
        let current_tokens = crate::context::count_tokens(&trimmed);
        let target_tokens = 500usize;
        let excess_ratio = current_tokens as f64 / target_tokens.max(1) as f64;
        let stage = crate::context::CompactionStage::from_excess(excess_ratio);
        let compacted = crate::context::compact_to_stage(&trimmed, stage);

        // Format the compacted messages into a summary block.
        let summary_lines: Vec<String> = compacted
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| format!("{}: {}", m.role, m.content))
            .collect();
        let summary_body = if summary_lines.is_empty() {
            compacted
                .iter()
                .map(|m| m.content.clone())
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            summary_lines.join("\n")
        };
        let digest = format!(
            "[Conversation Summary — {stage:?}]\n{}",
            summary_body.chars().take(2_000).collect::<String>()
        );
        ironclad_db::sessions::append_message(db, session_id, "system", &digest)?;
        Ok(())
    }

    /// Adjust learned skill priorities based on success/failure ratios.
    ///
    /// - Skills with > 5 total uses and > 80% success → boost priority
    /// - Skills where failures exceed successes → decay priority
    ///
    /// Returns the number of skills whose priority was actually changed.
    fn adjust_learned_skill_priorities(&self, db: &Database) -> ironclad_core::Result<usize> {
        if !self.learning_config.enabled {
            return Ok(0);
        }
        let skills = ironclad_db::learned_skills::list_learned_skills(db, 200)?;
        let mut adjusted = 0usize;
        let boost = self.learning_config.priority_boost_on_success as i64;
        let decay = self.learning_config.priority_decay_on_failure as i64;

        for skill in &skills {
            let total = skill.success_count + skill.failure_count;
            let ratio = if total > 0 {
                skill.success_count as f64 / total as f64
            } else {
                0.0
            };

            let new_priority = if total > 5 && ratio > 0.8 {
                // Reliable skill — boost
                (skill.priority + boost).min(100)
            } else if skill.failure_count > skill.success_count {
                // Unreliable skill — decay
                (skill.priority - decay).max(0)
            } else {
                continue;
            };

            if new_priority != skill.priority {
                if let Err(e) = ironclad_db::learned_skills::update_learned_skill_priority(
                    db,
                    &skill.name,
                    new_priority,
                ) {
                    tracing::warn!(error = %e, skill = %skill.name, "failed to adjust skill priority");
                } else {
                    adjusted += 1;
                }
            }
        }
        Ok(adjusted)
    }

    fn decay_episodic_importance(&self, db: &Database) -> ironclad_core::Result<usize> {
        let half_life_days = self.digest_config.decay_half_life_days as f64;
        if half_life_days <= 0.0 {
            return Ok(0);
        }

        let now = Utc::now();
        let conn = db.conn();
        let mut stmt = conn
            .prepare("SELECT id, importance, created_at FROM episodic_memory")
            .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let importance: i32 = row.get(1)?;
                let created_at: String = row.get(2)?;
                Ok((id, importance, created_at))
            })
            .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;

        let mut updates: Vec<(String, i32)> = Vec::new();
        for row in rows {
            let (id, importance, created_at) =
                row.map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
            if let Ok(created_dt) = DateTime::parse_from_rfc3339(&created_at) {
                let age_days = (now - created_dt.with_timezone(&Utc))
                    .to_std()
                    .map(|d| d.as_secs_f64() / 86_400.0)
                    .unwrap_or(0.0);
                let decayed = crate::digest::decay_importance(importance, age_days, half_life_days);
                if decayed != importance {
                    updates.push((id, decayed));
                }
            }
        }
        drop(stmt);

        for (id, new_importance) in &updates {
            conn.execute(
                "UPDATE episodic_memory SET importance = ?1 WHERE id = ?2",
                (&new_importance, id),
            )
            .map_err(|e| ironclad_core::IroncladError::Database(e.to_string()))?;
        }

        Ok(updates.len())
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
        assert!(
            !msgs
                .iter()
                .any(|m| m.content.contains("[Conversation Summary"))
        );
    }

    #[test]
    fn compact_before_archive_with_enough_messages_appends_digest() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let sid = ironclad_db::sessions::create_new(&db, "compact-enough", None).unwrap();

        // Add 6 messages (>= 4 threshold)
        for i in 0..6 {
            let role = if i % 2 == 0 { "user" } else { "assistant" };
            ironclad_db::sessions::append_message(&db, &sid, role, &format!("message number {i}"))
                .unwrap();
        }

        gov.compact_before_archive(&db, &sid).unwrap();

        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        // Should have original 6 + 1 compaction system message = 7
        assert_eq!(msgs.len(), 7);
        let last = msgs.last().unwrap();
        assert_eq!(last.role, "system");
        assert!(
            last.content.contains("[Conversation Summary"),
            "expected summary header"
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
            ironclad_db::sessions::append_message(&db, &sid, role, &format!("content-{i}"))
                .unwrap();
        }

        gov.compact_before_archive(&db, &sid).unwrap();

        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        let summary_msg = msgs
            .iter()
            .find(|m| m.content.contains("[Conversation Summary"))
            .unwrap();

        // The summary should reference content from trimmed messages (0..4)
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
            ironclad_db::sessions::append_message(&db, &sid, role, &format!("msg-{i}")).unwrap();
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
            ironclad_db::sessions::append_message(&db, &sid, role, &format!("stale-msg-{i}"))
                .unwrap();
        }

        // Allow the session to become stale
        std::thread::sleep(std::time::Duration::from_millis(50));

        let expired = gov.tick(&db).unwrap();
        assert_eq!(expired, 1);

        // Check the session is now expired
        let session = ironclad_db::sessions::get_session(&db, &sid)
            .unwrap()
            .unwrap();
        assert_eq!(session.status, "expired");

        // Compaction should have run — check for summary message
        let msgs = ironclad_db::sessions::list_messages(&db, &sid, Some(50)).unwrap();
        assert!(
            msgs.iter()
                .any(|m| m.content.contains("[Conversation Summary")),
            "compaction should have appended a summary"
        );
    }

    #[test]
    fn rotate_with_no_sessions_returns_zero() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();
        let rotated = gov
            .rotate_agent_scope_sessions(&db, "nonexistent-agent")
            .unwrap();
        assert_eq!(rotated, 0);
    }

    // ── Learned skill priority adjustment ─────────────────────────

    #[test]
    fn adjust_priorities_boosts_reliable_skills() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();

        // Create a skill with > 5 uses and > 80% success ratio
        ironclad_db::learned_skills::store_learned_skill(
            &db,
            "reliable-skill",
            "A reliable skill",
            "[]",
            "[]",
            None,
        )
        .unwrap();
        // Start at priority 50, success_count=1. Add more successes.
        for _ in 0..6 {
            ironclad_db::learned_skills::record_learned_skill_success(&db, "reliable-skill")
                .unwrap();
        }
        // Now: success_count=7, failure_count=0, ratio=1.0, total=7 > 5

        let adjusted = gov.adjust_learned_skill_priorities(&db).unwrap();
        assert_eq!(adjusted, 1);

        let skill = ironclad_db::learned_skills::get_learned_skill_by_name(&db, "reliable-skill")
            .unwrap()
            .unwrap();
        assert!(
            skill.priority > 50,
            "priority should have been boosted from 50, got {}",
            skill.priority
        );
    }

    #[test]
    fn adjust_priorities_decays_unreliable_skills() {
        let gov = SessionGovernor::new(SessionConfig::default());
        let db = test_db();

        ironclad_db::learned_skills::store_learned_skill(
            &db,
            "flaky-skill",
            "An unreliable skill",
            "[]",
            "[]",
            None,
        )
        .unwrap();
        // success_count=1. Add many failures so failure > success.
        for _ in 0..3 {
            ironclad_db::learned_skills::record_learned_skill_failure(&db, "flaky-skill").unwrap();
        }
        // Now: success_count=1, failure_count=3, failure > success

        let adjusted = gov.adjust_learned_skill_priorities(&db).unwrap();
        assert_eq!(adjusted, 1);

        let skill = ironclad_db::learned_skills::get_learned_skill_by_name(&db, "flaky-skill")
            .unwrap()
            .unwrap();
        assert!(
            skill.priority < 50,
            "priority should have decayed from 50, got {}",
            skill.priority
        );
    }

    #[test]
    fn adjust_priorities_disabled_config_skips() {
        let mut learning_config = LearningConfig::default();
        learning_config.enabled = false;
        let gov = SessionGovernor::new(SessionConfig::default())
            .with_learning(learning_config, PathBuf::from("/tmp"));
        let db = test_db();

        let adjusted = gov.adjust_learned_skill_priorities(&db).unwrap();
        assert_eq!(adjusted, 0);
    }
}
