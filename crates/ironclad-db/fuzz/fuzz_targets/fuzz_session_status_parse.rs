#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // SessionStatus::from_str_lossy maps arbitrary strings to an enum.
    // It should never panic, but the match logic is worth exercising with
    // random input.
    let _status = ironclad_db::sessions::SessionStatus::from_str_lossy(data);
});
