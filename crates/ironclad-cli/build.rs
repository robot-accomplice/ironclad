//! Build script — embeds workspace-level assets that `include_str!` cannot
//! reach inside a `cargo publish` tarball.

use std::env;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    let out_dir = env::var("OUT_DIR").unwrap();
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").unwrap();

    // Primary: workspace registry (normal dev/CI builds).
    let workspace_path = Path::new(&manifest_dir).join("../../registry/builtin-skills.json");

    // Fallback: bundled copy inside the crate (cargo publish verification).
    let bundled_path = Path::new(&manifest_dir).join("data/builtin-skills.json");

    let source: PathBuf = if workspace_path.exists() {
        workspace_path
    } else if bundled_path.exists() {
        bundled_path
    } else {
        panic!(
            "builtin-skills.json not found at workspace ({}) or bundled ({}) path",
            workspace_path.display(),
            bundled_path.display()
        );
    };

    let dest = Path::new(&out_dir).join("builtin-skills.json");
    fs::copy(&source, &dest).unwrap_or_else(|e| {
        panic!("Failed to copy {}: {e}", source.display());
    });

    println!("cargo:rerun-if-changed={}", source.display());
}
