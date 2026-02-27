#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Parse, then validate: exercises both the TOML deserialization and
    // the semantic validation (name format, version format, tool names).
    if let Ok(manifest) = ironclad_plugin_sdk::manifest::PluginManifest::from_str(data) {
        let _ = manifest.validate();
    }
});
