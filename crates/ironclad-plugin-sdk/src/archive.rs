//! Plugin archive (.ic.zip) creation, extraction, and verification.
//!
//! An `.ic.zip` is a standard ZIP archive whose root contains `plugin.toml`.
//! The naming convention is `<name>-<version>.ic.zip`.

use std::io::Write;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::manifest::PluginManifest;

/// Result of packing a plugin directory into an archive.
#[derive(Debug)]
pub struct PackResult {
    /// Path to the created `.ic.zip` file.
    pub archive_path: PathBuf,
    /// SHA-256 hex digest of the archive bytes.
    pub sha256: String,
    /// Plugin name from the manifest.
    pub name: String,
    /// Plugin version from the manifest.
    pub version: String,
    /// Number of files included in the archive.
    pub file_count: usize,
    /// Total uncompressed size in bytes.
    pub uncompressed_bytes: u64,
}

/// Result of unpacking an archive to a staging directory.
#[derive(Debug)]
pub struct UnpackResult {
    /// Directory where the plugin was extracted.
    pub dest_dir: PathBuf,
    /// Parsed manifest from the extracted `plugin.toml`.
    pub manifest: PluginManifest,
    /// SHA-256 hex digest of the archive bytes (for verification).
    pub sha256: String,
    /// Number of files extracted.
    pub file_count: usize,
}

/// Errors specific to archive operations.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ZIP error: {0}")]
    Zip(#[from] zip::result::ZipError),
    #[error("manifest error: {0}")]
    Manifest(String),
    #[error("archive verification failed: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("archive missing plugin.toml at root")]
    MissingManifest,
    #[error("path traversal detected in archive entry: {0}")]
    PathTraversal(String),
}

/// Compute SHA-256 hex digest of a byte slice.
pub fn sha256_hex(data: &[u8]) -> String {
    let hash = Sha256::digest(data);
    hex::encode(hash)
}

/// Compute SHA-256 hex digest of a file.
pub fn file_sha256(path: &Path) -> Result<String, ArchiveError> {
    let bytes = std::fs::read(path)?;
    Ok(sha256_hex(&bytes))
}

/// Pack a plugin directory into a `.ic.zip` archive.
///
/// The archive is written to `output_dir/<name>-<version>.ic.zip`.
/// The plugin directory must contain a valid `plugin.toml` at its root.
pub fn pack(plugin_dir: &Path, output_dir: &Path) -> Result<PackResult, ArchiveError> {
    // Parse and validate manifest first
    let manifest_path = plugin_dir.join("plugin.toml");
    if !manifest_path.exists() {
        return Err(ArchiveError::MissingManifest);
    }
    let manifest = PluginManifest::from_file(&manifest_path)
        .map_err(|e| ArchiveError::Manifest(e.to_string()))?;

    let archive_name = format!("{}-{}.ic.zip", manifest.name, manifest.version);
    let archive_path = output_dir.join(&archive_name);

    std::fs::create_dir_all(output_dir)?;

    // Collect all files relative to plugin_dir
    let mut entries: Vec<(PathBuf, PathBuf)> = Vec::new(); // (absolute, relative)
    collect_files(plugin_dir, plugin_dir, &mut entries)?;

    // Create zip
    let file = std::fs::File::create(&archive_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let mut uncompressed_bytes: u64 = 0;

    for (abs_path, rel_path) in &entries {
        let rel_str = rel_path.to_string_lossy().replace('\\', "/");
        zip.start_file(&rel_str, options)?;
        let data = std::fs::read(abs_path)?;
        uncompressed_bytes += data.len() as u64;
        zip.write_all(&data)?;
    }

    zip.finish()?;

    // Compute checksum of the final archive
    let sha256 = file_sha256(&archive_path)?;

    Ok(PackResult {
        archive_path,
        sha256,
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        file_count: entries.len(),
        uncompressed_bytes,
    })
}

/// Unpack a `.ic.zip` archive into a destination directory.
///
/// Extracts to `dest_dir/<plugin-name>/`. Validates the manifest after extraction.
/// Returns the parsed manifest and checksum for verification.
pub fn unpack(archive_path: &Path, dest_dir: &Path) -> Result<UnpackResult, ArchiveError> {
    let archive_bytes = std::fs::read(archive_path)?;
    let sha256 = sha256_hex(&archive_bytes);

    unpack_bytes(&archive_bytes, dest_dir, sha256)
}

/// Unpack from in-memory bytes (useful after downloading).
pub fn unpack_bytes(
    data: &[u8],
    dest_dir: &Path,
    sha256: String,
) -> Result<UnpackResult, ArchiveError> {
    let cursor = std::io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)?;

    // First pass: verify plugin.toml exists and check for path traversal
    let mut has_manifest = false;
    for i in 0..archive.len() {
        let entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        // Security: reject path traversal attempts (covers Unix and Windows patterns)
        if name.contains("..")
            || name.starts_with('/')
            || name.starts_with('\\')
            || name.chars().nth(1) == Some(':')
        {
            return Err(ArchiveError::PathTraversal(name));
        }

        if name == "plugin.toml"
            || (name.ends_with("/plugin.toml") && name.matches('/').count() == 1)
        {
            has_manifest = true;
        }
    }

    if !has_manifest {
        return Err(ArchiveError::MissingManifest);
    }

    // Create a uniquely-named temp extraction dir to avoid races on concurrent installs
    std::fs::create_dir_all(dest_dir)?;

    let temp_suffix: u64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
        ^ std::process::id() as u64;
    let temp_dir = dest_dir.join(format!(".unpack_{temp_suffix:x}"));
    if temp_dir.exists() {
        std::fs::remove_dir_all(&temp_dir)?;
    }
    std::fs::create_dir_all(&temp_dir)?;

    // Run extraction in a helper so we can clean up temp_dir on ANY error
    let result = extract_and_finalize(&temp_dir, &mut archive, dest_dir, sha256);
    if result.is_err() {
        let _ = std::fs::remove_dir_all(&temp_dir);
    }
    result
}

/// Inner helper for `unpack_bytes` — extracted so the caller can guarantee
/// temp dir cleanup on any error path (IO, manifest parse, rename, etc.).
fn extract_and_finalize(
    temp_dir: &Path,
    archive: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>,
    dest_dir: &Path,
    sha256: String,
) -> Result<UnpackResult, ArchiveError> {
    let canonical_temp = temp_dir.canonicalize()?;
    let mut file_count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let name = entry.name().to_string();

        let out_path = temp_dir.join(&name);

        // Defense-in-depth: verify joined path stays inside temp_dir
        if let Ok(canonical) = out_path.canonicalize().or_else(|_| {
            // Path doesn't exist yet; canonicalize parent and append filename
            out_path
                .parent()
                .and_then(|p| p.canonicalize().ok())
                .map(|p| p.join(out_path.file_name().unwrap_or_default()))
                .ok_or_else(|| std::io::Error::other("no parent"))
        }) && !canonical.starts_with(&canonical_temp)
        {
            return Err(ArchiveError::PathTraversal(name));
        }

        if entry.is_dir() {
            std::fs::create_dir_all(&out_path)?;
        } else {
            if let Some(parent) = out_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out_file = std::fs::File::create(&out_path)?;
            std::io::copy(&mut entry, &mut out_file)?;
            file_count += 1;
        }
    }

    // Parse manifest from extracted files
    let manifest_path = temp_dir.join("plugin.toml");
    let manifest = PluginManifest::from_file(&manifest_path)
        .map_err(|e| ArchiveError::Manifest(e.to_string()))?;

    // Move to final destination: dest_dir/<plugin-name>/
    let final_dir = dest_dir.join(&manifest.name);
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir)?;
    }
    std::fs::rename(temp_dir, &final_dir)?;

    Ok(UnpackResult {
        dest_dir: final_dir,
        manifest,
        sha256,
        file_count,
    })
}

/// Verify an archive's SHA-256 matches an expected value.
pub fn verify_checksum(archive_path: &Path, expected_sha256: &str) -> Result<bool, ArchiveError> {
    let actual = file_sha256(archive_path)?;
    if actual != expected_sha256 {
        return Err(ArchiveError::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }
    Ok(true)
}

/// Verify in-memory bytes against an expected checksum.
pub fn verify_bytes_checksum(data: &[u8], expected_sha256: &str) -> Result<bool, ArchiveError> {
    let actual = sha256_hex(data);
    if actual != expected_sha256 {
        return Err(ArchiveError::ChecksumMismatch {
            expected: expected_sha256.to_string(),
            actual,
        });
    }
    Ok(true)
}

// ── Helpers ──────────────────────────────────────────────────

/// Directories excluded from archive packing (not meaningful to plugin runtime).
const EXCLUDED_DIRS: &[&str] = &[
    ".git",
    ".svn",
    ".hg",
    "node_modules",
    "target",
    "__pycache__",
];

/// Individual files excluded from archive packing.
const EXCLUDED_FILES: &[&str] = &[".DS_Store", "Thumbs.db", ".env"];

fn collect_files(
    base: &Path,
    dir: &Path,
    out: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), ArchiveError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let name_str = entry.file_name().to_string_lossy().to_string();

        if path.is_dir() {
            if EXCLUDED_DIRS.contains(&name_str.as_str()) {
                continue;
            }
            collect_files(base, &path, out)?;
        } else {
            if EXCLUDED_FILES.contains(&name_str.as_str()) {
                continue;
            }
            let rel = path.strip_prefix(base).unwrap_or(&path).to_path_buf();
            out.push((path, rel));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_plugin(dir: &Path) {
        std::fs::write(
            dir.join("plugin.toml"),
            r#"
name = "test-archive"
version = "1.2.3"
description = "Archive test plugin"

[[tools]]
name = "greet"
description = "Say hello"
"#,
        )
        .unwrap();
        std::fs::write(dir.join("greet.sh"), "#!/bin/sh\necho hello").unwrap();

        // Add a subdirectory with a file
        let sub = dir.join("skills");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("guide.md"), "# Guide\nSome guidance.").unwrap();
    }

    #[test]
    fn pack_creates_archive_with_correct_name() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        make_test_plugin(src.path());

        let result = pack(src.path(), out.path()).unwrap();
        assert_eq!(result.name, "test-archive");
        assert_eq!(result.version, "1.2.3");
        assert_eq!(result.file_count, 3); // plugin.toml, greet.sh, skills/guide.md
        assert!(result.archive_path.exists());
        assert!(
            result
                .archive_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .contains("test-archive-1.2.3.ic.zip")
        );
        assert!(!result.sha256.is_empty());
    }

    #[test]
    fn pack_fails_without_manifest() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        // No plugin.toml
        std::fs::write(src.path().join("hello.sh"), "echo hi").unwrap();

        let err = pack(src.path(), out.path()).unwrap_err();
        assert!(matches!(err, ArchiveError::MissingManifest));
    }

    #[test]
    fn roundtrip_pack_unpack() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        make_test_plugin(src.path());

        let packed = pack(src.path(), out.path()).unwrap();
        let unpacked = unpack(&packed.archive_path, staging.path()).unwrap();

        assert_eq!(unpacked.manifest.name, "test-archive");
        assert_eq!(unpacked.manifest.version, "1.2.3");
        assert_eq!(unpacked.file_count, 3);
        assert_eq!(unpacked.sha256, packed.sha256);

        // Verify extracted files exist
        let plugin_dir = staging.path().join("test-archive");
        assert!(plugin_dir.join("plugin.toml").exists());
        assert!(plugin_dir.join("greet.sh").exists());
        assert!(plugin_dir.join("skills").join("guide.md").exists());
    }

    #[test]
    fn checksum_verification_passes() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        make_test_plugin(src.path());

        let packed = pack(src.path(), out.path()).unwrap();
        assert!(verify_checksum(&packed.archive_path, &packed.sha256).unwrap());
    }

    #[test]
    fn checksum_verification_fails_on_mismatch() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        make_test_plugin(src.path());

        let packed = pack(src.path(), out.path()).unwrap();
        let err = verify_checksum(&packed.archive_path, "deadbeef").unwrap_err();
        assert!(matches!(err, ArchiveError::ChecksumMismatch { .. }));
    }

    #[test]
    fn unpack_bytes_works() {
        let src = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let staging = tempfile::tempdir().unwrap();
        make_test_plugin(src.path());

        let packed = pack(src.path(), out.path()).unwrap();
        let bytes = std::fs::read(&packed.archive_path).unwrap();
        let sha = sha256_hex(&bytes);

        let unpacked = unpack_bytes(&bytes, staging.path(), sha).unwrap();
        assert_eq!(unpacked.manifest.name, "test-archive");
        assert!(
            staging
                .path()
                .join("test-archive")
                .join("plugin.toml")
                .exists()
        );
    }

    #[test]
    fn sha256_hex_is_deterministic() {
        let a = sha256_hex(b"hello world");
        let b = sha256_hex(b"hello world");
        assert_eq!(a, b);
        assert_eq!(a.len(), 64); // 32 bytes = 64 hex chars
    }
}
