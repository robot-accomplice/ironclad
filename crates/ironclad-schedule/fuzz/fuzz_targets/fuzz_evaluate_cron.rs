#![no_main]
use libfuzzer_sys::fuzz_target;

use ironclad_schedule::DurableScheduler;

fuzz_target!(|data: &str| {
    // Feed the entire fuzz input as a cron expression with a fixed "now"
    // timestamp. This exercises the cron::Schedule parser, timezone
    // extraction, and the due-window logic.
    let now = "2025-06-15T12:00:00+00:00";
    let _ = DurableScheduler::evaluate_cron(data, None, now);
    let _ = DurableScheduler::evaluate_cron(data, Some("2025-06-15T11:59:00+00:00"), now);
});
