#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Parse raw bytes as JSON, then feed the Value into the Telegram
    // inbound parser. Exercises nested field extraction and type coercion.
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            let _ = ironclad_channels::telegram::TelegramAdapter::parse_inbound(&value);
        }
    }
});
