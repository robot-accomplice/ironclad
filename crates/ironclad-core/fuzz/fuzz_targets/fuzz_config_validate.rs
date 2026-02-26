#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Parse, then validate: exercises both the TOML deserialization and the
    // semantic validation layer. Only configs that parse successfully reach
    // validate(), but partial/malformed TOML that happens to deserialize is
    // the interesting case.
    if let Ok(config) = ironclad_core::IroncladConfig::from_str(data) {
        let _ = config.validate();
    }
});
