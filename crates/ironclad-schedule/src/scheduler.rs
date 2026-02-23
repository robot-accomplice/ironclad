use std::str::FromStr;

use chrono::{DateTime, Duration, NaiveDateTime};

/// Pure-function scheduler for cron, interval, and at-style schedule evaluation.
/// No DB dependency — all state is passed in as arguments.
pub struct DurableScheduler;

impl DurableScheduler {
    /// Evaluates whether a cron expression matches the current time.
    /// Uses standard 5-field cron syntax with full support for ranges, lists, and steps.
    pub fn evaluate_cron(cron_expr: &str, _last_run: Option<&str>, now: &str) -> bool {
        let now_dt = match parse_iso(now) {
            Some(dt) => dt,
            None => return false,
        };

        // The `cron` crate uses 7-field syntax: sec min hour dom month dow year
        // Convert 5-field user syntax to 7-field by prepending "0" (seconds) and appending "*" (year)
        let full_expr = format!("0 {cron_expr} *");
        let schedule = match cron::Schedule::from_str(&full_expr) {
            Ok(s) => s,
            Err(_) => return false,
        };

        let now_utc = now_dt.and_utc();
        // Check if any occurrence falls within a 60-second window around `now`
        let window_start = now_utc - Duration::seconds(30);
        schedule
            .after(&window_start)
            .take(1)
            .any(|t| (t - now_utc).num_seconds().abs() < 60)
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
                let full_expr = format!("0 {expr} *");
                let schedule = cron::Schedule::from_str(&full_expr).ok()?;
                let now_utc = now_dt.and_utc();
                schedule.after(&now_utc).next().map(|t| t.to_rfc3339())
            }
            _ => None,
        }
    }
}

fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.naive_utc())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
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
    fn interval_exact_boundary_is_due() {
        assert!(DurableScheduler::evaluate_interval(
            Some("2025-01-01T00:00:00+00:00"),
            60_000,
            "2025-01-01T00:01:00+00:00"
        ));
    }
}
