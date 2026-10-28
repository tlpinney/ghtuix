//! Relative ages for GitHub's UTC timestamps (`2026-10-27T22:48:06Z`).

use std::time::{SystemTime, UNIX_EPOCH};

/// Seconds since the Unix epoch for an RFC 3339 UTC timestamp.
pub fn parse(timestamp: &str) -> Option<i64> {
    let digits = |range: std::ops::Range<usize>| timestamp.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (digits(0..4)?, digits(5..7)?, digits(8..10)?);
    let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
    // Days from civil (Howard Hinnant's algorithm).
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3600 + minute * 60 + second)
}

pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

/// `45s`, `12m`, `3h`, `5d`, `8mo`, `2y` since `timestamp`; `?` when it cannot be read.
pub fn age(timestamp: &str) -> String {
    age_at(timestamp, now())
}

pub fn age_at(timestamp: &str, now: i64) -> String {
    match parse(timestamp) {
        Some(then) => duration(now - then),
        None => "?".into(),
    }
}

/// The age of a Unix time, such as when something was fetched.
pub fn age_at_seconds(then: i64) -> String {
    duration(now() - then)
}

fn duration(seconds: i64) -> String {
    match seconds.max(0) {
        s if s < 60 => format!("{s}s"),
        s if s < 3600 => format!("{}m", s / 60),
        s if s < 86_400 => format!("{}h", s / 3600),
        s if s < 30 * 86_400 => format!("{}d", s / 86_400),
        s if s < 365 * 86_400 => format!("{}mo", s / (30 * 86_400)),
        s => format!("{}y", s / (365 * 86_400)),
    }
}

/// `YYYY-MM-DD` for `days` days before now (search qualifiers such as `merged:>=`).
pub fn date_days_ago(days: i64) -> String {
    let civil = now().div_euclid(86_400) - days + 719_468;
    let era = civil.div_euclid(146_097);
    let doe = civil - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_utc_timestamps() {
        assert_eq!(parse("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse("2026-10-27T12:00:00Z"), Some(1_793_102_400));
        assert_eq!(parse("garbage"), None);
    }

    #[test]
    fn ages_are_short_and_relative() {
        let now = 1_793_102_400;
        assert_eq!(age_at("2026-10-27T11:59:30Z", now), "30s");
        assert_eq!(age_at("2026-10-27T10:00:00Z", now), "2h");
        assert_eq!(age_at("2026-10-20T12:00:00Z", now), "7d");
        assert_eq!(age_at("nope", now), "?");
    }

    #[test]
    fn dates_round_trip_through_parse() {
        let date = date_days_ago(7);
        let parsed = parse(&format!("{date}T00:00:00Z")).unwrap();
        assert!(
            (now() - 8 * 86_400..=now() - 6 * 86_400).contains(&parsed),
            "{date}"
        );
    }
}
