mod safety;
mod transform;

pub use safety::{
    FindingCategory, SafetyFinding, SafetyVerdict, Severity, SkillSafetyReport,
    scan_directory_safety, scan_script_safety,
};

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use transform::*;

// ── Public types ────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Import,
    Export,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MigrationArea {
    Config,
    Personality,
    Skills,
    Sessions,
    Cron,
    Channels,
    Agents,
}

impl MigrationArea {
    fn all() -> &'static [MigrationArea] {
        &[
            Self::Config,
            Self::Personality,
            Self::Skills,
            Self::Sessions,
            Self::Cron,
            Self::Channels,
            Self::Agents,
        ]
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "config" => Some(Self::Config),
            "personality" => Some(Self::Personality),
            "skills" => Some(Self::Skills),
            "sessions" => Some(Self::Sessions),
            "cron" => Some(Self::Cron),
            "channels" => Some(Self::Channels),
            "agents" => Some(Self::Agents),
            _ => None,
        }
    }

    fn label(&self) -> &'static str {
        match self {
            Self::Config => "Configuration",
            Self::Personality => "Personality",
            Self::Skills => "Skills",
            Self::Sessions => "Sessions",
            Self::Cron => "Cron Jobs",
            Self::Channels => "Channels",
            Self::Agents => "Sub-Agents",
        }
    }
}

#[derive(Debug, Clone)]
pub struct AreaResult {
    pub area: MigrationArea,
    pub success: bool,
    pub items_processed: usize,
    pub warnings: Vec<String>,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct MigrationReport {
    pub direction: Direction,
    pub source: PathBuf,
    pub results: Vec<AreaResult>,
}

impl MigrationReport {
    fn print(&self) {
        let dir_label = match self.direction {
            Direction::Import => "Import",
            Direction::Export => "Export",
        };
        eprintln!();
        eprintln!(
            "  \u{256d}\u{2500} Migration Report ({dir_label}) \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
        );
        eprintln!("  \u{2502} Source: {}", self.source.display());
        eprintln!("  \u{2502}");
        for r in &self.results {
            let icon = if r.success { "\u{2714}" } else { "\u{2718}" };
            eprintln!(
                "  \u{2502} {icon} {:<14} {} items",
                r.area.label(),
                r.items_processed
            );
            for w in &r.warnings {
                eprintln!("  \u{2502}   \u{26a0} {w}");
            }
            if let Some(e) = &r.error {
                eprintln!("  \u{2502}   \u{2718} {e}");
            }
        }
        let ok = self.results.iter().filter(|r| r.success).count();
        let total = self.results.len();
        eprintln!("  \u{2502}");
        eprintln!("  \u{2502} {ok}/{total} areas completed successfully");
        eprintln!(
            "  \u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
        );
        eprintln!();
    }
}

// ── Orchestrators ──────────────────────────────────────────────────────

fn resolve_areas(area_strs: &[String]) -> Vec<MigrationArea> {
    if area_strs.is_empty() {
        return MigrationArea::all().to_vec();
    }
    area_strs
        .iter()
        .filter_map(|s| MigrationArea::from_str(s))
        .collect()
}

pub fn cmd_migrate_import(
    source: &str,
    areas: &[String],
    yes: bool,
    no_safety_check: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_path = PathBuf::from(source);
    if !source_path.exists() {
        eprintln!("  \u{2718} Source path does not exist: {source}");
        return Ok(());
    }

    let ironclad_root = default_ironclad_root();
    let areas = resolve_areas(areas);

    eprintln!();
    eprintln!(
        "  \u{256d}\u{2500} OpenClaw \u{2192} Ironclad Import \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
    eprintln!("  \u{2502} Source: {}", source_path.display());
    eprintln!("  \u{2502} Target: {}", ironclad_root.display());
    eprintln!(
        "  \u{2502} Areas:  {}",
        areas
            .iter()
            .map(|a| a.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!(
        "  \u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );

    if !yes {
        eprint!("  Proceed? [y/N] ");
        let _ = io::stderr().flush();
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        if !input.trim().eq_ignore_ascii_case("y") {
            eprintln!("  Aborted.");
            return Ok(());
        }
    }

    let mut results = Vec::new();
    for area in &areas {
        eprint!("  \u{25b8} Importing {} ... ", area.label());
        let result = match area {
            MigrationArea::Config => import_config(&source_path, &ironclad_root),
            MigrationArea::Personality => import_personality(&source_path, &ironclad_root),
            MigrationArea::Skills => import_skills(&source_path, &ironclad_root, no_safety_check),
            MigrationArea::Sessions => import_sessions(&source_path, &ironclad_root),
            MigrationArea::Cron => import_cron(&source_path, &ironclad_root),
            MigrationArea::Channels => import_channels(&source_path, &ironclad_root),
            MigrationArea::Agents => import_agents(&source_path, &ironclad_root),
        };
        if result.success {
            eprintln!("\u{2714} ({} items)", result.items_processed);
        } else {
            eprintln!("\u{2718}");
        }
        results.push(result);
    }

    MigrationReport {
        direction: Direction::Import,
        source: source_path,
        results,
    }
    .print();
    Ok(())
}

pub fn cmd_migrate_export(
    target: &str,
    areas: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let target_path = PathBuf::from(target);
    let ironclad_root = default_ironclad_root();
    let areas = resolve_areas(areas);

    eprintln!();
    eprintln!(
        "  \u{256d}\u{2500} Ironclad \u{2192} OpenClaw Export \u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );
    eprintln!("  \u{2502} Source: {}", ironclad_root.display());
    eprintln!("  \u{2502} Target: {}", target_path.display());
    eprintln!(
        "  \u{2502} Areas:  {}",
        areas
            .iter()
            .map(|a| a.label())
            .collect::<Vec<_>>()
            .join(", ")
    );
    eprintln!(
        "  \u{2570}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}\u{2500}"
    );

    if let Err(e) = fs::create_dir_all(&target_path) {
        eprintln!("  \u{2718} Failed to create target directory: {e}");
        return Ok(());
    }

    let mut results = Vec::new();
    for area in &areas {
        eprint!("  \u{25b8} Exporting {} ... ", area.label());
        let result = match area {
            MigrationArea::Config => export_config(&ironclad_root, &target_path),
            MigrationArea::Personality => export_personality(&ironclad_root, &target_path),
            MigrationArea::Skills => export_skills(&ironclad_root, &target_path),
            MigrationArea::Sessions => export_sessions(&ironclad_root, &target_path),
            MigrationArea::Cron => export_cron(&ironclad_root, &target_path),
            MigrationArea::Channels => export_channels(&ironclad_root, &target_path),
            MigrationArea::Agents => export_agents(&ironclad_root, &target_path),
        };
        if result.success {
            eprintln!("\u{2714} ({} items)", result.items_processed);
        } else {
            eprintln!("\u{2718}");
        }
        results.push(result);
    }

    MigrationReport {
        direction: Direction::Export,
        source: ironclad_root,
        results,
    }
    .print();
    Ok(())
}

// ── Standalone skill import/export ─────────────────────────────────────

pub fn cmd_skill_import(
    source: &str,
    no_safety_check: bool,
    accept_warnings: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let source_path = PathBuf::from(source);
    if !source_path.exists() {
        eprintln!("  \u{2718} Source path does not exist: {source}");
        return Ok(());
    }

    eprintln!("  \u{25b8} Scanning skills from: {}", source_path.display());

    if !no_safety_check {
        let report = if source_path.is_dir() {
            scan_directory_safety(&source_path)
        } else {
            scan_script_safety(&source_path)
        };

        report.print();

        match &report.verdict {
            SafetyVerdict::Critical(_) => {
                eprintln!("  \u{2718} Import blocked due to critical safety findings.");
                eprintln!("    Use --no-safety-check to override (dangerous!).");
                return Ok(());
            }
            SafetyVerdict::Warnings(_) if !accept_warnings => {
                eprint!("  \u{26a0} Warnings found. Import anyway? [y/N] ");
                let _ = io::stderr().flush();
                let mut input = String::new();
                io::stdin().read_line(&mut input)?;
                if !input.trim().eq_ignore_ascii_case("y") {
                    eprintln!("  Aborted.");
                    return Ok(());
                }
            }
            _ => {}
        }
    }

    let ironclad_root = default_ironclad_root();
    let skills_dir = ironclad_root.join("skills");
    fs::create_dir_all(&skills_dir)?;

    let mut count = 0;
    if source_path.is_dir() {
        if let Ok(entries) = fs::read_dir(&source_path) {
            for entry in entries.flatten() {
                let src = entry.path();
                let dest = skills_dir.join(entry.file_name());
                if src.is_file() {
                    fs::copy(&src, &dest)?;
                    count += 1;
                } else if src.is_dir() {
                    copy_dir_recursive(&src, &dest)?;
                    count += 1;
                }
            }
        }
    } else {
        let dest = skills_dir.join(source_path.file_name().unwrap_or_default());
        fs::copy(&source_path, &dest)?;
        count = 1;
    }

    eprintln!(
        "  \u{2714} Imported {count} skill(s) to {}",
        skills_dir.display()
    );
    Ok(())
}

pub fn cmd_skill_export(output: &str, ids: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let ironclad_root = default_ironclad_root();
    let skills_dir = ironclad_root.join("skills");

    if !skills_dir.exists() {
        eprintln!(
            "  \u{2718} No skills directory found at {}",
            skills_dir.display()
        );
        return Ok(());
    }

    let output_path = PathBuf::from(output);
    fs::create_dir_all(&output_path)?;

    let mut count = 0;
    if let Ok(entries) = fs::read_dir(&skills_dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !ids.is_empty() && !ids.iter().any(|id| name.contains(id.as_str())) {
                continue;
            }
            let src = entry.path();
            let dest = output_path.join(entry.file_name());
            if src.is_file() {
                fs::copy(&src, &dest)?;
                count += 1;
            } else if src.is_dir() {
                copy_dir_recursive(&src, &dest)?;
                count += 1;
            }
        }
    }
    eprintln!(
        "  \u{2714} Exported {count} skill(s) to {}",
        output_path.display()
    );

    Ok(())
}

// ── Helpers ────────────────────────────────────────────────────────────

fn default_ironclad_root() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/tmp"))
        .join(".ironclad")
}

pub(crate) fn copy_dir_recursive(src: &Path, dst: &Path) -> io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let src_path = entry.path();
        let dst_path = dst.join(entry.file_name());
        if src_path.is_dir() {
            copy_dir_recursive(&src_path, &dst_path)?;
        } else {
            fs::copy(&src_path, &dst_path)?;
        }
    }
    Ok(())
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn resolve_areas_empty_returns_all() {
        assert_eq!(resolve_areas(&[]).len(), 7);
    }

    #[test]
    fn resolve_areas_specific() {
        let areas = resolve_areas(&["config".into(), "skills".into()]);
        assert_eq!(areas.len(), 2);
        assert!(areas.contains(&MigrationArea::Config));
        assert!(areas.contains(&MigrationArea::Skills));
    }

    #[test]
    fn resolve_areas_invalid_filtered() {
        assert_eq!(
            resolve_areas(&["config".into(), "nonsense".into()]).len(),
            1
        );
    }

    #[test]
    fn migration_area_labels() {
        assert_eq!(MigrationArea::Config.label(), "Configuration");
        assert_eq!(MigrationArea::Personality.label(), "Personality");
        assert_eq!(MigrationArea::Skills.label(), "Skills");
        assert_eq!(MigrationArea::Sessions.label(), "Sessions");
        assert_eq!(MigrationArea::Cron.label(), "Cron Jobs");
        assert_eq!(MigrationArea::Channels.label(), "Channels");
        assert_eq!(MigrationArea::Agents.label(), "Sub-Agents");
    }

    #[test]
    fn migration_area_from_str_valid() {
        assert_eq!(
            MigrationArea::from_str("config"),
            Some(MigrationArea::Config)
        );
        assert_eq!(
            MigrationArea::from_str("CONFIG"),
            Some(MigrationArea::Config)
        );
    }

    #[test]
    fn migration_area_from_str_invalid() {
        assert_eq!(MigrationArea::from_str("nonsense"), None);
    }

    #[test]
    fn copy_dir_recursive_works() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        fs::create_dir_all(src.path().join("sub")).unwrap();
        fs::write(src.path().join("a.txt"), "hello").unwrap();
        fs::write(src.path().join("sub/b.txt"), "world").unwrap();
        let target = dst.path().join("copy");
        copy_dir_recursive(src.path(), &target).unwrap();
        assert_eq!(fs::read_to_string(target.join("a.txt")).unwrap(), "hello");
        assert_eq!(
            fs::read_to_string(target.join("sub/b.txt")).unwrap(),
            "world"
        );
    }

    #[test]
    fn qt_escapes() {
        assert_eq!(transform::qt("hello"), "\"hello\"");
        assert_eq!(transform::qt("he\"llo"), "\"he\\\"llo\"");
    }

    #[test]
    fn migration_area_all_returns_seven() {
        assert_eq!(MigrationArea::all().len(), 7);
    }

    #[test]
    fn direction_debug_and_eq() {
        assert_eq!(Direction::Import, Direction::Import);
        assert_ne!(Direction::Import, Direction::Export);
        assert_eq!(format!("{:?}", Direction::Export), "Export");
    }

    #[test]
    fn migration_area_from_str_all_variants() {
        for s in &[
            "config",
            "personality",
            "skills",
            "sessions",
            "cron",
            "channels",
            "agents",
        ] {
            assert!(MigrationArea::from_str(s).is_some(), "failed for: {s}");
        }
    }

    #[test]
    fn migration_area_from_str_case_insensitive() {
        assert_eq!(
            MigrationArea::from_str("Personality"),
            Some(MigrationArea::Personality)
        );
        assert_eq!(
            MigrationArea::from_str("SESSIONS"),
            Some(MigrationArea::Sessions)
        );
        assert_eq!(MigrationArea::from_str("CrOn"), Some(MigrationArea::Cron));
    }

    #[test]
    fn area_result_construction() {
        let r = AreaResult {
            area: MigrationArea::Config,
            success: true,
            items_processed: 5,
            warnings: vec!["warn1".into()],
            error: None,
        };
        assert!(r.success);
        assert_eq!(r.items_processed, 5);
        assert_eq!(r.warnings.len(), 1);
        assert!(r.error.is_none());
    }

    #[test]
    fn area_result_failure() {
        let r = AreaResult {
            area: MigrationArea::Skills,
            success: false,
            items_processed: 0,
            warnings: vec![],
            error: Some("something broke".into()),
        };
        assert!(!r.success);
        assert_eq!(r.error.unwrap(), "something broke");
    }

    #[test]
    fn default_ironclad_root_contains_ironclad() {
        let root = default_ironclad_root();
        assert!(root.to_string_lossy().contains(".ironclad"));
    }

    #[test]
    fn copy_dir_recursive_empty_dir() {
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let target = dst.path().join("empty_copy");
        copy_dir_recursive(src.path(), &target).unwrap();
        assert!(target.exists());
    }

    #[test]
    fn resolve_areas_all_invalid_returns_empty() {
        let areas = resolve_areas(&["foo".into(), "bar".into()]);
        assert!(areas.is_empty());
    }

    #[test]
    fn migration_report_print_does_not_panic() {
        let report = MigrationReport {
            direction: Direction::Import,
            source: PathBuf::from("/tmp/test"),
            results: vec![
                AreaResult {
                    area: MigrationArea::Config,
                    success: true,
                    items_processed: 3,
                    warnings: vec!["minor issue".into()],
                    error: None,
                },
                AreaResult {
                    area: MigrationArea::Skills,
                    success: false,
                    items_processed: 0,
                    warnings: vec![],
                    error: Some("failed".into()),
                },
            ],
        };
        report.print();
    }

    #[test]
    fn qt_empty_string() {
        assert_eq!(transform::qt(""), "\"\"");
    }

    #[test]
    fn qt_backslash() {
        let result = transform::qt("a\\b");
        assert!(result.contains("\\\\"));
    }

    #[test]
    fn qt_ml_wraps_in_triple_quotes() {
        let result = transform::qt_ml("line1\nline2");
        assert!(result.starts_with("\"\"\"\n"));
        assert!(result.ends_with("\n\"\"\""));
        assert!(result.contains("line1\nline2"));
    }

    #[test]
    fn titlecase_single_word() {
        assert_eq!(transform::titlecase("hello"), "Hello");
    }

    #[test]
    fn titlecase_underscored() {
        assert_eq!(transform::titlecase("hello_world"), "Hello World");
    }

    #[test]
    fn titlecase_empty() {
        assert_eq!(transform::titlecase(""), "");
    }

    #[test]
    fn import_config_basic() {
        let oc = TempDir::new().unwrap();
        let ic = TempDir::new().unwrap();
        let config = serde_json::json!({
            "name": "TestBot",
            "model": "gpt-4"
        });
        fs::write(
            oc.path().join("openclaw.json"),
            serde_json::to_string(&config).unwrap(),
        )
        .unwrap();
        let r = transform::import_config(oc.path(), ic.path());
        assert!(r.success);
        assert!(ic.path().join("ironclad.toml").exists());
    }

    #[test]
    fn export_config_missing_toml_fails() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        let r = transform::export_config(ic.path(), oc.path());
        assert!(!r.success);
    }

    #[test]
    fn export_personality_missing_files_warns() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        fs::create_dir_all(ic.path().join("workspace")).unwrap();
        let r = transform::export_personality(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_sessions_no_database() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        let r = transform::export_sessions(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_cron_no_database() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        let r = transform::export_cron(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_skills_no_skills_dir() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        let r = transform::export_skills(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }

    #[test]
    fn export_channels_no_config() {
        let ic = TempDir::new().unwrap();
        let oc = TempDir::new().unwrap();
        let r = transform::export_channels(ic.path(), oc.path());
        assert!(r.success);
        assert_eq!(r.items_processed, 0);
    }
}
