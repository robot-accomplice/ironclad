use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use regex::Regex;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tracing;

use ironclad_core::config::ObsidianConfig;
use ironclad_core::{IroncladError, Result};

use crate::knowledge::{KnowledgeChunk, KnowledgeSource};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed `[[target|display]]` wikilink with optional heading anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WikiLink {
    pub target: String,
    pub display: Option<String>,
    pub heading: Option<String>,
}

/// A parsed Obsidian note with metadata extracted from frontmatter and content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObsidianNote {
    pub path: PathBuf,
    pub title: String,
    pub content: String,
    pub frontmatter: Option<serde_yaml::Value>,
    pub tags: Vec<String>,
    #[serde(skip)]
    pub outgoing_links: Vec<String>,
    pub created_at: Option<String>,
    pub modified_at: Option<String>,
}

/// The vault manager — handles scanning, indexing, wikilink resolution,
/// backlink tracking, template rendering, and note I/O.
pub struct ObsidianVault {
    pub root: PathBuf,
    pub vault_name: String,
    config: ObsidianConfig,
    notes: HashMap<String, ObsidianNote>,
    /// Lowercase note title -> relative path (for case-insensitive resolution).
    name_index: HashMap<String, PathBuf>,
    /// Normalized target -> list of source note relative paths.
    backlink_index: HashMap<String, Vec<String>>,
}

impl ObsidianVault {
    /// Construct a vault from config. Discovers the vault root (explicit or auto-detect)
    /// and optionally scans on creation.
    pub fn from_config(config: &ObsidianConfig) -> Result<Self> {
        let root = if let Some(ref explicit) = config.vault_path {
            explicit.clone()
        } else if config.auto_detect {
            auto_detect_vault(&config.auto_detect_paths)?
        } else {
            return Err(IroncladError::Config(
                "obsidian.vault_path must be set, or enable auto_detect with auto_detect_paths"
                    .into(),
            ));
        };

        if !root.exists() {
            return Err(IroncladError::Config(format!(
                "obsidian vault path does not exist: {}",
                root.display()
            )));
        }

        let vault_name = root
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("vault")
            .to_string();

        let mut vault = Self {
            root,
            vault_name,
            config: config.clone(),
            notes: HashMap::new(),
            name_index: HashMap::new(),
            backlink_index: HashMap::new(),
        };

        if config.index_on_start {
            vault.scan()?;
        }

        Ok(vault)
    }

    /// Recursively scan the vault, respecting `ignored_folders`.
    pub fn scan(&mut self) -> Result<()> {
        self.notes.clear();
        self.name_index.clear();
        self.backlink_index.clear();

        let mut files = Vec::new();
        self.collect_markdown_files(&self.root.clone(), &mut files);

        for path in files {
            if let Ok(note) = self.parse_note(&path) {
                let rel = self.relative_path(&path);
                let key = rel.to_string_lossy().to_string();

                let title_lower = note.title.to_lowercase();
                let existing = self.name_index.get(&title_lower);
                if existing.is_none()
                    || existing.is_some_and(|e| key.len() < e.to_string_lossy().len())
                {
                    self.name_index.insert(title_lower, PathBuf::from(&key));
                }

                self.notes.insert(key, note);
            }
        }

        self.rebuild_backlinks();

        tracing::info!(
            vault = %self.vault_name,
            notes = self.notes.len(),
            "Obsidian vault scanned"
        );

        Ok(())
    }

    fn collect_markdown_files(&self, dir: &Path, out: &mut Vec<PathBuf>) {
        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let dir_name = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or_default();
                if !self.config.ignored_folders.iter().any(|f| f == dir_name) {
                    self.collect_markdown_files(&path, out);
                }
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.push(path);
            }
        }
    }

    fn parse_note(&self, path: &Path) -> Result<ObsidianNote> {
        let raw = std::fs::read_to_string(path).map_err(|e| {
            IroncladError::Config(format!("failed to read {}: {e}", path.display()))
        })?;

        let (frontmatter, content) = parse_frontmatter(&raw);
        let tags = extract_tags(&frontmatter, content);
        let outgoing = parse_wikilink_targets(content);

        let title = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();

        let meta = std::fs::metadata(path).ok();
        let modified_at = meta.as_ref().and_then(|m| m.modified().ok()).map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        });
        let created_at = meta.as_ref().and_then(|m| m.created().ok()).map(|t| {
            chrono::DateTime::<chrono::Utc>::from(t)
                .format("%Y-%m-%dT%H:%M:%S")
                .to_string()
        });

        Ok(ObsidianNote {
            path: path.to_path_buf(),
            title,
            content: content.to_string(),
            frontmatter,
            tags,
            outgoing_links: outgoing,
            created_at,
            modified_at,
        })
    }

    fn rebuild_backlinks(&mut self) {
        self.backlink_index.clear();
        for (source_key, note) in &self.notes {
            for target in &note.outgoing_links {
                let normalized = target.to_lowercase();
                self.backlink_index
                    .entry(normalized)
                    .or_default()
                    .push(source_key.clone());
            }
        }
    }

    fn relative_path(&self, path: &Path) -> PathBuf {
        path.strip_prefix(&self.root).unwrap_or(path).to_path_buf()
    }

    // -- Public API --

    pub fn get_note(&self, rel_path: &str) -> Option<&ObsidianNote> {
        self.notes.get(rel_path)
    }

    pub fn search_by_tag(&self, tag: &str) -> Vec<&ObsidianNote> {
        let tag_lower = tag.to_lowercase();
        self.notes
            .values()
            .filter(|n| n.tags.iter().any(|t| t.to_lowercase() == tag_lower))
            .collect()
    }

    pub fn search_by_content(
        &self,
        query: &str,
        max_results: usize,
    ) -> Vec<(&str, &ObsidianNote, f64)> {
        let query_lower = query.to_lowercase();
        let mut results: Vec<(&str, &ObsidianNote, f64)> = self
            .notes
            .iter()
            .filter_map(|(key, note)| {
                let content_lower = note.content.to_lowercase();
                let title_lower = note.title.to_lowercase();

                let content_hits = content_lower.matches(&query_lower).count();
                let title_hit = if title_lower.contains(&query_lower) {
                    1.0
                } else {
                    0.0
                };

                if content_hits == 0 && title_hit == 0.0 {
                    return None;
                }

                let content_score = content_hits as f64 / note.content.len().max(1) as f64;

                let tag_boost = if note
                    .tags
                    .iter()
                    .any(|t| t.to_lowercase().contains(&query_lower))
                {
                    self.config.tag_boost
                } else {
                    0.0
                };

                let backlink_count = self.backlinks_for_key(key).len() as f64;
                let backlink_boost = (backlink_count / 10.0).min(0.2);

                let score = content_score + title_hit * 0.5 + tag_boost + backlink_boost;

                Some((key.as_str(), note, score))
            })
            .collect();

        results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(max_results);
        results
    }

    /// Resolve a wikilink target to a relative path (case-insensitive).
    pub fn resolve_wikilink(&self, target: &str) -> Option<PathBuf> {
        let normalized = target.split('#').next().unwrap_or(target);
        let normalized = normalized.split('|').next().unwrap_or(normalized);
        let lower = normalized.to_lowercase().trim().to_string();

        if let Some(path) = self.name_index.get(&lower) {
            return Some(path.clone());
        }

        if lower.contains('/')
            && let path @ Some(_) = self.notes.get(&lower).map(|_| PathBuf::from(&lower))
        {
            return path;
        }

        None
    }

    pub fn backlinks_for(&self, note_path: &str) -> Vec<&ObsidianNote> {
        self.backlinks_for_key(note_path)
            .into_iter()
            .filter_map(|k| self.notes.get(k))
            .collect()
    }

    fn backlinks_for_key(&self, key: &str) -> Vec<&str> {
        let title = Path::new(key)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(key)
            .to_lowercase();

        self.backlink_index
            .get(&title)
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Write a note to the vault. Creates parent directories and prepends YAML frontmatter.
    pub fn write_note(
        &mut self,
        rel_path: &str,
        content: &str,
        frontmatter: Option<serde_json::Value>,
    ) -> Result<PathBuf> {
        let path = if rel_path.contains('/') || rel_path.contains('\\') {
            self.root.join(rel_path)
        } else {
            self.root.join(&self.config.default_folder).join(rel_path)
        };

        let path = if path.extension().is_none() {
            path.with_extension("md")
        } else {
            path
        };

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| IroncladError::Config(format!("failed to create dirs: {e}")))?;
        }

        let mut file_content = String::new();

        let fm = if let Some(extra) = frontmatter {
            let mut map = match extra {
                serde_json::Value::Object(m) => m,
                _ => serde_json::Map::new(),
            };
            map.entry("created_by")
                .or_insert(serde_json::Value::String("ironclad".into()));
            map.entry("created_at").or_insert(serde_json::Value::String(
                chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            ));
            Some(serde_json::Value::Object(map))
        } else {
            Some(serde_json::json!({
                "created_by": "ironclad",
                "created_at": chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            }))
        };

        if let Some(ref fm_val) = fm
            && let Ok(yaml) = serde_yaml::to_string(fm_val)
        {
            file_content.push_str("---\n");
            file_content.push_str(&yaml);
            file_content.push_str("---\n\n");
        }

        file_content.push_str(content);

        std::fs::write(&path, &file_content)
            .map_err(|e| IroncladError::Config(format!("failed to write note: {e}")))?;

        // Re-parse and index the new note
        if let Ok(note) = self.parse_note(&path) {
            let rel = self.relative_path(&path);
            let key = rel.to_string_lossy().to_string();
            let title_lower = note.title.to_lowercase();
            self.name_index.insert(title_lower, PathBuf::from(&key));
            self.notes.insert(key, note);
            self.rebuild_backlinks();
        }

        Ok(path)
    }

    /// Apply a template by substituting `{{variable}}` placeholders.
    pub fn apply_template(
        &self,
        template_name: &str,
        vars: &HashMap<String, String>,
    ) -> Result<String> {
        let template_dir = self.root.join(&self.config.template_folder);
        let template_path = template_dir.join(template_name);
        let template_path = if template_path.extension().is_none() {
            template_path.with_extension("md")
        } else {
            template_path
        };

        if !template_path.exists() {
            return Err(IroncladError::Config(format!(
                "template not found: {}",
                template_path.display()
            )));
        }

        let raw = std::fs::read_to_string(&template_path)
            .map_err(|e| IroncladError::Config(format!("failed to read template: {e}")))?;

        let mut result = raw;
        for (key, value) in vars {
            let placeholder = format!("{{{{{key}}}}}");
            result = result.replace(&placeholder, value);
        }

        // Built-in variables
        result = result.replace(
            "{{date}}",
            &chrono::Utc::now().format("%Y-%m-%d").to_string(),
        );
        result = result.replace(
            "{{time}}",
            &chrono::Utc::now().format("%H:%M:%S").to_string(),
        );

        Ok(result)
    }

    /// Generate an `obsidian://` URI for a note.
    pub fn obsidian_uri(&self, note_rel_path: &str) -> String {
        let vault_encoded = urlencoding::encode(&self.vault_name);
        let file = note_rel_path.strip_suffix(".md").unwrap_or(note_rel_path);
        let file_encoded = urlencoding::encode(file);
        format!("obsidian://open?vault={vault_encoded}&file={file_encoded}")
    }

    pub fn note_count(&self) -> usize {
        self.notes.len()
    }

    pub fn all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .notes
            .values()
            .flat_map(|n| n.tags.iter().cloned())
            .collect();
        tags.sort();
        tags.dedup();
        tags
    }

    pub fn notes_in_folder(&self, folder: &str) -> Vec<(&str, &ObsidianNote)> {
        self.notes
            .iter()
            .filter(|(k, _)| k.starts_with(folder))
            .map(|(k, v)| (k.as_str(), v))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Auto-detect
// ---------------------------------------------------------------------------

fn auto_detect_vault(search_paths: &[PathBuf]) -> Result<PathBuf> {
    for base in search_paths {
        if let Some(found) = find_obsidian_dir(base) {
            tracing::info!(vault = %found.display(), "Auto-detected Obsidian vault");
            return Ok(found);
        }
    }
    Err(IroncladError::Config(
        "auto_detect enabled but no .obsidian directory found in specified paths".into(),
    ))
}

fn find_obsidian_dir(base: &Path) -> Option<PathBuf> {
    if !base.is_dir() {
        return None;
    }

    if base.join(".obsidian").is_dir() {
        return Some(base.to_path_buf());
    }

    let entries = std::fs::read_dir(base).ok()?;
    let mut candidates = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join(".obsidian").is_dir() {
            candidates.push(path);
        }
    }

    if candidates.len() > 1 {
        tracing::warn!(
            count = candidates.len(),
            "Multiple Obsidian vaults found, using shortest path"
        );
        candidates.sort_by_key(|p| p.to_string_lossy().len());
    }

    candidates.into_iter().next()
}

// ---------------------------------------------------------------------------
// Frontmatter / tag / wikilink parsing
// ---------------------------------------------------------------------------

fn parse_frontmatter(raw: &str) -> (Option<serde_yaml::Value>, &str) {
    if !raw.starts_with("---") {
        return (None, raw);
    }

    if let Some(end) = raw[3..].find("\n---") {
        let yaml_str = &raw[3..3 + end];
        let rest_start = 3 + end + 4; // skip past "\n---"
        let rest = if rest_start < raw.len() {
            raw[rest_start..].trim_start_matches('\n')
        } else {
            ""
        };

        match serde_yaml::from_str(yaml_str) {
            Ok(val) => (Some(val), rest),
            Err(_) => (None, raw),
        }
    } else {
        (None, raw)
    }
}

fn extract_tags(frontmatter: &Option<serde_yaml::Value>, content: &str) -> Vec<String> {
    let mut tags = Vec::new();

    // Tags from frontmatter
    if let Some(fm) = frontmatter
        && let Some(fm_tags) = fm.get("tags")
    {
        match fm_tags {
            serde_yaml::Value::Sequence(seq) => {
                for item in seq {
                    if let Some(s) = item.as_str() {
                        tags.push(s.to_string());
                    }
                }
            }
            serde_yaml::Value::String(s) => {
                for tag in s.split(',') {
                    let trimmed = tag.trim();
                    if !trimmed.is_empty() {
                        tags.push(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    // Inline #tags from content
    let tag_re = Regex::new(r"(?:^|\s)#([a-zA-Z][\w/-]*)").expect("valid regex");
    for cap in tag_re.captures_iter(content) {
        if let Some(m) = cap.get(1) {
            let tag = m.as_str().to_string();
            if !tags.contains(&tag) {
                tags.push(tag);
            }
        }
    }

    tags
}

/// Parse all wikilink targets from content (just the target names, not display text).
fn parse_wikilink_targets(content: &str) -> Vec<String> {
    let link_re = Regex::new(r"\[\[([^\]]+)\]\]").expect("valid regex");
    let mut targets = Vec::new();

    for cap in link_re.captures_iter(content) {
        if let Some(inner) = cap.get(1) {
            let raw = inner.as_str();
            let target = raw.split('|').next().unwrap_or(raw);
            let target = target.split('#').next().unwrap_or(target);
            let target = target.trim().to_string();
            if !target.is_empty() && !targets.contains(&target) {
                targets.push(target);
            }
        }
    }

    targets
}

/// Parse a wikilink string into a structured `WikiLink`.
pub fn parse_wikilink(raw: &str) -> WikiLink {
    let inner = raw.trim_start_matches("[[").trim_end_matches("]]");
    let (target_part, display) = if let Some(idx) = inner.find('|') {
        (&inner[..idx], Some(inner[idx + 1..].to_string()))
    } else {
        (inner, None)
    };

    let (target, heading) = if let Some(idx) = target_part.find('#') {
        (
            target_part[..idx].to_string(),
            Some(target_part[idx + 1..].to_string()),
        )
    } else {
        (target_part.to_string(), None)
    };

    WikiLink {
        target,
        display,
        heading,
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let boundary = s.floor_char_boundary(max);
        format!("{}...", &s[..boundary])
    }
}

// ---------------------------------------------------------------------------
// KnowledgeSource implementation
// ---------------------------------------------------------------------------

pub struct ObsidianSource {
    vault: Arc<RwLock<ObsidianVault>>,
}

impl ObsidianSource {
    pub fn new(vault: Arc<RwLock<ObsidianVault>>) -> Self {
        Self { vault }
    }
}

#[async_trait]
impl KnowledgeSource for ObsidianSource {
    fn name(&self) -> &str {
        "obsidian"
    }

    fn source_type(&self) -> &str {
        "obsidian"
    }

    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>> {
        let vault = self.vault.read().await;
        let results = vault.search_by_content(query, max_results);

        Ok(results
            .into_iter()
            .map(|(key, note, score)| {
                let mut metadata = serde_json::json!({
                    "path": key,
                    "title": note.title,
                    "tags": note.tags,
                });

                if let Some(ref fm) = note.frontmatter {
                    metadata["frontmatter"] = serde_json::to_value(fm).unwrap_or_default();
                }

                let backlink_count = vault.backlinks_for(key).len();
                metadata["backlink_count"] = serde_json::json!(backlink_count);

                let obsidian_uri = vault.obsidian_uri(key);
                metadata["obsidian_uri"] = serde_json::json!(obsidian_uri);

                KnowledgeChunk {
                    content: truncate(&note.content, 2000),
                    source: format!("obsidian://{}", key),
                    relevance: score,
                    metadata: Some(metadata),
                }
            })
            .collect())
    }

    async fn ingest(&self, content: &str, source: &str) -> Result<()> {
        let mut vault = self.vault.write().await;
        let path = source.strip_prefix("obsidian://").unwrap_or(source);
        vault.write_note(path, content, None)?;
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

// ---------------------------------------------------------------------------
// File watcher (feature-gated behind "vault-watcher")
// ---------------------------------------------------------------------------

#[cfg(feature = "vault-watcher")]
pub mod watcher {
    use std::sync::Arc;
    use std::time::Duration;

    use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
    use tokio::sync::RwLock;
    use tokio::sync::mpsc;

    use super::ObsidianVault;

    pub struct VaultWatcher {
        _watcher: RecommendedWatcher,
    }

    impl VaultWatcher {
        /// Spawn a file watcher that re-scans the vault on changes.
        /// Uses a 500ms debounce to avoid thrashing during bulk edits.
        pub fn start(vault: Arc<RwLock<ObsidianVault>>) -> Result<Self, notify::Error> {
            let (tx, mut rx) = mpsc::channel::<()>(16);

            let vault_root = {
                let v = vault.blocking_read();
                v.root.clone()
            };

            let mut watcher = notify::recommended_watcher(move |res: Result<Event, _>| {
                if let Ok(event) = res {
                    match event.kind {
                        EventKind::Create(_) | EventKind::Modify(_) | EventKind::Remove(_) => {
                            let _ = tx.try_send(());
                        }
                        _ => {}
                    }
                }
            })?;

            watcher.watch(&vault_root, RecursiveMode::Recursive)?;

            let debounce_vault = Arc::clone(&vault);
            tokio::spawn(async move {
                let debounce = Duration::from_millis(500);
                loop {
                    if rx.recv().await.is_none() {
                        break;
                    }
                    // Drain any buffered events during debounce window
                    tokio::time::sleep(debounce).await;
                    while rx.try_recv().is_ok() {}

                    let mut v = debounce_vault.write().await;
                    if let Err(e) = v.scan() {
                        tracing::warn!(error = %e, "Vault re-scan after file change failed");
                    } else {
                        tracing::debug!(
                            notes = v.note_count(),
                            "Vault re-scanned after file change"
                        );
                    }
                }
            });

            tracing::info!("Obsidian vault file watcher started");

            Ok(Self { _watcher: watcher })
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn create_test_vault() -> (TempDir, ObsidianConfig) {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".obsidian")).unwrap();
        fs::create_dir(dir.path().join("templates")).unwrap();
        fs::create_dir(dir.path().join("ironclad")).unwrap();

        let config = ObsidianConfig {
            enabled: true,
            vault_path: Some(dir.path().to_path_buf()),
            index_on_start: false,
            ..Default::default()
        };

        (dir, config)
    }

    #[test]
    fn parse_frontmatter_with_tags() {
        let raw = "---\ntags:\n  - rust\n  - coding\ntitle: Test\n---\n\nHello world";
        let (fm, content) = parse_frontmatter(raw);
        assert!(fm.is_some());
        assert_eq!(content, "Hello world");
        let tags = extract_tags(&fm, content);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"coding".to_string()));
    }

    #[test]
    fn parse_frontmatter_none_without_dashes() {
        let raw = "No frontmatter here";
        let (fm, content) = parse_frontmatter(raw);
        assert!(fm.is_none());
        assert_eq!(content, "No frontmatter here");
    }

    #[test]
    fn extract_inline_tags() {
        let content = "Hello #rust and #coding are great. Not# a tag.";
        let tags = extract_tags(&None, content);
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"coding".to_string()));
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn parse_wikilink_simple() {
        let link = parse_wikilink("[[My Note]]");
        assert_eq!(link.target, "My Note");
        assert!(link.display.is_none());
        assert!(link.heading.is_none());
    }

    #[test]
    fn parse_wikilink_with_display() {
        let link = parse_wikilink("[[Target|Display Text]]");
        assert_eq!(link.target, "Target");
        assert_eq!(link.display.as_deref(), Some("Display Text"));
    }

    #[test]
    fn parse_wikilink_with_heading() {
        let link = parse_wikilink("[[Note#Section]]");
        assert_eq!(link.target, "Note");
        assert_eq!(link.heading.as_deref(), Some("Section"));
    }

    #[test]
    fn parse_wikilink_targets_from_content() {
        let content = "See [[Note A]] and [[Note B|alias]] and [[Note A]] again.";
        let targets = parse_wikilink_targets(content);
        assert_eq!(targets, vec!["Note A", "Note B"]);
    }

    #[test]
    fn vault_scan_and_search() {
        let (dir, config) = create_test_vault();
        fs::write(
            dir.path().join("alpha.md"),
            "---\ntags:\n  - rust\n---\n\nRust programming notes",
        )
        .unwrap();
        fs::write(dir.path().join("beta.md"), "Python programming notes").unwrap();
        fs::write(
            dir.path().join("gamma.md"),
            "See [[alpha]] for Rust details",
        )
        .unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        assert_eq!(vault.note_count(), 3);

        let results = vault.search_by_content("Rust", 10);
        assert!(!results.is_empty());
        assert!(results[0].1.content.contains("Rust"));

        let by_tag = vault.search_by_tag("rust");
        assert_eq!(by_tag.len(), 1);
        assert_eq!(by_tag[0].title, "alpha");
    }

    #[test]
    fn wikilink_resolution() {
        let (dir, config) = create_test_vault();
        fs::write(dir.path().join("My Note.md"), "Content here").unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        assert!(vault.resolve_wikilink("My Note").is_some());
        assert!(vault.resolve_wikilink("my note").is_some());
        assert!(vault.resolve_wikilink("Nonexistent").is_none());
    }

    #[test]
    fn backlink_index_built() {
        let (dir, config) = create_test_vault();
        fs::write(dir.path().join("target.md"), "I am the target").unwrap();
        fs::write(dir.path().join("source.md"), "Linking to [[target]] here").unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        let backlinks = vault.backlinks_for("target.md");
        assert_eq!(backlinks.len(), 1);
        assert_eq!(backlinks[0].title, "source");
    }

    #[test]
    fn write_note_creates_file() {
        let (_dir, config) = create_test_vault();
        let mut vault = ObsidianVault::from_config(&config).unwrap();

        let result = vault.write_note("test-note", "Hello from Ironclad", None);
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("Hello from Ironclad"));
        assert!(content.contains("created_by: ironclad"));
    }

    #[test]
    fn write_note_with_frontmatter() {
        let (_dir, config) = create_test_vault();
        let mut vault = ObsidianVault::from_config(&config).unwrap();

        let fm = serde_json::json!({
            "tags": ["test", "demo"],
            "status": "draft"
        });

        let path = vault
            .write_note("custom.md", "Custom content", Some(fm))
            .unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("custom content") || content.contains("Custom content"));
        assert!(content.contains("created_by"));
    }

    #[test]
    fn template_application() {
        let (dir, config) = create_test_vault();
        fs::write(
            dir.path().join("templates/daily.md"),
            "# {{title}}\n\nDate: {{date}}\n\n## Notes\n",
        )
        .unwrap();

        let vault = ObsidianVault::from_config(&config).unwrap();
        let mut vars = HashMap::new();
        vars.insert("title".into(), "My Daily Note".into());

        let result = vault.apply_template("daily", &vars).unwrap();
        assert!(result.contains("# My Daily Note"));
        assert!(result.contains("Date:"));
        assert!(!result.contains("{{title}}"));
        assert!(!result.contains("{{date}}"));
    }

    #[test]
    fn template_missing_error() {
        let (_dir, config) = create_test_vault();
        let vault = ObsidianVault::from_config(&config).unwrap();
        let result = vault.apply_template("nonexistent", &HashMap::new());
        assert!(result.is_err());
    }

    #[test]
    fn obsidian_uri_generation() {
        let (_dir, config) = create_test_vault();
        let vault = ObsidianVault::from_config(&config).unwrap();

        let uri = vault.obsidian_uri("folder/My Note.md");
        assert!(uri.starts_with("obsidian://open?vault="));
        assert!(uri.contains("file="));
        assert!(!uri.contains(".md"));
    }

    #[test]
    fn auto_detect_finds_vault() {
        let dir = TempDir::new().unwrap();
        let vault_dir = dir.path().join("MyVault");
        fs::create_dir(&vault_dir).unwrap();
        fs::create_dir(vault_dir.join(".obsidian")).unwrap();

        let result = auto_detect_vault(&[dir.path().to_path_buf()]);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), vault_dir);
    }

    #[test]
    fn auto_detect_no_vault_errors() {
        let dir = TempDir::new().unwrap();
        let result = auto_detect_vault(&[dir.path().to_path_buf()]);
        assert!(result.is_err());
    }

    #[test]
    fn ignored_folders_respected() {
        let (dir, config) = create_test_vault();
        fs::create_dir(dir.path().join(".trash")).unwrap();
        fs::write(dir.path().join(".trash/deleted.md"), "deleted note").unwrap();
        fs::write(dir.path().join("visible.md"), "visible note").unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        assert_eq!(vault.note_count(), 1);
        assert!(vault.get_note("visible.md").is_some());
    }

    #[test]
    fn all_tags_deduped() {
        let (dir, config) = create_test_vault();
        fs::write(
            dir.path().join("a.md"),
            "---\ntags:\n  - rust\n  - coding\n---\nContent",
        )
        .unwrap();
        fs::write(
            dir.path().join("b.md"),
            "---\ntags:\n  - rust\n  - docs\n---\nMore content",
        )
        .unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        let tags = vault.all_tags();
        assert!(tags.contains(&"rust".to_string()));
        assert!(tags.contains(&"coding".to_string()));
        assert!(tags.contains(&"docs".to_string()));
        assert_eq!(tags.iter().filter(|t| *t == "rust").count(), 1);
    }

    #[tokio::test]
    async fn obsidian_source_query() {
        let (dir, config) = create_test_vault();
        fs::write(
            dir.path().join("knowledge.md"),
            "Important Rust knowledge about ownership",
        )
        .unwrap();

        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();

        let vault = Arc::new(RwLock::new(vault));
        let source = ObsidianSource::new(vault);

        let chunks = source.query("Rust", 5).await.unwrap();
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].content.contains("Rust"));
        assert!(chunks[0].source.starts_with("obsidian://"));
    }

    #[tokio::test]
    async fn obsidian_source_ingest() {
        let (dir, config) = create_test_vault();
        let mut vault = ObsidianVault::from_config(&config).unwrap();
        vault.scan().unwrap();
        let vault = Arc::new(RwLock::new(vault));
        let source = ObsidianSource::new(vault);

        source
            .ingest("New note content", "obsidian://ingested-note")
            .await
            .unwrap();

        let written = dir.path().join("ironclad/ingested-note.md");
        assert!(written.exists());
    }
}
