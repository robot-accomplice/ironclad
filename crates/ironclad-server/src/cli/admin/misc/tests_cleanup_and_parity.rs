    use super::*;
    use crate::test_support::EnvGuard;
    #[test]
    fn path_contains_dir_and_go_bin_detection() {
        let dir = tempfile::tempdir().unwrap();
        let path_var = std::ffi::OsString::from(dir.path().to_str().unwrap());
        assert!(path_contains_dir_in(dir.path(), &path_var));
        assert!(!path_contains_dir_in(
            Path::new("/definitely/not/here"),
            &path_var
        ));

        let gopath = tempfile::tempdir().unwrap();
        let bin_dir = gopath.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        #[cfg(windows)]
        let gosh = bin_dir.join("gosh.exe");
        #[cfg(not(windows))]
        let gosh = bin_dir.join("gosh");
        std::fs::write(&gosh, "stub").unwrap();
        assert_eq!(
            find_gosh_in_go_bins_with(gopath.path().to_str()),
            Some(gosh)
        );
    }
    #[test]
    fn recent_log_snapshot_and_count_occurrences_work() {
        let dir = tempfile::tempdir().unwrap();
        let older = dir.path().join("ironclad.log");
        let newer = dir.path().join("ironclad.stderr.log");
        std::fs::write(&older, "old line").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&newer, "abc abc abc").unwrap();

        let snap = recent_log_snapshot(dir.path(), 8).unwrap();
        assert!(snap.contains("abc"));
        assert_eq!(count_occurrences("abc abc abc", "abc"), 3);
    }

    #[test]
    fn finding_builder_sets_repair_metadata() {
        let f = finding(
            "id-1",
            "high",
            0.9,
            "summary",
            "details",
            "plan",
            vec!["cmd".into()],
            true,
            false,
        );
        assert_eq!(f.id, "id-1");
        assert_eq!(f.severity, "high");
        assert!(f.repair_plan.safe_auto_repair);
        assert!(!f.repair_plan.requires_human_approval);
        assert_eq!(f.repair_plan.commands, vec!["cmd"]);
    }

    #[test]
    fn cleanup_internalized_skill_artifacts_detects_db_and_filesystem_drift() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("workflow-design.md"), "# legacy").unwrap();
        std::fs::write(skills_dir.join("hello.md"), "# deprecated").unwrap();
        std::fs::create_dir_all(skills_dir.join("fast-cache")).unwrap();

        let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
        ironclad_db::skills::register_skill(
            &db,
            "workflow-design",
            "instruction",
            Some("legacy externalized form"),
            "/tmp/workflow-design.md",
            "h1",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        ironclad_db::skills::register_skill(
            &db,
            "hello",
            "instruction",
            Some("deprecated generic skill"),
            "/tmp/hello.md",
            "h2",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();

        let report = cleanup_internalized_skill_artifacts(&db_path, &skills_dir, false);
        assert!(
            report
                .stale_db_skills
                .iter()
                .any(|s| s.eq_ignore_ascii_case("workflow-design"))
        );
        assert!(
            report
                .stale_db_skills
                .iter()
                .any(|s| s.eq_ignore_ascii_case("hello"))
        );
        assert!(
            report
                .stale_files
                .iter()
                .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("workflow-design.md"))
        );
        assert!(
            report
                .stale_dirs
                .iter()
                .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("fast-cache"))
        );
        assert!(
            report
                .stale_files
                .iter()
                .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("hello.md"))
        );
        assert!(report.removed_db_skills.is_empty());
        assert!(report.removed_paths.is_empty());
    }

    #[test]
    fn cleanup_internalized_skill_artifacts_repair_removes_drift() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("state.db");
        let skills_dir = dir.path().join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(skills_dir.join("session-operator.md"), "# legacy").unwrap();
        std::fs::write(skills_dir.join("search.md"), "# deprecated").unwrap();

        let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
        let skill_id = ironclad_db::skills::register_skill(
            &db,
            "session-operator",
            "instruction",
            Some("legacy externalized form"),
            "/tmp/session-operator.md",
            "h1",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let deprecated_id = ironclad_db::skills::register_skill(
            &db,
            "search",
            "instruction",
            Some("deprecated generic skill"),
            "/tmp/search.md",
            "h2",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        assert!(
            ironclad_db::skills::get_skill(&db, &skill_id)
                .unwrap()
                .is_some()
        );
        assert!(
            ironclad_db::skills::get_skill(&db, &deprecated_id)
                .unwrap()
                .is_some()
        );

        let report = cleanup_internalized_skill_artifacts(&db_path, &skills_dir, true);
        assert!(
            report
                .removed_db_skills
                .iter()
                .any(|s| s.eq_ignore_ascii_case("session-operator"))
        );
        assert!(
            report
                .removed_db_skills
                .iter()
                .any(|s| s.eq_ignore_ascii_case("search"))
        );
        assert!(
            report
                .removed_paths
                .iter()
                .any(|p| { p.file_name().and_then(|n| n.to_str()) == Some("session-operator.md") })
        );
        assert!(
            report
                .removed_paths
                .iter()
                .any(|p| p.file_name().and_then(|n| n.to_str()) == Some("search.md"))
        );
        assert!(!skills_dir.join("session-operator.md").exists());
        assert!(!skills_dir.join("search.md").exists());
        let db = ironclad_db::Database::new(db_path.to_string_lossy().as_ref()).unwrap();
        assert!(
            ironclad_db::skills::get_skill(&db, &skill_id)
                .unwrap()
                .is_none()
        );
    assert!(
        ironclad_db::skills::get_skill(&db, &deprecated_id)
            .unwrap()
            .is_none()
    );
}
