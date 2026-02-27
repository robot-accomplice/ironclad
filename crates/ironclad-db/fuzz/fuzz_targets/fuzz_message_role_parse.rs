#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // MessageRole::from_str_lossy maps arbitrary strings to an enum.
    let _role = ironclad_db::sessions::MessageRole::from_str_lossy(data);
});
