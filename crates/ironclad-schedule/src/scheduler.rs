use std::str::FromStr;

use chrono::{DateTime, Duration, FixedOffset, NaiveDateTime, Offset, Utc};
use chrono_tz::Tz;

/// Pure-function scheduler for cron, interval, and at-style schedule evaluation.
/// No DB dependency — all state is passed in as arguments.
pub struct DurableScheduler;

impl DurableScheduler {
    /// Returns true if a cron expression is syntactically valid.
    /// Accepts 5-field expressions with optional `TZ=` / `CRON_TZ=` prefixes.
    pub fn is_valid_cron_expression(cron_expr: &str) -> bool {
        let now_utc = Utc::now();
        let (_tz, expr) = split_schedule_timezone_at(cron_expr, now_utc);
        let full_expr = format!("0 {expr} *");
        cron::Schedule::from_str(&full_expr).is_ok()
    }

    /// Evaluates whether a cron expression matches the current time.
    /// Uses standard 5-field cron syntax with full support for ranges, lists, and steps.
    pub fn evaluate_cron(cron_expr: &str, last_run: Option<&str>, now: &str) -> bool {
        let now_dt = match parse_rfc3339(now) {
            Some(dt) => dt,
            None => return false,
        };

        let now_utc = now_dt.with_timezone(&Utc);
        let (tz, expr) = split_schedule_timezone_at(cron_expr, now_utc);
        let full_expr = format!("0 {expr} *");
        let schedule = match cron::Schedule::from_str(&full_expr) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(cron_expr, error = %e, "invalid cron expression, schedule will never fire");
                return false;
            }
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
    /// Returns false for zero/negative intervals to prevent fire storms.
    pub fn evaluate_interval(last_run: Option<&str>, interval_ms: i64, now: &str) -> bool {
        if interval_ms <= 0 {
            return false;
        }
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
                let now_utc = now_dt.and_utc();
                let (tz, expr) = split_schedule_timezone_at(expr, now_utc);
                let full_expr = format!("0 {expr} *");
                let schedule = cron::Schedule::from_str(&full_expr).ok()?;
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
    DateTime::parse_from_rfc3339(s).ok().or_else(|| {
        // Backward-compat: SQLite's datetime('now') produces "YYYY-MM-DD HH:MM:SS"
        // (space-separated, no timezone). Treat as UTC.
        NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
            .ok()
            .map(|naive| naive.and_utc().fixed_offset())
    })
}

fn parse_iso(s: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(s)
        .map(|dt| dt.naive_utc())
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S").ok())
        // Backward-compat: SQLite's datetime('now') produces "YYYY-MM-DD HH:MM:SS"
        .or_else(|| NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S").ok())
}

#[cfg(test)]
fn split_schedule_timezone(schedule: &str) -> (FixedOffset, &str) {
    split_schedule_timezone_at(schedule, Utc::now())
}

fn split_schedule_timezone_at(schedule: &str, at: DateTime<Utc>) -> (FixedOffset, &str) {
    let schedule = schedule.trim();
    for prefix in ["CRON_TZ=", "TZ="] {
        if let Some(rest) = schedule.strip_prefix(prefix)
            && let Some((tz_raw, expr)) = rest.split_once(' ')
        {
            let tz = parse_timezone_at(tz_raw, at).unwrap_or_else(zero_offset);
            return (tz, expr.trim());
        }
    }
    (zero_offset(), schedule)
}

#[cfg(test)]
fn parse_timezone(raw: &str) -> Option<FixedOffset> {
    parse_timezone_at(raw, Utc::now())
}

/// Resolves a timezone string to a `FixedOffset` at a specific instant.
/// Supports UTC/Z literals, UTC±HH:MM offsets, and IANA timezone names
/// (e.g. "America/New_York"). IANA names are resolved at the given instant
/// to account for DST transitions.
fn parse_timezone_at(raw: &str, at: DateTime<Utc>) -> Option<FixedOffset> {
    let tz = raw.trim();
    if tz.eq_ignore_ascii_case("UTC") || tz.eq_ignore_ascii_case("Z") {
        return Some(zero_offset());
    }

    let cleaned = tz
        .strip_prefix("UTC")
        .or_else(|| tz.strip_prefix("utc"))
        .unwrap_or(tz);

    // Try numeric offset first (e.g. +05:30, -08:00)
    if let Some(offset) = parse_offset(cleaned) {
        return Some(offset);
    }

    // Try IANA timezone name (e.g. America/New_York, Europe/London)
    if let Ok(iana) = tz.parse::<Tz>() {
        return Some(at.with_timezone(&iana).offset().fix());
    }

    None
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
                prop_assert!(next.as_str() > now, "next_run must be after now");
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

    // ── BUG-094: parse_offset exhaustive coverage ──────────────────────

    #[test]
    fn parse_offset_empty_returns_zero() {
        let result = parse_offset("");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), zero_offset());
    }

    #[test]
    fn parse_offset_whitespace_only_returns_zero() {
        let result = parse_offset("   ");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), zero_offset());
    }

    #[test]
    fn parse_offset_positive_hours_only() {
        let result = parse_offset("+5");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::east_opt(5 * 3600).unwrap());
    }

    #[test]
    fn parse_offset_negative_hours_only() {
        let result = parse_offset("-8");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::west_opt(8 * 3600).unwrap());
    }

    #[test]
    fn parse_offset_colon_notation() {
        let result = parse_offset("+05:30");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap()
        );
    }

    #[test]
    fn parse_offset_negative_colon_notation() {
        let result = parse_offset("-03:30");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            FixedOffset::west_opt(3 * 3600 + 30 * 60).unwrap()
        );
    }

    #[test]
    fn parse_offset_four_digit_compact() {
        let result = parse_offset("+0530");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap()
        );
    }

    #[test]
    fn parse_offset_negative_four_digit() {
        let result = parse_offset("-0930");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            FixedOffset::west_opt(9 * 3600 + 30 * 60).unwrap()
        );
    }

    #[test]
    fn parse_offset_no_sign_treated_as_positive() {
        let result = parse_offset("3");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::east_opt(3 * 3600).unwrap());
    }

    #[test]
    fn parse_offset_invalid_returns_none() {
        assert!(parse_offset("abc").is_none());
        assert!(parse_offset("+abc").is_none());
        assert!(parse_offset("+05:xx").is_none());
    }

    // ── BUG-094: parse_timezone exhaustive coverage ────────────────────

    #[test]
    fn parse_timezone_utc_literal() {
        let result = parse_timezone("UTC");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), zero_offset());
    }

    #[test]
    fn parse_timezone_z_literal() {
        let result = parse_timezone("Z");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), zero_offset());
    }

    #[test]
    fn parse_timezone_utc_case_insensitive() {
        assert!(parse_timezone("utc").is_some());
        assert_eq!(parse_timezone("utc").unwrap(), zero_offset());
        assert!(parse_timezone("Utc").is_some());
        assert_eq!(parse_timezone("Utc").unwrap(), zero_offset());
        assert!(parse_timezone("z").is_some());
        assert_eq!(parse_timezone("z").unwrap(), zero_offset());
    }

    #[test]
    fn parse_timezone_utc_with_positive_offset() {
        let result = parse_timezone("UTC+05:30");
        assert!(result.is_some());
        assert_eq!(
            result.unwrap(),
            FixedOffset::east_opt(5 * 3600 + 30 * 60).unwrap()
        );
    }

    #[test]
    fn parse_timezone_utc_with_negative_offset() {
        let result = parse_timezone("UTC-08");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::west_opt(8 * 3600).unwrap());
    }

    #[test]
    fn parse_timezone_lowercase_utc_prefix() {
        let result = parse_timezone("utc+02:00");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::east_opt(2 * 3600).unwrap());
    }

    #[test]
    fn parse_timezone_bare_offset_no_utc_prefix() {
        let result = parse_timezone("+09:00");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), FixedOffset::east_opt(9 * 3600).unwrap());
    }

    #[test]
    fn parse_timezone_whitespace_trimmed() {
        let result = parse_timezone("  UTC  ");
        assert!(result.is_some());
        assert_eq!(result.unwrap(), zero_offset());
    }

    #[test]
    fn parse_timezone_iana_name_resolves() {
        assert!(parse_timezone("America/New_York").is_some());
        assert!(parse_timezone("Europe/London").is_some());
        assert!(parse_timezone("Asia/Tokyo").is_some());
        assert_eq!(
            parse_timezone("Asia/Tokyo").unwrap(),
            FixedOffset::east_opt(9 * 3600).unwrap()
        );
    }

    #[test]
    fn parse_timezone_invalid_returns_none() {
        assert!(parse_timezone("PST").is_none());
        assert!(parse_timezone("foobar").is_none());
        assert!(parse_timezone("Not/A_Zone").is_none());
    }

    // ── split_schedule_timezone coverage ────────────────────────────────

    #[test]
    fn split_schedule_timezone_no_prefix() {
        let (tz, expr) = split_schedule_timezone("0 12 * * *");
        assert_eq!(tz, zero_offset());
        assert_eq!(expr, "0 12 * * *");
    }

    #[test]
    fn split_schedule_timezone_cron_tz_prefix() {
        let (tz, expr) = split_schedule_timezone("CRON_TZ=UTC+05:00 30 9 * * *");
        assert_eq!(tz, FixedOffset::east_opt(5 * 3600).unwrap());
        assert_eq!(expr, "30 9 * * *");
    }

    #[test]
    fn split_schedule_timezone_tz_prefix() {
        let (tz, expr) = split_schedule_timezone("TZ=UTC-03:00 15 18 * * *");
        assert_eq!(tz, FixedOffset::west_opt(3 * 3600).unwrap());
        assert_eq!(expr, "15 18 * * *");
    }

    #[test]
    fn split_schedule_timezone_iana_name_resolves() {
        let (tz, expr) = split_schedule_timezone("CRON_TZ=Asia/Tokyo 0 12 * * *");
        assert_eq!(tz, FixedOffset::east_opt(9 * 3600).unwrap());
        assert_eq!(expr, "0 12 * * *");
    }

    #[test]
    fn split_schedule_timezone_invalid_tz_falls_back_to_zero() {
        let (tz, expr) = split_schedule_timezone("CRON_TZ=NotAZone 0 12 * * *");
        assert_eq!(tz, zero_offset());
        assert_eq!(expr, "0 12 * * *");
    }

    #[test]
    fn split_schedule_timezone_leading_trailing_whitespace() {
        let (tz, expr) = split_schedule_timezone("  0 12 * * *  ");
        assert_eq!(tz, zero_offset());
        assert_eq!(expr, "0 12 * * *");
    }

    // ── parse_iso and parse_rfc3339 coverage via scheduler APIs ────────

    #[test]
    fn evaluate_cron_invalid_now_returns_false() {
        assert!(!DurableScheduler::evaluate_cron(
            "0 12 * * *",
            None,
            "garbage"
        ));
    }

    #[test]
    fn is_valid_cron_expression_detects_invalid_inputs() {
        assert!(DurableScheduler::is_valid_cron_expression("* * * * *"));
        assert!(DurableScheduler::is_valid_cron_expression(
            "TZ=Europe/Zurich 0 * * * *"
        ));
        assert!(!DurableScheduler::is_valid_cron_expression(
            "NOT_VALID_CRON"
        ));
    }

    #[test]
    fn evaluate_interval_iso_without_offset() {
        // parse_iso supports bare ISO timestamps without timezone offset
        assert!(DurableScheduler::evaluate_interval(
            Some("2025-01-01T00:00:00"),
            60_000,
            "2025-01-01T00:02:00"
        ));
    }

    #[test]
    fn evaluate_at_iso_without_offset() {
        assert!(DurableScheduler::evaluate_at(
            "2025-01-01T00:00:00",
            "2025-01-01T01:00:00"
        ));
    }

    #[test]
    fn calculate_next_run_invalid_now() {
        assert!(
            DurableScheduler::calculate_next_run("interval", None, Some(60_000), "bad").is_none()
        );
    }

    #[test]
    fn calculate_next_run_at_missing_expr() {
        assert!(
            DurableScheduler::calculate_next_run("at", None, None, "2025-01-01T00:00:00+00:00")
                .is_none()
        );
    }

    #[test]
    fn calculate_next_run_cron_missing_expr() {
        assert!(
            DurableScheduler::calculate_next_run("cron", None, None, "2025-01-01T00:00:00+00:00")
                .is_none()
        );
    }

    #[test]
    fn calculate_next_run_cron_invalid_expr() {
        // Pass a malformed cron expression
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("this is not cron"),
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none());
    }

    #[test]
    fn calculate_next_run_at_with_bad_expr() {
        let result = DurableScheduler::calculate_next_run(
            "at",
            Some("not-a-date"),
            None,
            "2025-01-01T00:00:00+00:00",
        );
        assert!(result.is_none());
    }

    // ── cron edge cases for coverage ───────────────────────────────────

    #[test]
    fn cron_slot_missed_by_more_than_60_seconds() {
        // The probe should still find the slot but delta > 60s means not due
        assert!(!DurableScheduler::evaluate_cron(
            "0 12 * * *",
            None,
            "2025-01-01T12:02:00+00:00" // 2 minutes past the slot
        ));
    }

    #[test]
    fn cron_tz_prefix_with_utc_bare() {
        // TZ=UTC (bare UTC, no offset)
        assert!(DurableScheduler::evaluate_cron(
            "TZ=UTC 0 12 * * *",
            None,
            "2025-01-01T12:00:00+00:00"
        ));
    }

    #[test]
    fn next_run_cron_with_tz_prefix() {
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("TZ=UTC-05:00 0 9 * * *"),
            None,
            "2025-01-01T10:00:00+00:00",
        );
        assert!(result.is_some());
    }

    // ── IANA timezone integration tests ─────────────────────────────────

    #[test]
    fn cron_with_iana_timezone_fires_at_local_time() {
        // Asia/Tokyo is UTC+9. "0 9 * * *" in Tokyo = 00:00 UTC.
        assert!(DurableScheduler::evaluate_cron(
            "CRON_TZ=Asia/Tokyo 0 9 * * *",
            None,
            "2025-06-15T00:00:00+00:00"
        ));
        // At 09:00 UTC, it's 18:00 in Tokyo — should NOT fire.
        assert!(!DurableScheduler::evaluate_cron(
            "CRON_TZ=Asia/Tokyo 0 9 * * *",
            None,
            "2025-06-15T09:00:00+00:00"
        ));
    }

    #[test]
    fn next_run_with_iana_timezone() {
        let result = DurableScheduler::calculate_next_run(
            "cron",
            Some("CRON_TZ=Asia/Tokyo 0 9 * * *"),
            None,
            "2025-06-15T01:00:00+00:00",
        );
        assert!(result.is_some());
        let next = result.unwrap();
        // Next 09:00 Tokyo after 2025-06-15T01:00Z (= 10:00 Tokyo)
        // should be 2025-06-16T00:00Z (= 09:00 Tokyo next day)
        assert!(next.contains("2025-06-16T00:00:00"));
    }

    // ── DST transition conformance ──────────────────────────────────────

    #[test]
    fn parse_timezone_at_dst_winter_vs_summer() {
        use chrono::TimeZone;
        let winter = Utc.with_ymd_and_hms(2025, 1, 15, 12, 0, 0).unwrap();
        let summer = Utc.with_ymd_and_hms(2025, 7, 15, 12, 0, 0).unwrap();

        let winter_offset = parse_timezone_at("America/New_York", winter).unwrap();
        let summer_offset = parse_timezone_at("America/New_York", summer).unwrap();

        // EST = UTC-5, EDT = UTC-4
        assert_eq!(winter_offset, FixedOffset::west_opt(5 * 3600).unwrap());
        assert_eq!(summer_offset, FixedOffset::west_opt(4 * 3600).unwrap());
    }

    #[test]
    fn cron_spring_forward_gap_does_not_double_fire() {
        // 2025-03-09: US spring forward. 2:00 AM becomes 3:00 AM.
        // "30 2 * * *" in America/New_York — 2:30 AM doesn't exist on this day.
        // At 07:30 UTC (would be 2:30 EST, but clock jumps to 3:30 EDT),
        // the cron should NOT fire since 2:30 AM doesn't exist.
        assert!(!DurableScheduler::evaluate_cron(
            "CRON_TZ=America/New_York 30 2 * * *",
            None,
            "2025-03-09T07:30:00+00:00"
        ));
    }

    #[test]
    fn cron_fall_back_does_not_miss() {
        // 2025-11-02: US fall back. 2:00 AM EDT becomes 1:00 AM EST.
        // "30 1 * * *" should fire at 1:30 AM local. First occurrence
        // is 1:30 AM EDT = 05:30 UTC.
        assert!(DurableScheduler::evaluate_cron(
            "CRON_TZ=America/New_York 30 1 * * *",
            None,
            "2025-11-02T05:30:00+00:00"
        ));
    }

    // ── Sub-minute cron validation ──────────────────────────────────────

    #[test]
    fn cron_every_minute_fires_correctly() {
        // "* * * * *" should fire every minute
        assert!(DurableScheduler::evaluate_cron(
            "* * * * *",
            None,
            "2025-06-15T14:37:15+00:00" // 15s after :37:00 slot
        ));
    }

    #[test]
    fn cron_step_expression_every_5_minutes() {
        assert!(DurableScheduler::evaluate_cron(
            "*/5 * * * *",
            None,
            "2025-06-15T14:35:00+00:00"
        ));
        assert!(!DurableScheduler::evaluate_cron(
            "*/5 * * * *",
            None,
            "2025-06-15T14:37:00+00:00"
        ));
    }

    #[test]
    fn cron_dedup_prevents_double_fire_within_60s_window() {
        // If last_run was 20s ago in the same slot, should NOT re-fire
        assert!(!DurableScheduler::evaluate_cron(
            "0 12 * * *",
            Some("2025-01-01T12:00:10+00:00"),
            "2025-01-01T12:00:30+00:00"
        ));
    }
}
