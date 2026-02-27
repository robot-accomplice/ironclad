#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Exercise the TOML manifest parser with arbitrary string input.
    // PluginManifest::from_str returns Result, so parse failures are
    // expected and should never panic.
    let _ = ironclad_plugin_sdk::manifest::PluginManifest::from_str(data);
});
