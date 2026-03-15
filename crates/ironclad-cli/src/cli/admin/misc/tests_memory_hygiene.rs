    #[test]
    fn memory_hygiene_detects_contamination_without_repair() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");

        // Bootstrap a minimal DB with the three memory tables.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE working_memory (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE episodic_memory (
                id TEXT PRIMARY KEY,
                classification TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE semantic_memory (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(category, key)
            );",
        )
        .unwrap();

        // Insert contamination across all three tiers.
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w1', 's1', 'assistant', 'Duncan here. The prior generation degraded. Fallback.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w2', 's1', 'assistant', 'Duncan: by your command, proceeding.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w3', 's1', 'assistant', 'This is a legitimate working memory entry')",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO semantic_memory (id, category, key, value)
             VALUES ('sm1', 'learned', 'turn_42', 'Duncan here. The prior generation degraded.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO semantic_memory (id, category, key, value)
             VALUES ('sm2', 'learned', 'concept_rust', 'Rust is a systems programming language')",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e1', 'task', 'subtask 1 -> geopolitical-sitrep: fabricated content')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e2', 'task', 'subtask 1 -> moltbook-monitor: hallucinated data')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e3', 'observation', 'Legitimate episodic memory entry')",
            [],
        ).unwrap();
        drop(conn);

        // Diagnostic mode: detect but do NOT purge.
        let report = run_memory_hygiene(&db_path, false).unwrap();
        assert_eq!(report.working_canned, 2, "should detect 2 canned working entries");
        assert_eq!(report.semantic_canned, 1, "should detect 1 canned semantic entry");
        assert_eq!(report.episodic_hallucinated, 2, "should detect 2 hallucinated episodic entries");
        assert_eq!(report.total_detected, 5);
        assert_eq!(report.total_purged, 0, "diagnostic mode must not purge");

        // Verify rows are still intact.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let wm_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM working_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(wm_count, 3, "all working_memory rows should still exist");
        let em_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM episodic_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(em_count, 3, "all episodic_memory rows should still exist");
    }

    #[test]
    fn memory_hygiene_purges_contamination_on_repair() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE working_memory (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE episodic_memory (
                id TEXT PRIMARY KEY,
                classification TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE semantic_memory (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(category, key)
            );",
        )
        .unwrap();

        // Insert 3 toxic + 1 clean per tier.
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w1', 's1', 'assistant', 'Duncan here. The prior generation degraded. Fallback.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w2', 's1', 'assistant', 'Duncan here. I rejected a low-value response.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w3', 's1', 'assistant', 'Active path confirmed. No detours needed.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w4', 's1', 'user', 'Legitimate user request')",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO semantic_memory (id, category, key, value)
             VALUES ('sm1', 'learned', 'turn_7', 'Duncan: by your command, adjusting.')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO semantic_memory (id, category, key, value)
             VALUES ('sm2', 'learned', 'concept_sql', 'SQL is a query language')",
            [],
        ).unwrap();

        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e1', 'task', 'subtask 1 -> geopolitical-sitrep: Iran tensions')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e2', 'observation', 'Clean episodic memory')",
            [],
        ).unwrap();
        drop(conn);

        // Repair mode: detect AND purge.
        let report = run_memory_hygiene(&db_path, true).unwrap();
        assert_eq!(report.working_canned, 3);
        assert_eq!(report.semantic_canned, 1);
        assert_eq!(report.episodic_hallucinated, 1);
        assert_eq!(report.total_detected, 5);
        assert_eq!(report.total_purged, 5, "repair mode should purge all detected");

        // Verify only clean rows remain.
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        let wm_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM working_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(wm_count, 1, "only the legitimate entry should survive");

        let sm_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM semantic_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(sm_count, 1, "only the clean semantic entry should survive");

        let em_count: u32 = conn
            .query_row("SELECT COUNT(*) FROM episodic_memory", [], |r| r.get(0))
            .unwrap();
        assert_eq!(em_count, 1, "only the clean episodic entry should survive");
    }

    #[test]
    fn memory_hygiene_clean_db_returns_zeros() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");

        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE working_memory (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                entry_type TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE episodic_memory (
                id TEXT PRIMARY KEY,
                classification TEXT NOT NULL,
                content TEXT NOT NULL,
                importance INTEGER NOT NULL DEFAULT 5,
                created_at TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE TABLE semantic_memory (
                id TEXT PRIMARY KEY,
                category TEXT NOT NULL,
                key TEXT NOT NULL,
                value TEXT NOT NULL,
                confidence REAL NOT NULL DEFAULT 0.8,
                created_at TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at TEXT NOT NULL DEFAULT (datetime('now')),
                UNIQUE(category, key)
            );",
        )
        .unwrap();

        // Insert only clean data.
        conn.execute(
            "INSERT INTO working_memory (id, session_id, entry_type, content)
             VALUES ('w1', 's1', 'user', 'Normal user message')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO semantic_memory (id, category, key, value)
             VALUES ('sm1', 'learned', 'concept_ai', 'AI stands for artificial intelligence')",
            [],
        ).unwrap();
        conn.execute(
            "INSERT INTO episodic_memory (id, classification, content)
             VALUES ('e1', 'observation', 'User completed a deployment successfully')",
            [],
        ).unwrap();
        drop(conn);

        let report = run_memory_hygiene(&db_path, false).unwrap();
        assert_eq!(report.total_detected, 0);
        assert_eq!(report.total_purged, 0);
        assert_eq!(report.working_canned, 0);
        assert_eq!(report.semantic_canned, 0);
        assert_eq!(report.episodic_hallucinated, 0);
    }

    #[test]
    fn memory_hygiene_missing_db_returns_default() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("nonexistent.db");

        let report = run_memory_hygiene(&db_path, true).unwrap();
        assert_eq!(report.total_detected, 0);
        assert_eq!(report.total_purged, 0);
    }
