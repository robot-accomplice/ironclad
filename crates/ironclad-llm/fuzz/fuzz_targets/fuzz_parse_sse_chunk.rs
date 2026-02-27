#![no_main]
use libfuzzer_sys::fuzz_target;

use ironclad_core::ApiFormat;

fuzz_target!(|data: &str| {
    // Exercise the SSE line parser against every supported API format.
    // parse_sse_chunk returns Option, so None on bad input is expected.
    for format in &[
        ApiFormat::OpenAiCompletions,
        ApiFormat::OpenAiResponses,
        ApiFormat::AnthropicMessages,
        ApiFormat::GoogleGenerativeAi,
    ] {
        let _ = ironclad_llm::format::parse_sse_chunk(data, format);
    }
});
