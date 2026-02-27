use std::sync::Arc;

use tracing::{debug, info, warn};

use ironclad_core::config::PluginsConfig;
use ironclad_plugin_sdk::loader::discover_plugins;
use ironclad_plugin_sdk::registry::{PermissionPolicy, PluginRegistry};
use ironclad_plugin_sdk::script::ScriptPlugin;

/// Discover plugin manifests, instantiate `ScriptPlugin`s, and register them.
pub async fn init_plugin_registry(config: &PluginsConfig) -> Arc<PluginRegistry> {
    let registry = Arc::new(PluginRegistry::new(
        config.allow.clone(),
        config.deny.clone(),
        PermissionPolicy {
            strict: config.strict_permissions,
            allowed: config.allowed_permissions.clone(),
        },
    ));

    let plugins_dir = &config.dir;
    if !plugins_dir.exists() {
        debug!(dir = %plugins_dir.display(), "plugins directory does not exist, skipping discovery");
        return registry;
    }

    let discovered = match discover_plugins(plugins_dir) {
        Ok(d) => d,
        Err(e) => {
            warn!(error = %e, "failed to discover plugins");
            return registry;
        }
    };

    info!(count = discovered.len(), "discovered plugins");

    for dp in discovered {
        let name = dp.manifest.name.clone();
        let version = dp.manifest.version.clone();
        let tool_count = dp.manifest.tools.len();

        let plugin = ScriptPlugin::new(dp.manifest, dp.dir);
        match registry.register(Box::new(plugin)).await {
            Ok(()) => {
                info!(
                    name = %name,
                    version = %version,
                    tools = tool_count,
                    "registered script plugin"
                );
            }
            Err(e) => {
                warn!(name = %name, error = %e, "failed to register plugin");
            }
        }
    }

    let init_errors = registry.init_all().await;
    if !init_errors.is_empty() {
        for err in &init_errors {
            warn!(error = %err, "plugin init error");
        }
    }

    let count = registry.plugin_count().await;
    info!(active = count, "plugin registry ready");

    registry
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    #[tokio::test]
    async fn init_with_nonexistent_dir() {
        let config = PluginsConfig {
            dir: PathBuf::from("/nonexistent/plugins"),
            allow: vec![],
            deny: vec![],
            strict_permissions: false,
            allowed_permissions: vec![],
        };
        let registry = init_plugin_registry(&config).await;
        assert_eq!(registry.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn init_with_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let config = PluginsConfig {
            dir: dir.path().to_path_buf(),
            allow: vec![],
            deny: vec![],
            strict_permissions: false,
            allowed_permissions: vec![],
        };
        let registry = init_plugin_registry(&config).await;
        assert_eq!(registry.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn init_discovers_and_registers_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("hello-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
name = "hello-plugin"
version = "0.1.0"
description = "A test plugin"

[[tools]]
name = "say_hello"
description = "Says hello"
"#,
        )
        .unwrap();
        fs::write(plugin_dir.join("say_hello.gosh"), "echo hello").unwrap();

        let config = PluginsConfig {
            dir: dir.path().to_path_buf(),
            allow: vec![],
            deny: vec![],
            strict_permissions: false,
            allowed_permissions: vec![],
        };
        let registry = init_plugin_registry(&config).await;
        assert_eq!(registry.plugin_count().await, 1);

        let plugins = registry.list_plugins().await;
        assert_eq!(plugins[0].name, "hello-plugin");
        assert_eq!(plugins[0].tools.len(), 1);
    }

    #[tokio::test]
    async fn deny_list_blocks_plugin() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("blocked");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            "name = \"blocked\"\nversion = \"1.0.0\"\n",
        )
        .unwrap();

        let config = PluginsConfig {
            dir: dir.path().to_path_buf(),
            allow: vec![],
            deny: vec!["blocked".into()],
            strict_permissions: false,
            allowed_permissions: vec![],
        };
        let registry = init_plugin_registry(&config).await;
        assert_eq!(registry.plugin_count().await, 0);
    }

    #[tokio::test]
    async fn plugin_tool_execution() {
        let dir = tempfile::tempdir().unwrap();
        let plugin_dir = dir.path().join("echo-plugin");
        fs::create_dir(&plugin_dir).unwrap();
        fs::write(
            plugin_dir.join("plugin.toml"),
            r#"
name = "echo-plugin"
version = "0.1.0"
[[tools]]
name = "echo"
description = "Echoes input"
"#,
        )
        .unwrap();
        fs::write(
            plugin_dir.join("echo.sh"),
            "#!/bin/sh\necho $IRONCLAD_INPUT",
        )
        .unwrap();

        let config = PluginsConfig {
            dir: dir.path().to_path_buf(),
            allow: vec![],
            deny: vec![],
            strict_permissions: false,
            allowed_permissions: vec![],
        };
        let registry = init_plugin_registry(&config).await;

        let result = registry
            .execute_tool("echo", &serde_json::json!({"msg": "hi"}))
            .await
            .unwrap();
        assert!(result.success);
        assert!(result.output.contains("msg"));
    }
}
