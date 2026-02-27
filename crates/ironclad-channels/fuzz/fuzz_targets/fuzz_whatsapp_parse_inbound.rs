#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Parse raw bytes as JSON, then feed the Value into the WhatsApp
    // webhook parser. Exercises deeply nested array/object traversal.
    if let Ok(text) = std::str::from_utf8(data) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(text) {
            let _ = ironclad_channels::whatsapp::WhatsAppAdapter::parse_inbound(&value);
        }
    }
});
