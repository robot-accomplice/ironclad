use std::str::FromStr;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, Utc};

/// Pure-function scheduler for cron, interval, and at-style schedule evaluation.
/// No DB dependency — all state is passed in as arguments.
pub struct DurableScheduler;

impl DurableScheduler {
    /// Evaluates whether a cron expression matches the current time.
    /// Uses standard 5-field cron syntax with full support for ranges, lists, and steps.
    pub fn evaluate_cron(cron_expr: &str, last_run: Option<&str>, now: &str) -> bool {
        let now_dt = match parse_rfc3339(now) {
            Some(dt) => dt,
            None => return false,
        };

        let (tz, expr) = split_schedule_timezone(cron_expr);
        let full_expr = format!("0 {expr} *");
        let schedule = match cron::Schedule::from_str(&full_expr) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let now_in_tz = now_dt.with_timezone(&tz);
        let probe_start = now_in_tz - Duration::seconds(61);
        let Some(slot) = schedule.after(&probe_start).next() else {
            return false;
        };
        let delta = now_in_tz.signed_duration_since(slot).num_seconds();
        if !(0..60).contains(&delta) {
            return false;
        }

        if let Some(last) = last_run.and_then(parse_rfc3339) {
            let last_in_tz = last.with_timezone(&tz);
            if last_in_tz >= slot {
                return false;
            }
        }
        true
    }

    /// Returns true if enough time has elapsed since `last_run` (or if there was no previous run).
    pub fn evaluate_interval(last_run: Option<&str>, interval_ms: i64, now: &str) -> bool {
        let now_dt = match parse_iso(now) {
            Some(dt) => dt,
            None => return false,
        };

        match last_run.and_then(parse_iso) {
            Some(last) => {
                let elapsed = now_dt.signed_duration_since(last).num_milliseconds();
                elapsed >= interval_ms
            }
            None => true,
        }
    }

    /// Returns true if `now` is at or past the `schedule_expr` ISO timestamp.
    pub fn evaluate_at(schedule_expr: &str, now: &str) -> bool {
        let target = match parse_iso(schedule_expr) {
            Some(dt) => dt,
            None => return false,
        };
        let now_dt = match parse_iso(now) {
            Some(dt) => dt,
            None => return false,
        };
        now_dt >= target
    }

    /// Calculate the next run time based on schedule kind.
    /// - "interval": now + interval_ms
    /// - "at": the schedule_expr itself (one-shot)
    /// - "cron": now + 60s as a rough next-minute approximation
    pub fn calculate_next_run(
        schedule_kind: &str,
        schedule_expr: Option<&str>,
        schedule_every_ms: Option<i64>,
        now: &str,
    ) -> Option<String> {
        let now_dt = parse_iso(now)?;

        match schedule_kind {
            "interval" => {
                let ms = schedule_every_ms?;
                let next = now_dt + Duration::milliseconds(ms);
                Some(next.and_utc().to_rfc3339())
            }
            "at" => {
                let expr = schedule_expr?;
                let target = parse_iso(expr)?;
                if now_dt >= target {
                    None
                } else {
                    Some(target.and_utc().to_rfc3339())
                }
            }
            "cron" => {
                let expr = schedule_expr?;
                let (tz, expr) = split_schedule_timezone(expr);
                let full_expr = format!("0 {expr} *");
                let schedule = cron::Schedule::from_str(&full_expr).ok()?;
                let now_utc = now_dt.and_utc();
                let now_in_tz = now_utc.with_timezone(&tz);
                schedule
                    .after(&now_in_tz)
                    .next()
                    .map(|t| t.with_timezone(&Utc).to_rfc3339())
            }
            _ => None,
        }
    }
}

fn parse_rfc3339(s: &str) -> Option<DateTime<FixedOffset>> {
    DateTime::parse_from_rfc3339(s).ok()
}

fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.naive_utc())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
}

fn split_schedule_timezone(schedule: &str) -> (FixedOffset, &str) {
    let schedule = schedule.trim();
    for prefix in ["CRON_TZ=", "TZ="] {
        if let Some(rest) = schedule.strip_prefix(prefix)
            && let Some((tz_raw, expr)) = rest.split_once(' ')
        {
            let tz = parse_timezone(tz_raw).unwrap_or_else(zero_offset);
            return (tz, expr.trim());
        }
    }
    (zero_offset(), schedule)
}

fn parse_timezone(raw: &str) -> Option<FixedOffset> {
    let tz = raw.trim();
    if tz.eq_ignore_ascii_case("UTC") || tz.eq_ignore_ascii_case("Z") {
        return Some(zero_offset());
    }

    let cleaned = tz
        .strip_prefix("UTC")
        .or_else(|| tz.strip_prefix("utc"))
        .unwrap_or(tz);
    parse_offset(cleaned)
}

fn parse_offset(raw: &str) -> Option<FixedOffset> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Some(zero_offset());
    }
    let sign = if raw.starts_with('-') { -1 } else { 1 };
    let trimmed = raw.trim_start_matches(['+', '-']);

    let (hours, minutes) = if let Some((h, m)) = trimmed.split_once(':') {
        (h.parse::<i32>().ok()?, m.parse::<i32>().ok()?)
    } else if trimmed.len() == 4 {
        (
            trimmed[..2].parse::<i32>().ok()?,
            trimmed[2..].parse::<i32>().ok()?,
        )
    } else {
        (trimmed.parse::<i32>().ok()?, 0)
    };

    let secs = sign * (hours * 3600 + minutes * 60);
    FixedOffset::east_opt(secs)
}

fn zero_offset() -> FixedOffset {
    FixedOffset::east_opt(0).expect("zero offset is valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interval_due() {
        let last = "2025-01-01T00:00:00+00:00";
        let now = "2025-01-01T00:01:00+00:00";
        assert!(DurableScheduler::evaluate_interval(Some(last), 60_000, now));
    }

    #[test]
    fn interval_not_due() {
        let last = "2025-01-01T00:00:00+00:00";
        let now = "2025-01-01T00:00:30+00:00";
        assert!(!DurableScheduler::evaluate_interval(
            Some(last),
            60_000,
            now
        ));
    }

    #[test]
    fn interval_no_last_run() {
        assert!(DurableScheduler::evaluate_interval(
            None,
            60_000,
            "2025-01-01T00:00:00+00:00"
        ));
    }

    #[test]
    fn at_schedule_past() {
        assert!(DurableScheduler::evaluate_at(
            "2025-01-01T00:00:00+00:00",
            "2025-01-01T01:00:00+00:00"
        ));
    }

    #[test]
    fn at_schedule_future() {
        assert!(!DurableScheduler::evaluate_at(
            "2025-01-01T02:00:00+00:00",
            "2025-01-01T01:00:00+00:00"
        ));
    }

    #[test]
    fn next_run_interval() {
        let result = DurableScheduler::calculate_next_run(
            "interval",
            None,
            Some(60_000),
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_some());
        let next = result.unwrap();
        assert!(next.contains("00:01:00"));
    }

    #[test]
    fn next_run_at_already_passed() {
        let result = DurableScheduler::calculate_next_run(
            "at",
            Some("2025-01-01T00:00:00+00:00"),
            None,
            "2025-01-01T01:00:00+00:00",
        );
        assert!(result.is_none());
    }

    #[test]
    fn cron_matches() {
        // 2025-01-01 is a Wednesday (day_of_week=3)
        assert!(DurableScheduler::evaluate_cron(
            "0 12 * * *",
            None,
            "2025-01-01T12:00:00+00:00"
        ));
    }

    #[test]
    fn cron_no_match() {
        assert!(!DurableScheduler::evaluate_cron(
            "30 12 * * *",
            None,
            "2025-01-01T12:00:00+00:00"
        ));
    }

    #[test]
    fn next_run_at_future() {
        let result = DurableScheduler::calculate_next_run(
            "at",
            Some("2025-01-01T02:00:00+00:00"),
            None,
            "2025-01-01T01:00:00+00:00",
        );
        assert!(result.is_some());
    }

    #[test]
    fn next_run_cron() {
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("0 12 * * *"),
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_some());
    }

    #[test]
    fn next_run_unknown_kind() {
        let result =
            DurableScheduler::calculate_next_run("weekly", None, None, "2025-01-01T00:00:00+00:00");
        assert!(result.is_none());
    }

    #[test]
    fn next_run_interval_missing_ms() {
        let result = DurableScheduler::calculate_next_run(
            "interval",
            None,
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none());
    }

    #[test]
    fn cron_wrong_field_count() {
        assert!(!DurableScheduler::evaluate_cron(
            "0 12 *",
            None,
            "2025-01-01T12:00:00+00:00"
        ));
    }

    #[test]
    fn interval_invalid_now() {
        assert!(!DurableScheduler::evaluate_interval(
            None,
            60_000,
            "not-a-date"
        ));
    }

    #[test]
    fn at_invalid_target() {
        assert!(!DurableScheduler::evaluate_at(
            "bad",
            "2025-01-01T00:00:00+00:00"
        ));
    }

    #[test]
    fn at_invalid_now() {
        assert!(!DurableScheduler::evaluate_at(
            "2025-01-01T00:00:00+00:00",
            "bad"
        ));
    }

    #[test]
    fn cron_with_last_run_still_matches() {
        assert!(DurableScheduler::evaluate_cron(
            "0 12 * * *",
            Some("2024-12-31T12:00:00+00:00"),
            "2025-01-01T12:00:00+00:00"
        ));
    }

    #[test]
    fn cron_does_not_retrigger_same_slot() {
        assert!(!DurableScheduler::evaluate_cron(
            "0 12 * * *",
            Some("2025-01-01T12:00:30+00:00"),
            "2025-01-01T12:00:45+00:00"
        ));
    }

    #[test]
    fn cron_supports_timezone_prefix() {
        assert!(DurableScheduler::evaluate_cron(
            "CRON_TZ=UTC+02:00 0 9 * * *",
            None,
            "2025-01-01T07:00:10+00:00"
        ));
        assert!(!DurableScheduler::evaluate_cron(
            "CRON_TZ=UTC+02:00 0 9 * * *",
            None,
            "2025-01-01T08:00:10+00:00"
        ));
    }

    #[test]
    fn next_run_cron_with_timezone_is_utc_normalized() {
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("CRON_TZ=UTC+02:00 0 9 * * *"),
            None,
            "2025-01-01T06:00:00+00:00",
        )
        .expect("next run");
        assert!(
            result.starts_with("2025-01-01T07:00:00"),
            "expected UTC-normalized 07:00 run, got {result}"
        );
    }

    #[test]
    fn interval_exact_boundary_is_due() {
        assert!(DurableScheduler::evaluate_interval(
            Some("2025-01-01T00:00:00+00:00"),
            60_000,
            "2025-01-01T00:01:00+00:00"
        ));
    }

    // ── property-based tests (v0.8.0 stabilization) ────────────────────

    use proptest::prelude::*;

    proptest! {
        #[test]
        fn proptest_calculate_next_run_is_after_now(interval_ms in 1000i64..86_400_000) {
            let now = "2025-06-15T12:00:00+00:00";
            if let Some(next) = DurableScheduler::calculate_next_run(
                "interval",
                None,
                Some(interval_ms),
                now,
            ) {
                prop_assert!(next > now.to_string(), "next_run must be after now");
            }
        }

        #[test]
        fn proptest_evaluate_interval_elapsed_returns_true(interval_ms in 1000i64..3_600_000) {
            // last_run = 36 hours ago, so any interval <= 36h should be elapsed
            let now = "2025-06-15T12:00:00+00:00";
            let last_run = "2025-06-14T00:00:00+00:00";
            let result = DurableScheduler::evaluate_interval(Some(last_run), interval_ms, now);
            prop_assert!(result, "interval {}ms should have elapsed since 36h ago", interval_ms);
        }

        #[test]
        fn proptest_evaluate_interval_not_elapsed_returns_false(interval_ms in 120_000i64..86_400_000) {
            // last_run = 1 second ago, so any interval >= 2 minutes should NOT be elapsed
            let now = "2025-06-15T12:00:01+00:00";
            let last_run = "2025-06-15T12:00:00+00:00";
            let result = DurableScheduler::evaluate_interval(Some(last_run), interval_ms, now);
            prop_assert!(!result, "interval {}ms should not have elapsed after 1 second", interval_ms);
        }

        #[test]
        fn proptest_calculate_next_run_is_valid_rfc3339(interval_ms in 1000i64..86_400_000) {
            let now = "2025-06-15T12:00:00+00:00";
            if let Some(next) = DurableScheduler::calculate_next_run(
                "interval",
                None,
                Some(interval_ms),
                now,
            ) {
                let parsed = chrono::DateTime::parse_from_rfc3339(&next);
                prop_assert!(parsed.is_ok(), "next_run '{}' must be valid RFC3339", next);
            }
        }

        #[test]
        fn proptest_evaluate_cron_no_last_run_matches_now(minute in 0u32..60) {
            // Build a cron expression that matches "now" at any given minute
            let now = format!("2025-06-15T12:{:02}:00+00:00", minute);
            let cron_expr = format!("{} 12 * * *", minute);
            let result = DurableScheduler::evaluate_cron(&cron_expr, None, &now);
            prop_assert!(result, "cron '{}' should match now '{}'", cron_expr, now);
        }
    }
}
