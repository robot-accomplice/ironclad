//! Document ingestion pipeline: file → parse → chunk → embed → store.
//!
//! Supports `.md`, `.txt`, `.rs`, `.py`, `.js`, `.ts`, `.pdf` files.
//! PDF parsing uses the `pdf-extract` crate (pure Rust, no C dependencies).
//!
//! The pipeline:
//! 1. Detect file type by extension
//! 2. Extract raw text (plain-text passthrough, or PDF text extraction)
//! 3. Chunk using existing `ChunkConfig` (512 tokens, 64-token overlap)
//! 4. Store each chunk as semantic memory + embedding entry
//! 5. Register the document as a knowledge source in hippocampus

use std::path::Path;

use ironclad_core::Result;
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::retrieval::{ChunkConfig, chunk_text};

// ── File type detection ────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FileType {
    Markdown,
    PlainText,
    RustSource,
    PythonSource,
    JavaScriptSource,
    TypeScriptSource,
    Pdf,
}

impl FileType {
    /// Detect file type from extension. Returns `None` for unsupported types.
    pub fn from_path(path: &Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        match ext.as_str() {
            "md" | "markdown" => Some(Self::Markdown),
            "txt" | "text" => Some(Self::PlainText),
            "rs" => Some(Self::RustSource),
            "py" => Some(Self::PythonSource),
            "js" | "jsx" | "mjs" => Some(Self::JavaScriptSource),
            "ts" | "tsx" | "mts" => Some(Self::TypeScriptSource),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }

    pub fn is_code(&self) -> bool {
        matches!(
            self,
            Self::RustSource | Self::PythonSource | Self::JavaScriptSource | Self::TypeScriptSource
        )
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::PlainText => "plain_text",
            Self::RustSource => "rust",
            Self::PythonSource => "python",
            Self::JavaScriptSource => "javascript",
            Self::TypeScriptSource => "typescript",
            Self::Pdf => "pdf",
        }
    }
}

// ── Text extraction ────────────────────────────────────────────

/// Extract raw text from a file. For text-based formats, reads UTF-8 content
/// directly. For PDF, extracts text using pdf-extract.
pub fn extract_text(path: &Path, file_type: FileType) -> Result<String> {
    match file_type {
        FileType::Pdf => extract_pdf_text(path),
        _ => {
            let content = std::fs::read_to_string(path).map_err(|e| {
                ironclad_core::IroncladError::Config(format!(
                    "failed to read {}: {e}",
                    path.display()
                ))
            })?;
            Ok(content)
        }
    }
}

fn extract_pdf_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|e| {
        ironclad_core::IroncladError::Config(format!("failed to read PDF {}: {e}", path.display()))
    })?;
    let text = pdf_extract::extract_text_from_mem(&bytes).map_err(|e| {
        ironclad_core::IroncladError::Config(format!(
            "failed to extract text from PDF {}: {e}",
            path.display()
        ))
    })?;
    Ok(text)
}

// ── Ingestion result ───────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestResult {
    pub file_path: String,
    pub file_type: FileType,
    pub chunks_stored: usize,
    pub total_chars: usize,
    pub source_id: String,
}

// ── Pipeline ───────────────────────────────────────────────────

/// Maximum file size we'll ingest (10 MB). Prevents OOM on giant files.
const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;

/// Ingest a single file into the knowledge system.
///
/// Steps:
/// 1. Validate file exists and is within size limits
/// 2. Detect file type
/// 3. Extract text
/// 4. Chunk with standard config (512 tokens, 64-token overlap)
/// 5. Store each chunk as semantic memory + embedding entry
/// 6. Register in hippocampus as a knowledge source
pub fn ingest_file(db: &ironclad_db::Database, path: &Path) -> Result<IngestResult> {
    // Validate
    let metadata = std::fs::metadata(path).map_err(|e| {
        ironclad_core::IroncladError::Config(format!("cannot access {}: {e}", path.display()))
    })?;

    if !metadata.is_file() {
        return Err(ironclad_core::IroncladError::Config(format!(
            "{} is not a regular file",
            path.display()
        )));
    }

    if metadata.len() > MAX_FILE_SIZE {
        return Err(ironclad_core::IroncladError::Config(format!(
            "{} exceeds maximum file size ({} bytes > {} bytes)",
            path.display(),
            metadata.len(),
            MAX_FILE_SIZE
        )));
    }

    let file_type = FileType::from_path(path).ok_or_else(|| {
        ironclad_core::IroncladError::Config(format!("unsupported file type: {}", path.display()))
    })?;

    // Extract text
    let text = extract_text(path, file_type)?;
    let total_chars = text.len();

    if text.trim().is_empty() {
        return Err(ironclad_core::IroncladError::Config(format!(
            "{} contains no extractable text",
            path.display()
        )));
    }

    // Chunk
    let config = ChunkConfig::default(); // 512 tokens, 64 overlap
    let chunks = chunk_text(&text, &config);

    // Generate a stable source ID from the file path
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let source_id = format!(
        "ingest:{}",
        canonical.to_string_lossy().replace(['/', '\\'], ":")
    );

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown");

    // Store each chunk
    let mut stored = 0;
    for chunk in &chunks {
        let chunk_id = format!("{}:chunk:{}", source_id, chunk.index);
        let preview = if chunk.text.len() > 200 {
            format!("{}...", &chunk.text[..chunk.text.floor_char_boundary(200)])
        } else {
            chunk.text.clone()
        };

        // Store in semantic memory for FTS5 retrieval
        let category = if file_type.is_code() {
            "ingested_code"
        } else {
            "ingested_document"
        };
        let key = format!("{}:{}", file_name, chunk.index);

        if let Err(e) = ironclad_db::memory::store_semantic(db, category, &key, &chunk.text, 0.8) {
            warn!(error = %e, chunk = chunk.index, "failed to store semantic memory for chunk");
            continue;
        }

        // Store embedding entry (with zero-vector placeholder — actual embedding
        // requires an inference call which is async and model-dependent; the
        // embedding will be populated lazily on first retrieval or by a background
        // job).
        let placeholder_embedding: Vec<f32> = Vec::new();
        if let Err(e) = ironclad_db::embeddings::store_embedding(
            db,
            &chunk_id,
            "ingested_knowledge",
            &source_id,
            &preview,
            &placeholder_embedding,
        ) {
            warn!(error = %e, chunk = chunk.index, "failed to store embedding entry for chunk");
            continue;
        }

        stored += 1;
    }

    // Register in hippocampus as a knowledge source
    let description = format!(
        "Ingested {} ({}, {} chunks)",
        file_name,
        file_type.label(),
        stored
    );
    if let Err(e) = ironclad_db::hippocampus::register_table(
        db,
        &format!("knowledge:{}", file_name),
        &description,
        &[],      // no column schema — knowledge sources aren't relational tables
        "system", // created_by
        false,    // not agent-owned — system knowledge
        "read",   // access_level
        stored as i64,
    ) {
        warn!(error = %e, "failed to register ingested document in hippocampus");
    }

    Ok(IngestResult {
        file_path: path.display().to_string(),
        file_type,
        chunks_stored: stored,
        total_chars,
        source_id,
    })
}

/// Ingest all supported files in a directory (non-recursive).
pub fn ingest_directory(db: &ironclad_db::Database, dir: &Path) -> Result<Vec<IngestResult>> {
    if !dir.is_dir() {
        return Err(ironclad_core::IroncladError::Config(format!(
            "{} is not a directory",
            dir.display()
        )));
    }

    let mut results = Vec::new();
    let entries = std::fs::read_dir(dir).map_err(|e| {
        ironclad_core::IroncladError::Config(format!(
            "cannot read directory {}: {e}",
            dir.display()
        ))
    })?;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && FileType::from_path(&path).is_some() {
            match ingest_file(db, &path) {
                Ok(result) => results.push(result),
                Err(e) => {
                    warn!(
                        error = %e,
                        file = %path.display(),
                        "skipping file during directory ingestion"
                    );
                }
            }
        }
    }

    Ok(results)
}

// ── Tests ──────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_db() -> ironclad_db::Database {
        let db = ironclad_db::Database::new(":memory:").unwrap();
        ironclad_db::schema::initialize_db(&db).unwrap();
        db
    }

    #[test]
    fn file_type_detection() {
        assert_eq!(
            FileType::from_path(Path::new("readme.md")),
            Some(FileType::Markdown)
        );
        assert_eq!(
            FileType::from_path(Path::new("main.rs")),
            Some(FileType::RustSource)
        );
        assert_eq!(
            FileType::from_path(Path::new("app.tsx")),
            Some(FileType::TypeScriptSource)
        );
        assert_eq!(
            FileType::from_path(Path::new("doc.pdf")),
            Some(FileType::Pdf)
        );
        assert_eq!(FileType::from_path(Path::new("image.png")), None);
        assert_eq!(FileType::from_path(Path::new("archive.zip")), None);
    }

    #[test]
    fn ingest_markdown_file() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("test.md");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            writeln!(
                f,
                "# Test Document\n\nThis is a test document with enough content to be meaningful."
            )
            .unwrap();
            writeln!(
                f,
                "\n## Section Two\n\nMore content here for the chunker to work with."
            )
            .unwrap();
        }

        let result = ingest_file(&db, &file_path).unwrap();
        assert_eq!(result.file_type, FileType::Markdown);
        assert!(result.chunks_stored > 0);
        assert!(result.total_chars > 50);
        assert!(result.source_id.starts_with("ingest:"));
    }

    #[test]
    fn ingest_code_file() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("example.rs");
        {
            let mut f = std::fs::File::create(&file_path).unwrap();
            writeln!(f, "fn main() {{").unwrap();
            writeln!(f, "    println!(\"Hello, world!\");").unwrap();
            writeln!(f, "}}").unwrap();
        }

        let result = ingest_file(&db, &file_path).unwrap();
        assert_eq!(result.file_type, FileType::RustSource);
        assert_eq!(result.chunks_stored, 1); // small file = 1 chunk
    }

    #[test]
    fn ingest_empty_file_fails() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("empty.txt");
        std::fs::File::create(&file_path).unwrap();

        let err = ingest_file(&db, &file_path).unwrap_err();
        assert!(err.to_string().contains("no extractable text"));
    }

    #[test]
    fn ingest_unsupported_extension_fails() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("photo.png");
        std::fs::write(&file_path, b"fake png data").unwrap();

        let err = ingest_file(&db, &file_path).unwrap_err();
        assert!(err.to_string().contains("unsupported file type"));
    }

    #[test]
    fn ingest_directory_collects_supported_files() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();

        // Create some supported files
        std::fs::write(
            dir.path().join("a.md"),
            "# Doc A\nSome markdown content here.",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("b.txt"),
            "Plain text content for ingestion.",
        )
        .unwrap();
        // Unsupported file — should be skipped
        std::fs::write(dir.path().join("c.png"), b"fake image").unwrap();

        let results = ingest_directory(&db, dir.path()).unwrap();
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn hippocampus_registration_after_ingest() {
        let db = test_db();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("notes.md");
        std::fs::write(&file_path, "# My Notes\nImportant information.").unwrap();

        ingest_file(&db, &file_path).unwrap();

        // Verify hippocampus has the entry
        let tables = ironclad_db::hippocampus::list_tables(&db).unwrap();
        let found = tables.iter().any(|t| t.table_name == "knowledge:notes.md");
        assert!(
            found,
            "ingested document should be registered in hippocampus"
        );
    }
}
