#![no_main]
use libfuzzer_sys::fuzz_target;

use ironclad_schedule::DurableScheduler;

fuzz_target!(|data: &str| {
    // Exercise calculate_next_run with each schedule_kind and the fuzz
    // input as both schedule_expr and now parameters.
    let now = "2025-06-15T12:00:00+00:00";
    for kind in &["interval", "at", "cron", "bogus"] {
        let _ = DurableScheduler::calculate_next_run(kind, Some(data), Some(60_000), now);
        let _ = DurableScheduler::calculate_next_run(kind, Some(data), None, now);
        let _ = DurableScheduler::calculate_next_run(kind, None, Some(60_000), data);
    }
});
