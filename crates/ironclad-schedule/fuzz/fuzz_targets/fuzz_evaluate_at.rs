#![no_main]
use libfuzzer_sys::fuzz_target;

use ironclad_schedule::DurableScheduler;

fuzz_target!(|data: &str| {
    // Feed the fuzz input as both a schedule_expr and a now-timestamp
    // to exercise the ISO/RFC-3339 date parsing paths in evaluate_at.
    let now = "2025-06-15T12:00:00+00:00";
    let _ = DurableScheduler::evaluate_at(data, now);
    let _ = DurableScheduler::evaluate_at(now, data);
});
