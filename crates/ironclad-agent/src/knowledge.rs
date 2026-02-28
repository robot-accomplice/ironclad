use async_trait::async_trait;
use ironclad_core::{IroncladError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// A chunk of knowledge retrieved from a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeChunk {
    pub content: String,
    pub source: String,
    pub relevance: f64,
    pub metadata: Option<serde_json::Value>,
}

/// Configuration for a knowledge source.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeSourceConfig {
    pub name: String,
    pub source_type: String,
    pub path: Option<PathBuf>,
    pub url: Option<String>,
    pub max_chunks: usize,
}

/// Trait for external knowledge sources the agent can query.
#[async_trait]
pub trait KnowledgeSource: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> &str;
    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>>;
    async fn ingest(&self, content: &str, source: &str) -> Result<()>;
    fn is_available(&self) -> bool;
}

/// A knowledge source backed by a local directory of files.
pub struct DirectorySource {
    name: String,
    root: PathBuf,
    extensions: Vec<String>,
}

impl DirectorySource {
    pub fn new(name: &str, root: PathBuf) -> Self {
        Self {
            name: name.to_string(),
            root,
            extensions: vec![
                "md".into(),
                "txt".into(),
                "rs".into(),
                "py".into(),
                "js".into(),
                "ts".into(),
                "toml".into(),
                "yaml".into(),
                "json".into(),
            ],
        }
    }

    pub fn with_extensions(mut self, exts: Vec<String>) -> Self {
        self.extensions = exts;
        self
    }

    fn is_supported_extension(&self, path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| self.extensions.iter().any(|ext| ext == e))
            .unwrap_or(false)
    }

    /// Scan directory for files matching supported extensions.
    pub fn scan_files(&self) -> Vec<PathBuf> {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&self.root) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_file() && self.is_supported_extension(&path) {
                    files.push(path);
                } else if path.is_dir()
                    && let Ok(sub) = std::fs::read_dir(&path)
                {
                    for sub_entry in sub.flatten() {
                        let sub_path = sub_entry.path();
                        if sub_path.is_file() && self.is_supported_extension(&sub_path) {
                            files.push(sub_path);
                        }
                    }
                }
            }
        }
        files
    }
}

#[async_trait]
impl KnowledgeSource for DirectorySource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> &str {
        "directory"
    }

    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>> {
        let query_lower = query.to_lowercase();
        let files = self.scan_files();

        let chunks = tokio::task::spawn_blocking(move || {
            let mut chunks = Vec::new();
            for path in files {
                // Cap file reads at 10 MB to prevent OOM on huge files.
                const MAX_FILE_BYTES: u64 = 10 * 1024 * 1024;
                if let Ok(content) = (|| -> std::io::Result<String> {
                    use std::io::Read;
                    let file = std::fs::File::open(&path)?;
                    let meta = file.metadata()?;
                    if meta.len() > MAX_FILE_BYTES {
                        return Err(std::io::Error::other("file too large for knowledge query"));
                    }
                    let mut buf = String::new();
                    file.take(MAX_FILE_BYTES).read_to_string(&mut buf)?;
                    Ok(buf)
                })() {
                    let content_lower = content.to_lowercase();
                    if content_lower.contains(&query_lower) {
                        let relevance = content_lower.matches(&query_lower).count() as f64
                            / content.len().max(1) as f64;
                        chunks.push(KnowledgeChunk {
                            content: truncate(&content, 2000),
                            source: path.display().to_string(),
                            relevance,
                            metadata: Some(serde_json::json!({
                                "file_size": content.len(),
                                "path": path.display().to_string(),
                            })),
                        });
                    }
                }
            }
            chunks.sort_by(|a, b| {
                b.relevance
                    .partial_cmp(&a.relevance)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            chunks.truncate(max_results);
            chunks
        })
        .await
        .map_err(|e| IroncladError::Config(format!("blocking task failed: {e}")))?;

        Ok(chunks)
    }

    async fn ingest(&self, _content: &str, _source: &str) -> Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        self.root.exists() && self.root.is_dir()
    }
}

/// A knowledge source backed by a Git repository.
pub struct GitSource {
    name: String,
    repo_path: PathBuf,
    inner: DirectorySource,
}

impl GitSource {
    pub fn new(name: &str, repo_path: PathBuf) -> Self {
        let inner = DirectorySource::new(name, repo_path.clone());
        Self {
            name: name.to_string(),
            repo_path,
            inner,
        }
    }

    /// Check if the path is a Git repository.
    pub fn is_git_repo(&self) -> bool {
        self.repo_path.join(".git").exists()
    }
}

#[async_trait]
impl KnowledgeSource for GitSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> &str {
        "git"
    }

    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>> {
        self.inner.query(query, max_results).await
    }

    async fn ingest(&self, _content: &str, _source: &str) -> Result<()> {
        Ok(())
    }

    fn is_available(&self) -> bool {
        self.is_git_repo()
    }
}

/// A knowledge source backed by a vector database HTTP API.
pub struct VectorDbSource {
    name: String,
    url: String,
    http: reqwest::Client,
    api_key: Option<String>,
}

impl VectorDbSource {
    pub fn new(name: &str, url: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            url: url.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| IroncladError::Config(format!("HTTP client build failed: {e}")))?,
            api_key: None,
        })
    }

    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }
}

#[derive(Deserialize)]
struct VectorQueryResult {
    #[serde(default)]
    content: String,
    #[serde(default)]
    source: String,
    #[serde(default)]
    relevance: f64,
}

#[async_trait]
impl KnowledgeSource for VectorDbSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> &str {
        "vector_db"
    }

    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>> {
        let url = format!("{}/query", self.url);
        let body = serde_json::json!({
            "query": query,
            "top_k": max_results,
        });

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("vector DB query failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!(
                "vector DB returned {status}: {body}"
            )));
        }

        let results: Vec<VectorQueryResult> = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("vector DB response parse error: {e}")))?;

        Ok(results
            .into_iter()
            .map(|r| KnowledgeChunk {
                content: r.content,
                source: r.source,
                relevance: r.relevance,
                metadata: None,
            })
            .collect())
    }

    async fn ingest(&self, content: &str, source: &str) -> Result<()> {
        let url = format!("{}/upsert", self.url);
        let body = serde_json::json!({
            "documents": [{
                "content": content,
                "source": source,
            }],
        });

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("vector DB ingest failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!(
                "vector DB ingest returned {status}: {body}"
            )));
        }

        Ok(())
    }

    fn is_available(&self) -> bool {
        !self.url.is_empty()
    }
}

/// A knowledge source backed by a Neo4j graph database.
pub struct GraphSource {
    name: String,
    url: String,
    http: reqwest::Client,
    api_key: Option<String>,
}

impl GraphSource {
    pub fn new(name: &str, url: &str) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            url: url.to_string(),
            http: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .map_err(|e| IroncladError::Config(format!("HTTP client build failed: {e}")))?,
            api_key: None,
        })
    }

    pub fn with_api_key(mut self, key: String) -> Self {
        self.api_key = Some(key);
        self
    }
}

#[async_trait]
impl KnowledgeSource for GraphSource {
    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> &str {
        "graph"
    }

    async fn query(&self, query: &str, max_results: usize) -> Result<Vec<KnowledgeChunk>> {
        let url = format!("{}/db/neo4j/tx/commit", self.url);
        let cypher = "MATCH (n) WHERE n.content CONTAINS $query RETURN n.content AS content, \
             n.source AS source, 1.0 AS relevance LIMIT $limit"
            .to_string();
        let body = serde_json::json!({
            "statements": [{
                "statement": cypher,
                "parameters": {
                    "query": query,
                    "limit": max_results,
                },
            }],
        });

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("graph DB query failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!(
                "graph DB returned {status}: {body}"
            )));
        }

        let json: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| IroncladError::Network(format!("graph DB response parse error: {e}")))?;

        let mut chunks = Vec::new();
        if let Some(results) = json.get("results").and_then(|r| r.as_array()) {
            for result in results {
                if let Some(data) = result.get("data").and_then(|d| d.as_array()) {
                    for row in data {
                        if let Some(row_vals) = row.get("row").and_then(|r| r.as_array()) {
                            let content = row_vals
                                .first()
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let source = row_vals
                                .get(1)
                                .and_then(|v| v.as_str())
                                .unwrap_or_default()
                                .to_string();
                            let relevance = row_vals.get(2).and_then(|v| v.as_f64()).unwrap_or(0.0);

                            chunks.push(KnowledgeChunk {
                                content,
                                source,
                                relevance,
                                metadata: None,
                            });
                        }
                    }
                }
            }
        }

        Ok(chunks)
    }

    async fn ingest(&self, content: &str, source: &str) -> Result<()> {
        let url = format!("{}/db/neo4j/tx/commit", self.url);
        let body = serde_json::json!({
            "statements": [{
                "statement": "MERGE (n:Knowledge {source: $source}) SET n.content = $content",
                "parameters": {
                    "content": content,
                    "source": source,
                },
            }],
        });

        let mut req = self.http.post(&url).json(&body);
        if let Some(key) = &self.api_key {
            req = req.bearer_auth(key);
        }

        let resp = req
            .send()
            .await
            .map_err(|e| IroncladError::Network(format!("graph DB ingest failed: {e}")))?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(IroncladError::Network(format!(
                "graph DB ingest returned {status}: {body}"
            )));
        }

        Ok(())
    }

    fn is_available(&self) -> bool {
        !self.url.is_empty()
    }
}

/// Registry of all knowledge sources.
pub struct KnowledgeRegistry {
    sources: Vec<Box<dyn KnowledgeSource>>,
}

impl KnowledgeRegistry {
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
        }
    }

    pub fn add(&mut self, source: Box<dyn KnowledgeSource>) {
        self.sources.push(source);
    }

    pub fn list(&self) -> Vec<(&str, &str, bool)> {
        self.sources
            .iter()
            .map(|s| (s.name(), s.source_type(), s.is_available()))
            .collect()
    }

    pub async fn query_all(&self, query: &str, max_per_source: usize) -> Vec<KnowledgeChunk> {
        let mut all_chunks = Vec::new();
        for source in &self.sources {
            if source.is_available() {
                match source.query(query, max_per_source).await {
                    Ok(chunks) => all_chunks.extend(chunks),
                    Err(e) => tracing::warn!(
                        source = %source.name(),
                        error = %e,
                        "knowledge query failed"
                    ),
                }
            }
        }
        all_chunks.sort_by(|a, b| {
            b.relevance
                .partial_cmp(&a.relevance)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        all_chunks
    }

    pub fn available_count(&self) -> usize {
        self.sources.iter().filter(|s| s.is_available()).count()
    }
}

impl Default for KnowledgeRegistry {
    fn default() -> Self {
        Self::new()
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn directory_source_scan_finds_files() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("readme.md"), "# Hello").unwrap();
        fs::write(dir.path().join("code.rs"), "fn main() {}").unwrap();
        fs::write(dir.path().join("image.png"), "binary").unwrap();

        let source = DirectorySource::new("test", dir.path().to_path_buf());
        let files = source.scan_files();
        assert_eq!(files.len(), 2);
    }

    #[test]
    fn directory_source_not_available_for_missing_dir() {
        let source = DirectorySource::new("test", PathBuf::from("/nonexistent/path"));
        assert!(!source.is_available());
    }

    #[tokio::test]
    async fn directory_source_query_finds_matching_content() {
        let dir = TempDir::new().unwrap();
        fs::write(
            dir.path().join("notes.md"),
            "Rust is a systems programming language",
        )
        .unwrap();
        fs::write(dir.path().join("other.txt"), "Python is interpreted").unwrap();

        let source = DirectorySource::new("test", dir.path().to_path_buf());
        let results = source.query("Rust", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert!(results[0].content.contains("Rust"));
    }

    #[tokio::test]
    async fn directory_source_query_empty_for_no_match() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("notes.md"), "Hello world").unwrap();

        let source = DirectorySource::new("test", dir.path().to_path_buf());
        let results = source.query("nonexistent_query_term", 10).await.unwrap();
        assert!(results.is_empty());
    }

    #[test]
    fn git_source_detects_repo() {
        let dir = TempDir::new().unwrap();
        fs::create_dir(dir.path().join(".git")).unwrap();

        let source = GitSource::new("test", dir.path().to_path_buf());
        assert!(source.is_git_repo());
        assert!(source.is_available());
    }

    #[test]
    fn git_source_not_repo() {
        let dir = TempDir::new().unwrap();
        let source = GitSource::new("test", dir.path().to_path_buf());
        assert!(!source.is_git_repo());
        assert!(!source.is_available());
    }

    #[test]
    fn vector_db_source_available_with_url() {
        let source = VectorDbSource::new("pinecone", "https://pinecone.io").unwrap();
        assert!(source.is_available());
        assert_eq!(source.source_type(), "vector_db");
    }

    #[test]
    fn vector_db_source_not_available_empty_url() {
        let source = VectorDbSource::new("empty", "").unwrap();
        assert!(!source.is_available());
    }

    #[test]
    fn vector_db_source_with_api_key() {
        let source = VectorDbSource::new("pinecone", "https://pinecone.io")
            .unwrap()
            .with_api_key("sk-test".to_string());
        assert!(source.api_key.is_some());
    }

    #[test]
    fn graph_source_available_with_url() {
        let source = GraphSource::new("neo4j", "http://localhost:7474").unwrap();
        assert!(source.is_available());
        assert_eq!(source.source_type(), "graph");
    }

    #[test]
    fn graph_source_with_api_key() {
        let source = GraphSource::new("neo4j", "http://localhost:7474")
            .unwrap()
            .with_api_key("token".to_string());
        assert!(source.api_key.is_some());
    }

    #[test]
    fn registry_empty() {
        let reg = KnowledgeRegistry::new();
        assert_eq!(reg.available_count(), 0);
        assert!(reg.list().is_empty());
    }

    #[test]
    fn registry_lists_sources() {
        let dir = TempDir::new().unwrap();
        let mut reg = KnowledgeRegistry::new();
        reg.add(Box::new(DirectorySource::new(
            "docs",
            dir.path().to_path_buf(),
        )));
        reg.add(Box::new(
            VectorDbSource::new("pinecone", "https://api.pinecone.io").unwrap(),
        ));

        let list = reg.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].0, "docs");
        assert_eq!(list[1].0, "pinecone");
    }

    #[tokio::test]
    async fn registry_query_all_aggregates() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("file.md"), "knowledge about Rust").unwrap();

        let mut reg = KnowledgeRegistry::new();
        reg.add(Box::new(DirectorySource::new(
            "docs",
            dir.path().to_path_buf(),
        )));

        let results = reg.query_all("Rust", 5).await;
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn chunk_serialization() {
        let chunk = KnowledgeChunk {
            content: "test content".into(),
            source: "test.md".into(),
            relevance: 0.95,
            metadata: None,
        };
        let json = serde_json::to_string(&chunk).unwrap();
        let decoded: KnowledgeChunk = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded.content, "test content");
        assert_eq!(decoded.relevance, 0.95);
    }
}
