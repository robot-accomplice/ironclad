#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // sanitize_platform strips control characters and truncates.
    // Should never panic regardless of input.
    let _ = ironclad_channels::sanitize_platform(data);
});
