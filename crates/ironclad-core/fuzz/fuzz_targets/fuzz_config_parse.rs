#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // Exercise the TOML config parser with arbitrary string input.
    // IroncladConfig::from_str returns Result, so errors are expected
    // and should never panic.
    let _ = ironclad_core::IroncladConfig::from_str(data);
});
