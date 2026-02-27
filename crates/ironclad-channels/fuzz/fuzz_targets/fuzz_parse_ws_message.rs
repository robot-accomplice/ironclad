#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Parse arbitrary bytes as a UTF-8 string, then feed into the WebSocket
    // message parser. This exercises both JSON parsing and the field
    // extraction logic in parse_ws_message.
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = ironclad_channels::web::WebSocketChannel::parse_ws_message(text);
    }
});
