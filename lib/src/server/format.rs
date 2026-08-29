//! How a size and an age read on a page.
//!
//! Both renderings appear on the feed page and again in the admin, so they
//! live here rather than in a handler. That keeps one wording for each, and
//! it lets both be tested without a request.
//!
//! Each takes the absent case as a value rather than leaving the caller to
//! branch. A feed states neither a size nor a date reliably, so the pages
//! always have something to print.

use chrono::{DateTime, Utc};

/// The steps above a plain byte count, each 1024 of the one before.
const SIZE_UNITS: [&str; 4] = ["KB", "MB", "GB", "TB"];

/// Spans in seconds, largest first, so the first that fits is the one shown.
const AGE_UNITS: [(i64, &str); 3] = [(86_400, "day"), (3_600, "hour"), (60, "minute")];

/// Renders a release size.
///
/// A count below a kilobyte reads exactly, because a torrent that small is
/// an oddity worth seeing precisely. Anything larger rounds to one decimal,
/// which is as much as a listing has room for.
pub(super) fn size(bytes: Option<u64>) -> String {
    let Some(bytes) = bytes else {
        return "size unknown".to_owned();
    };

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64 / 1024.0;
    let mut unit = SIZE_UNITS[0];

    for next in &SIZE_UNITS[1..] {
        if value < 1024.0 {
            break;
        }

        value /= 1024.0;
        unit = next;
    }

    format!("{value:.1} {unit}")
}

/// Renders `amount` beside the noun it counts, in the form the count needs.
///
/// Both forms are given rather than derived, because an `s` is not always
/// what a plural adds. The admin counts `matches`, not `matchs`.
pub(super) fn count(amount: usize, singular: &str, plural: &str) -> String {
    let noun = if amount == 1 { singular } else { plural };

    format!("{amount} {noun}")
}

/// Renders how long ago `then` was, in the largest unit that fits whole.
///
/// A time in the future reads as `just now` rather than as a negative span.
/// A tracker with a skewed clock publishes dates slightly ahead, and a
/// listing that says `in -3 minutes` is worse than a small rounding.
pub(super) fn age(now: DateTime<Utc>, then: Option<DateTime<Utc>>) -> String {
    let Some(then) = then else {
        return "undated".to_owned();
    };

    let seconds = now.signed_duration_since(then).num_seconds();
    if seconds < 60 {
        return "just now".to_owned();
    }

    for (span, unit) in AGE_UNITS {
        let count = seconds / span;

        if count > 0 {
            let plural = if count == 1 { "" } else { "s" };
            return format!("{count} {unit}{plural} ago");
        }
    }

    // The minute step divides anything from 60 seconds up, and the guard
    // above rules out less, so the loop always returns.
    unreachable!("every span of a minute or more fits a unit")
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, TimeDelta, TimeZone, Utc};

    use super::{age, count, size};

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2025, 3, 4, 12, 0, 0)
            .single()
            .expect("the test time is unambiguous")
    }

    fn ago(seconds: i64) -> Option<DateTime<Utc>> {
        Some(now() - TimeDelta::seconds(seconds))
    }

    #[test]
    fn size_rounds_to_one_decimal() {
        assert_eq!(size(None), "size unknown");
        assert_eq!(size(Some(0)), "0 B");
        assert_eq!(size(Some(512)), "512 B");
        assert_eq!(size(Some(1023)), "1023 B", "a byte short of a kilobyte");
        assert_eq!(size(Some(1024)), "1.0 KB");
        assert_eq!(size(Some(1536)), "1.5 KB");
        assert_eq!(size(Some(2_576_980_378)), "2.4 GB");
        assert_eq!(size(Some(1024_u64.pow(4))), "1.0 TB");
        assert_eq!(
            size(Some(1024_u64.pow(5))),
            "1024.0 TB",
            "beyond a terabyte the unit stops climbing"
        );
    }

    #[test]
    fn count_agrees_with_its_number() {
        assert_eq!(count(0, "item", "items"), "0 items", "none is plural");
        assert_eq!(count(1, "item", "items"), "1 item");
        assert_eq!(count(2, "item", "items"), "2 items");
        assert_eq!(
            count(1, "match", "matches"),
            "1 match",
            "the plural is given, not derived"
        );
        assert_eq!(count(3, "match", "matches"), "3 matches");
    }

    #[test]
    fn age_picks_largest_whole_unit() {
        assert_eq!(age(now(), None), "undated");
        assert_eq!(age(now(), ago(0)), "just now");
        assert_eq!(age(now(), ago(59)), "just now");
        assert_eq!(age(now(), ago(-600)), "just now", "a skewed tracker clock");
        assert_eq!(age(now(), ago(60)), "1 minute ago");
        assert_eq!(age(now(), ago(180)), "3 minutes ago");
        assert_eq!(age(now(), ago(3_599)), "59 minutes ago");
        assert_eq!(age(now(), ago(3_600)), "1 hour ago");
        assert_eq!(age(now(), ago(7_200)), "2 hours ago");
        assert_eq!(age(now(), ago(86_399)), "23 hours ago");
        assert_eq!(age(now(), ago(86_400)), "1 day ago");
        assert_eq!(age(now(), ago(432_000)), "5 days ago");
    }
}
