//! Cron expression parsing + timezone-aware "next fire" calculation.
//! Callers pass `now` explicitly so cron-driven code can be tested by
//! invoking the tick function with a fixed timestamp.

use std::sync::OnceLock;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use croner::Cron;

/// Default IANA timezone used when a Cron trigger's spec omits `tz`. Reads
/// `AGENT_K_DEFAULT_TZ` at first call and caches the result; invalid or
/// missing values fall back to `"UTC"` (with a warning logged for an invalid
/// value).
pub fn default_tz_name() -> &'static str {
    static CACHE: OnceLock<String> = OnceLock::new();
    CACHE
        .get_or_init(|| match std::env::var("AGENT_K_DEFAULT_TZ") {
            Ok(name) => match name.parse::<Tz>() {
                Ok(_) => name,
                Err(e) => {
                    tracing::warn!(
                        "invalid AGENT_K_DEFAULT_TZ '{name}': {e}; falling back to UTC"
                    );
                    "UTC".to_string()
                }
            },
            Err(_) => "UTC".to_string(),
        })
        .as_str()
}

pub trait Clock: Send + Sync + 'static {
    fn now(&self) -> DateTime<Utc>;
}

#[derive(Default, Clone, Copy)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Parse `expr` in `tz_name` and compute the first occurrence strictly after
/// `after`. Only Linux-style 5-field POSIX cron (`min hour dom mon dow`) is
/// accepted — sub-minute precision is rejected because the worker's tick
/// interval and per-run execution latency make sub-minute cron meaningless
/// for automation use.
pub fn next_fire_after(
    expr: &str,
    tz_name: &str,
    after: DateTime<Utc>,
) -> Result<DateTime<Utc>, String> {
    let tz: Tz = tz_name
        .parse()
        .map_err(|e: chrono_tz::ParseError| format!("invalid timezone '{tz_name}': {e}"))?;
    validate_five_field(expr)?;
    let cron = Cron::new(expr)
        .parse()
        .map_err(|e| format!("invalid cron expression '{expr}': {e}"))?;
    let after_in_tz = after.with_timezone(&tz);
    cron.find_next_occurrence(&after_in_tz, false)
        .map(|t| t.with_timezone(&Utc))
        .map_err(|e| format!("no future occurrence for '{expr}': {e}"))
}

fn validate_five_field(expr: &str) -> Result<(), String> {
    let n = expr.split_whitespace().count();
    if n == 5 {
        Ok(())
    } else {
        Err(format!(
            "only 5-field POSIX cron is supported (got {n} fields); sub-minute precision not allowed"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn five_field_expr_resolves_next_fire() {
        let now = at("2026-06-01T08:30:00Z");
        // every day at 09:00 UTC
        let next = next_fire_after("0 9 * * *", "UTC", now).unwrap();
        assert_eq!(next, at("2026-06-01T09:00:00Z"));
    }

    #[test]
    fn next_fire_in_explicit_timezone() {
        // 9 AM Seoul on 2026-06-01 = 2026-06-01T00:00:00Z
        let before = Utc.with_ymd_and_hms(2026, 5, 31, 23, 0, 0).unwrap();
        let next = next_fire_after("0 9 * * *", "Asia/Seoul", before).unwrap();
        assert_eq!(next, at("2026-06-01T00:00:00Z"));
    }

    #[test]
    fn six_field_expr_is_rejected() {
        let now = at("2026-06-01T08:30:00Z");
        // sub-minute precision not allowed — automation uses Linux cron only.
        let err = next_fire_after("30 * * * * *", "UTC", now).unwrap_err();
        assert!(err.contains("5-field"), "unexpected error: {err}");
    }

    #[test]
    fn invalid_cron_returns_error() {
        let now = at("2026-06-01T08:30:00Z");
        assert!(next_fire_after("not a cron", "UTC", now).is_err());
    }

    #[test]
    fn invalid_timezone_returns_error() {
        let now = at("2026-06-01T08:30:00Z");
        assert!(next_fire_after("0 9 * * *", "Mars/Olympus", now).is_err());
    }
}
