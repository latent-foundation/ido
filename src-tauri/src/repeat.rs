//! Pure recurrence math for a task's `repeat:` rule. No I/O, no commands — a
//! `paths`-style module the tasks store calls to compute the next occurrence's
//! due date when a recurring task is completed (see `tasks::spawn_next`).
//!
//! The vocabulary is deliberately tiny (no RRULE authoring):
//!
//! ```text
//! daily | weekly | monthly | yearly | every N days | every N weeks | every N months
//! ```
//!
//! Parsing is lenient: the spec is trimmed, lowercased, and a trailing `s` on
//! the unit is optional (so `every 1 day` and `every 2 weeks` both parse).
//!
//! All calendar arithmetic is [`chrono::NaiveDate`]'s — never hand-rolled.
//! Month/year steps use [`chrono::NaiveDate::checked_add_months`], which already
//! clamps the day (31 Jan + 1 month → 28/29 Feb; a leap 29 Feb + 1 year → 28
//! Feb).

use chrono::{Days, Months, NaiveDate};

/// A parsed recurrence step, normalised to whole days or whole months.
enum Step {
    /// Advance by this many days (covers `daily` / `weekly` / `every N days|weeks`).
    Days(u64),
    /// Advance by this many months (covers `monthly` / `yearly` / `every N months|years`).
    Months(u32),
}

/// Parse a `repeat:` spec into a [`Step`], or `None` if it isn't recognised.
/// Case-, whitespace-, and (unit) plural-insensitive; `N` must be `>= 1`.
fn parse_step(spec: &str) -> Option<Step> {
    let spec = spec.trim().to_ascii_lowercase();
    match spec.as_str() {
        "daily" => return Some(Step::Days(1)),
        "weekly" => return Some(Step::Days(7)),
        "monthly" => return Some(Step::Months(1)),
        "yearly" => return Some(Step::Months(12)),
        _ => {}
    }
    // The only other shape is `every N <unit>`.
    let mut tokens = spec.split_whitespace();
    if tokens.next()? != "every" {
        return None;
    }
    let n: u64 = tokens.next()?.parse().ok()?;
    let unit = tokens.next()?;
    if tokens.next().is_some() {
        return None; // trailing junk — not a spec we understand
    }
    if n < 1 {
        return None;
    }
    let unit = unit.strip_suffix('s').unwrap_or(unit);
    match unit {
        "day" => Some(Step::Days(n)),
        "week" => Some(Step::Days(n.checked_mul(7)?)),
        "month" => Some(Step::Months(u32::try_from(n).ok()?)),
        "year" => Some(Step::Months(u32::try_from(n).ok()?.checked_mul(12)?)),
        _ => None,
    }
}

/// Parse a `YYYY-MM-DD` date, ignoring surrounding whitespace.
fn parse_ymd(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%Y-%m-%d").ok()
}

/// The next occurrence's due date as `YYYY-MM-DD`, or `None` when `spec` doesn't
/// parse. The base is the parsed `due`, falling back to `today` when `due` is
/// empty or unparseable (so a due-less recurring task anchors to completion day).
///
/// This is a *single* advance — no catch-up looping. A weekly task completed a
/// month late spawns an already-overdue next occurrence anchored to its original
/// weekday; that is intended (the user sees it's overdue and can act).
pub fn advance(due: &str, spec: &str, today: &str) -> Option<String> {
    let step = parse_step(spec)?;
    let base = parse_ymd(due).or_else(|| parse_ymd(today))?;
    let next = match step {
        Step::Days(n) => base.checked_add_days(Days::new(n))?,
        Step::Months(n) => base.checked_add_months(Months::new(n))?,
    };
    Some(next.format("%Y-%m-%d").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `advance` with a fixed reference "today" (a Wednesday) for the tables.
    fn adv(due: &str, spec: &str) -> Option<String> {
        advance(due, spec, "2026-07-08")
    }

    #[test]
    fn recurrence_table() {
        // Plain intervals.
        assert_eq!(adv("2026-07-08", "daily").as_deref(), Some("2026-07-09"));
        // Weekly stays weekday-anchored: 2026-07-08 and -15 are both Wednesdays.
        assert_eq!(adv("2026-07-08", "weekly").as_deref(), Some("2026-07-15"));
        // Monthly clamps: 31 Jan + 1 month → 28 Feb (2026 is not a leap year).
        assert_eq!(adv("2026-01-31", "monthly").as_deref(), Some("2026-02-28"));
        // …and to 29 Feb in a leap year.
        assert_eq!(adv("2024-01-31", "monthly").as_deref(), Some("2024-02-29"));
        // Yearly off a leap day clamps to 28 Feb the next year.
        assert_eq!(adv("2024-02-29", "yearly").as_deref(), Some("2025-02-28"));
        // every N …
        assert_eq!(
            adv("2026-07-08", "every 2 weeks").as_deref(),
            Some("2026-07-22")
        );
        assert_eq!(
            adv("2026-07-08", "every 3 days").as_deref(),
            Some("2026-07-11")
        );
        // every 2 months, with a clamp: 31 Dec → 28 Feb.
        assert_eq!(
            adv("2025-12-31", "every 2 months").as_deref(),
            Some("2026-02-28")
        );
    }

    #[test]
    fn empty_or_bad_due_falls_back_to_today() {
        assert_eq!(
            advance("", "daily", "2026-07-08").as_deref(),
            Some("2026-07-09")
        );
        assert_eq!(
            advance("not-a-date", "weekly", "2026-07-08").as_deref(),
            Some("2026-07-15")
        );
    }

    #[test]
    fn garbage_spec_is_none() {
        assert_eq!(adv("2026-07-08", "whenever-i-feel-like-it"), None);
        assert_eq!(adv("2026-07-08", ""), None);
        assert_eq!(adv("2026-07-08", "every 0 days"), None);
        assert_eq!(adv("2026-07-08", "every two weeks"), None);
        assert_eq!(adv("2026-07-08", "every 2"), None);
        assert_eq!(adv("2026-07-08", "monthly extra"), None);
    }

    #[test]
    fn lenient_about_case_whitespace_and_plurals() {
        assert_eq!(
            adv("2026-07-08", "  WEEKLY  ").as_deref(),
            Some("2026-07-15")
        );
        assert_eq!(
            adv("2026-07-08", "Every 1 Day").as_deref(),
            Some("2026-07-09")
        );
        // Singular unit form is accepted.
        assert_eq!(
            adv("2026-07-08", "every 2 week").as_deref(),
            Some("2026-07-22")
        );
        assert_eq!(
            adv("2026-07-08", "every 1 month").as_deref(),
            Some("2026-08-08")
        );
    }
}
