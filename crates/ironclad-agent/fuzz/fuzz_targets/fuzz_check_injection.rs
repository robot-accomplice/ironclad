#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    let _ = ironclad_agent::injection::check_injection(data);
});
