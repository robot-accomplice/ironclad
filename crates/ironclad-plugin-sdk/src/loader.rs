use std::path::{Path, PathBuf};

use tracing::{debug, warn};

use ironclad_core::Result;

use crate::manifest::PluginManifest;

#[derive(Debug, Clone)]
pub struct DiscoveredPlugin {
    pub manifest: PluginManifest,
    pub dir: PathBuf,
}

pub fn discover_plugins(plugins_dir: &Path) -> Result<Vec<DiscoveredPlugin>> {
    if !plugins_dir.exists() {
        debug!(dir = %plugins_dir.display(), "plugins directory does not exist");
        return Ok(Vec::new());
    }

    let mut discovered = Vec::new();

    let entries = std::fs::read_dir(plugins_dir)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if !path.is_dir() {
            continue;
        }

        let manifest_path = path.join("plugin.toml");
        if !manifest_path.exists() {
            debug!(dir = %path.display(), "skipping directory without plugin.toml");
            continue;
        }

        match PluginManifest::from_file(&manifest_path) {
            Ok(manifest) => {
                debug!(name = %manifest.name, version = %manifest.version, "discovered plugin");
                discovered.push(DiscoveredPlugin {
                    manifest,
                    dir: path,
                });
            }
            Err(e) => {
                warn!(path = %manifest_path.display(), error = %e, "failed to parse plugin manifest");
            }
        }
    }

    discovered.sort_by(|a, b| a.manifest.name.cmp(&b.manifest.name));
    Ok(discovered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discover_missing_dir() {
        let result = discover_plugins(Path::new("/nonexistent/plugins"));
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }

    #[test]
    fn discover_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let result = discover_plugins(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn discover_valid_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("my-plugin");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
name = "my-plugin"
version = "1.0.0"
description = "Test plugin"
"#,
        )
        .unwrap();

        let result = discover_plugins(dir.path()).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].manifest.name, "my-plugin");
        assert_eq!(result[0].dir, plugin_dir);
    }

    #[test]
    fn discover_skips_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("not-a-dir.txt"), "hello").unwrap();
        let result = discover_plugins(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn discover_skips_dir_without_manifest() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("no-manifest")).unwrap();
        let result = discover_plugins(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn discover_skips_invalid_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("bad");
        std::fs::create_dir(&plugin_dir).unwrap();
        std::fs::write(plugin_dir.join("plugin.toml"), "[[[[invalid").unwrap();
        let result = discover_plugins(dir.path()).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn discover_sorted_by_name() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["charlie", "alpha", "bravo"] {
            let p = dir.path().join(name);
            std::fs::create_dir(&p).unwrap();
            std::fs::write(
                p.join("plugin.toml"),
                format!("name = \"{name}\"\nversion = \"1.0.0\"\n"),
            )
            .unwrap();
        }
        let result = discover_plugins(dir.path()).unwrap();
        let names: Vec<_> = result.iter().map(|p| p.manifest.name.as_str()).collect();
        assert_eq!(names, vec!["alpha", "bravo", "charlie"]);
    }
}
