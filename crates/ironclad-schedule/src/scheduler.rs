use chrono::{DateTime, Datelike, Duration, NaiveDateTime, Timelike};

/// Pure-function scheduler for cron, interval, and at-style schedule evaluation.
/// No DB dependency — all state is passed in as arguments.
pub struct DurableScheduler;

impl DurableScheduler {
    /// Simplified cron evaluation: "minute hour day_of_month month day_of_week".
    /// Supports `*` (any) and specific numeric values only.
    pub fn evaluate_cron(cron_expr: &str, _last_run: Option<&str>, now: &str) -> bool {
        let now_dt = match parse_iso(now) {
            Some(dt) => dt,
            None => return false,
        };

        let fields: Vec<&str> = cron_expr.split_whitespace().collect();
        if fields.len() != 5 {
            return false;
        }

        let checks: [(u32, &str); 5] = [
            (now_dt.minute(), fields[0]),
            (now_dt.hour(), fields[1]),
            (now_dt.day(), fields[2]),
            (now_dt.month(), fields[3]),
            (now_dt.weekday().num_days_from_sunday(), fields[4]),
        ];

        checks
            .iter()
            .all(|(actual, pattern)| match_field(*actual, pattern))
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
                let next = now_dt + Duration::seconds(60);
                Some(next.and_utc().to_rfc3339())
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

fn match_field(actual: u32, pattern: &str) -> bool {
    if pattern == "*" {
        return true;
    }
    pattern.parse::<u32>().map_or(false, |v| v == actual)
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
}
