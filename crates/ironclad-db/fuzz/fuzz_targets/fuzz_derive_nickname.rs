#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &str| {
    // derive_nickname performs greeting-prefix stripping, sentence boundary
    // detection, Unicode-aware truncation, and title-casing. All of these
    // are interesting targets for edge-case strings (mixed scripts, emoji
    // clusters, extremely long lines, embedded NUL bytes, etc.).
    let _nickname = ironclad_db::sessions::derive_nickname(data);
});
