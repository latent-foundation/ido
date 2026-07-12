//! Shared date math: leap-year / month-length helpers, weekday calculation,
//! `YYYY-MM-DD` parsing/formatting, and calendar arithmetic (`add_days`,
//! `add_months`). Backs the date picker, the task board's due-date display,
//! and the calendar month-grid component. Everything here is pure except
//! [`today`] (and the [`today_ymd`] / [`this_year`] built on it), which reads
//! the JS clock — so those three are never unit-tested, and no other function
//! in this module calls them internally; callers pass "today" in explicitly.

/// Full month names, January-first.
pub(crate) const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
/// Abbreviated month names, January-first.
pub(crate) const MONTHS_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
/// Weekday column headers, Monday-first (the European / ISO-8601 week order).
pub(crate) const WEEKDAYS: [&str; 7] = ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"];

pub(crate) fn is_leap(y: i32) -> bool {
    y % 4 == 0 && (y % 100 != 0 || y % 400 == 0)
}

/// Days in month `m0` (0-based) of year `y`.
pub(crate) fn days_in_month(y: i32, m0: u32) -> u32 {
    match m0 {
        1 => {
            if is_leap(y) {
                29
            } else {
                28
            }
        }
        3 | 5 | 8 | 10 => 30,
        _ => 31,
    }
}

/// Weekday of `y-m1-d` (Sakamoto's algorithm), 0 = Sunday. `m1` is 1..=12.
pub(crate) fn weekday(y: i32, m1: u32, d: u32) -> u32 {
    let t = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if m1 < 3 { y - 1 } else { y };
    let w = (y + y / 4 - y / 100 + y / 400 + t[(m1 - 1) as usize] + d as i32) % 7;
    ((w + 7) % 7) as u32
}

/// Parse `YYYY-MM-DD` into `(year, month0, day)`.
pub(crate) fn parse_ymd(s: &str) -> Option<(i32, u32, u32)> {
    let mut it = s.split('-');
    let y: i32 = it.next()?.parse().ok()?;
    let m: u32 = it.next()?.parse().ok()?;
    let d: u32 = it.next()?.parse().ok()?;
    if it.next().is_none() && (1..=12).contains(&m) && (1..=31).contains(&d) {
        Some((y, m - 1, d))
    } else {
        None
    }
}

/// `(year, month0, day)` formatted as zero-padded `YYYY-MM-DD`.
pub(crate) fn ymd(y: i32, m0: u32, d: u32) -> String {
    format!("{y:04}-{:02}-{:02}", m0 + 1, d)
}

/// Today as `(year, month0, day)`, from the JS clock.
pub(crate) fn today() -> (i32, u32, u32) {
    let now = js_sys::Date::new_0();
    (now.get_full_year() as i32, now.get_month(), now.get_date())
}

/// Today as `YYYY-MM-DD` (the JS clock) — compares chronologically as text.
pub(crate) fn today_ymd() -> String {
    let (y, m0, d) = today();
    ymd(y, m0, d)
}

/// The current year (the JS clock), for `format_due`'s year elision, or a
/// calendar header.
pub(crate) fn this_year() -> i32 {
    today().0
}

/// `y-m0-d` shifted by `n` days (negative goes backward), rolling over months
/// and years — including leap Februaries. Implemented via Howard Hinnant's
/// `days_from_civil` / `civil_from_days` (proleptic Gregorian day-count
/// conversion), so arbitrarily large `n` needs no iteration. Backs the calendar
/// month-grid's leading/trailing days (`calendar::grid_days`).
pub(crate) fn add_days(y: i32, m0: u32, d: u32, n: i32) -> (i32, u32, u32) {
    civil_from_days(days_from_civil(y, m0 + 1, d) + n as i64)
}

/// `y-m0-d` shifted by `n` months (negative goes backward), rolling over
/// years; the day is clamped to the target month's length (e.g. 31 Jan + 1
/// month → 28 or 29 Feb). Backs the calendar's prev/next-month navigation
/// (`State::cal_shift_month`).
pub(crate) fn add_months(y: i32, m0: u32, d: u32, n: i32) -> (i32, u32, u32) {
    let total = y as i64 * 12 + m0 as i64 + n as i64;
    let ny = total.div_euclid(12) as i32;
    let nm0 = total.rem_euclid(12) as u32;
    let nd = d.min(days_in_month(ny, nm0));
    (ny, nm0, nd)
}

/// Days since the epoch (1970-01-01) for `y-m1-d` (`m1` is 1..=12), per
/// Howard Hinnant's `days_from_civil` algorithm — valid for any proleptic
/// Gregorian date, including negative/pre-epoch years.
fn days_from_civil(y: i32, m1: u32, d: u32) -> i64 {
    let y = if m1 <= 2 { y as i64 - 1 } else { y as i64 };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (m1 as i64 + 9) % 12; // [0, 11], Mar-based
    let doy = (153 * mp + 2) / 5 + d as i64 - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146097 + doe - 719468
}

/// Inverse of [`days_from_civil`]: a day-count since the epoch back to
/// `(year, month0, day)`.
fn civil_from_days(z: i64) -> (i32, u32, u32) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11], Mar-based
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m1 = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let y = if m1 <= 2 { y + 1 } else { y };
    (y as i32, (m1 - 1) as u32, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn date_math() {
        assert!(is_leap(2024) && is_leap(2000));
        assert!(!is_leap(2025) && !is_leap(2100));
        assert_eq!(days_in_month(2024, 1), 29); // Feb, leap
        assert_eq!(days_in_month(2025, 1), 28); // Feb, common
        assert_eq!(days_in_month(2026, 6), 31); // July
        assert_eq!(days_in_month(2026, 3), 30); // April
                                                // 1 July 2026 is a Wednesday (0 = Sunday).
        assert_eq!(weekday(2026, 7, 1), 3);
        // Monday-first: Wednesday sits in column index 2 (Mo, Tu, We…).
        assert_eq!((weekday(2026, 7, 1) + 6) % 7, 2);
        assert_eq!(parse_ymd("2026-07-31"), Some((2026, 6, 31)));
        assert_eq!(parse_ymd(""), None);
        assert_eq!(parse_ymd("2026-13-01"), None); // bad month
    }

    #[test]
    fn ymd_pads_month_and_day() {
        assert_eq!(ymd(2026, 6, 7), "2026-07-07");
        assert_eq!(ymd(2026, 0, 1), "2026-01-01");
        assert_eq!(ymd(9, 11, 3), "0009-12-03");
    }

    #[test]
    fn add_days_within_month() {
        assert_eq!(add_days(2026, 6, 15, 5), (2026, 6, 20));
        assert_eq!(add_days(2026, 6, 15, -5), (2026, 6, 10));
        assert_eq!(add_days(2026, 6, 15, 0), (2026, 6, 15));
    }

    #[test]
    fn add_days_rolls_over_month_boundaries() {
        // 30 Jun + 1 -> 1 Jul (30-day month rollover, no year change).
        assert_eq!(add_days(2026, 5, 30, 1), (2026, 6, 1));
        // 1 Jul - 1 -> 30 Jun (backward across the same boundary).
        assert_eq!(add_days(2026, 6, 1, -1), (2026, 5, 30));
        // 31 Dec + 1 -> 1 Jan next year.
        assert_eq!(add_days(2026, 11, 31, 1), (2027, 0, 1));
        // 1 Jan - 1 -> 31 Dec previous year.
        assert_eq!(add_days(2026, 0, 1, -1), (2025, 11, 31));
    }

    #[test]
    fn add_days_handles_leap_february() {
        // 28 Feb + 1 -> 29 Feb in a leap year.
        assert_eq!(add_days(2024, 1, 28, 1), (2024, 1, 29));
        // 29 Feb + 1 -> 1 Mar in a leap year.
        assert_eq!(add_days(2024, 1, 29, 1), (2024, 2, 1));
        // 28 Feb + 1 -> 1 Mar in a common year (no 29 Feb).
        assert_eq!(add_days(2025, 1, 28, 1), (2025, 2, 1));
        // 1 Mar - 1 -> 29 Feb in a leap year.
        assert_eq!(add_days(2024, 2, 1, -1), (2024, 1, 29));
    }

    #[test]
    fn add_days_spans_many_days_without_iteration() {
        // A month-grid can request a large jump; this should still resolve
        // correctly in one shot via the day-count round trip.
        assert_eq!(add_days(2026, 0, 1, 365), (2027, 0, 1)); // common year, 365 days
                                                             // 2024 is a leap year (366 days) and its span includes 29 Feb.
        assert_eq!(add_days(2024, 0, 1, 365), (2024, 11, 31));
        assert_eq!(add_days(2024, 0, 1, 366), (2025, 0, 1));
    }

    #[test]
    fn add_months_within_year() {
        assert_eq!(add_months(2026, 2, 10, 3), (2026, 5, 10));
        assert_eq!(add_months(2026, 5, 10, -3), (2026, 2, 10));
        assert_eq!(add_months(2026, 5, 10, 0), (2026, 5, 10));
    }

    #[test]
    fn add_months_rolls_over_year_boundaries() {
        // Nov + 3 -> Feb next year.
        assert_eq!(add_months(2026, 10, 5, 3), (2027, 1, 5));
        // Jan - 1 -> Dec previous year.
        assert_eq!(add_months(2026, 0, 5, -1), (2025, 11, 5));
        // Jan + 24 -> Jan two years later.
        assert_eq!(add_months(2026, 0, 5, 24), (2028, 0, 5));
    }

    #[test]
    fn add_months_clamps_day_to_target_month_length() {
        // 31 Jan + 1 month -> 28 Feb (common year).
        assert_eq!(add_months(2026, 0, 31, 1), (2026, 1, 28));
        // 31 Jan + 1 month -> 29 Feb (leap year).
        assert_eq!(add_months(2024, 0, 31, 1), (2024, 1, 29));
        // 29 Feb (leap) + 12 months -> 28 Feb (next year is common).
        assert_eq!(add_months(2024, 1, 29, 12), (2025, 1, 28));
        // 31 Mar - 1 month -> 28 Feb (common year).
        assert_eq!(add_months(2026, 2, 31, -1), (2026, 1, 28));
    }
}
