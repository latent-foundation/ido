//! Pure helpers for the board: display labels, search / goal filtering, the
//! overdue check, drag drop-position math, and column sorting. No reactive
//! state — just data in, data out.

use wasm_bindgen::JsCast;

use crate::dates;
use crate::model::Task;
use crate::state::TaskSort;

/// A column id rendered for display (`in-progress` → "in progress").
pub(super) fn col_label(name: &str) -> String {
    name.replace('-', " ")
}

/// Whether `task` matches the search `filter` (by display title, id, or any tag).
pub(super) fn matches(task: &Task, filter: &str) -> bool {
    if filter.is_empty() {
        return true;
    }
    let f = filter.to_lowercase();
    task.title.to_lowercase().contains(&f)
        || task.id.to_lowercase().contains(&f)
        || task.tags.iter().any(|t| t.to_lowercase().contains(&f))
}

/// Whether `task` passes the active goal filter (`None` = all goals).
pub(super) fn goal_ok(task: &Task, filter: &Option<String>) -> bool {
    match filter {
        Some(g) => &task.goal == g,
        None => true,
    }
}

/// Whether `task` passes the active tag filter (`None` = all tags). Exact,
/// case-sensitive match — tags render as typed, no normalisation.
pub(super) fn tag_ok(task: &Task, filter: &Option<String>) -> bool {
    match filter {
        Some(tag) => task.tags.iter().any(|t| t == tag),
        None => true,
    }
}

/// Whether a goal is overdue: it has a target date in the past and isn't yet
/// complete (`done < total`, or no tasks).
pub(super) fn goal_overdue(target: &str, done: usize, total: usize) -> bool {
    if target.is_empty() || (total > 0 && done == total) {
        return false;
    }
    target < dates::today_ymd().as_str()
}

/// A card due date's urgency, as its extra CSS class: `"overdue"` (past),
/// `"due-today"`, or `""`. Done-column cards are never urgent — the work is
/// done. Compares only the **date prefix** (first 10 chars, `YYYY-MM-DD`) —
/// `due` may carry an optional ` HH:MM` suffix (P2.1), and comparing the full
/// string would make a timed due today read as later-than-today and lose its
/// `due-today` flag. Mirrors [`overdue_count`]'s idiom.
pub(super) fn due_state(due: &str, today: &str, is_done: bool) -> &'static str {
    if due.is_empty() || is_done {
        return "";
    }
    let prefix = &due[..due.len().min(10)];
    if prefix < today {
        "overdue"
    } else if prefix == today {
        "due-today"
    } else {
        ""
    }
}

/// Whether a task's `due` falls on calendar day `day` (`YYYY-MM-DD`): compares
/// only the date prefix, so a timed `due` (`YYYY-MM-DD HH:MM`, P2.1) still
/// buckets into its day cell instead of vanishing from the grid. Backs the
/// calendar's day-cell and day-popover bucketing.
pub(super) fn due_on_day(due: &str, day: &str) -> bool {
    !due.is_empty() && &due[..due.len().min(10)] == day
}

/// Drag-to-reschedule: `due` moved onto `new_day` (`YYYY-MM-DD`), preserving a
/// timed due's ` HH:MM` suffix (replacing only the date prefix); a date-only
/// (or empty) due becomes date-only. Backs the calendar's day-cell drop
/// handler — the unschedule tray clears the field outright instead.
pub(super) fn reschedule(due: &str, new_day: &str) -> String {
    if due.len() > 10 {
        format!("{new_day}{}", &due[10..])
    } else {
        new_day.to_string()
    }
}

/// Count of tasks overdue *right now*, across the whole well: not archived,
/// carrying a due date, not sitting in the done column (`done_col`, the
/// board's last column), and overdue by [`due_state`]'s rule — the date
/// *prefix* (first 10 chars, `YYYY-MM-DD`) strictly before `today`, so a
/// `YYYY-MM-DD HH:MM` due value still compares correctly. Deliberately
/// unfiltered (no [`matches`]/[`goal_ok`] scoping) — this backs the rail's
/// global overdue badge, not the board.
pub(crate) fn overdue_count(tasks: &[Task], done_col: &str, today: &str) -> usize {
    tasks
        .iter()
        .filter(|t| {
            !t.archived && !t.due.is_empty() && t.status != done_col && {
                let prefix = &t.due[..t.due.len().min(10)];
                prefix < today
            }
        })
        .count()
}

/// A `YYYY-MM-DD` (optionally ` HH:MM`, P2.1) due date rendered day-first
/// ("7 Jul", or "7 Jul 2027" with the year when it isn't `this_year`; "7 Jul
/// 14:00" / "7 Jul 2027 14:00" when a time is present). Unparseable input
/// passes through as-is.
pub(super) fn format_due(due: &str, this_year: i32) -> String {
    let (date_part, time_part) = match due.split_once(' ') {
        Some((d, t)) => (d, Some(t)),
        None => (due, None),
    };
    let mut it = date_part.split('-');
    let parsed = (|| {
        let y: i32 = it.next()?.parse().ok()?;
        let m: usize = it.next()?.parse().ok()?;
        let d: u32 = it.next()?.parse().ok()?;
        if it.next().is_none() && (1..=12).contains(&m) {
            Some((y, m, d))
        } else {
            None
        }
    })();
    match parsed {
        Some((y, m, d)) => {
            let base = if y == this_year {
                format!("{d} {}", dates::MONTHS_SHORT[m - 1])
            } else {
                format!("{d} {} {y}", dates::MONTHS_SHORT[m - 1])
            };
            match time_part {
                Some(t) => format!("{base} {t}"),
                None => base,
            }
        }
        None => due.to_string(),
    }
}

/// The id of the item the cursor would drop *before* — the first descendant
/// of `ev`'s current target matching `selector` whose vertical midpoint is
/// below the cursor. `None` = below every match (append). Measuring against
/// midpoints keeps the insert point stable across the gaps between items; the
/// dragged item itself is skipped. Shared by the board (`.ido-task-card`,
/// scoped to a column) and the backlog (`.ido-backlog-row[data-orphan="false"]`,
/// scoped to the backlog list) — vertical lists, same math either way. The
/// backlog's selector excludes orphan rows so they can never be a drop
/// target: the query naturally resolves to the next true-backlog row after
/// an orphan in DOM order (or `None`, i.e. append, if it's the last row) —
/// see the "orphan rows" note on `board::Backlog`.
pub(super) fn drop_before_at(
    ev: &web_sys::DragEvent,
    dragged: &str,
    selector: &str,
) -> Option<String> {
    let y = ev.client_y() as f64;
    let container = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
    let items = container.query_selector_all(selector).ok()?;
    for i in 0..items.length() {
        let item = items.get(i)?.dyn_into::<web_sys::Element>().ok()?;
        let id = item.get_attribute("data-task-id");
        if id.as_deref() == Some(dragged) {
            continue;
        }
        let rect = item.get_bounding_client_rect();
        if y < rect.top() + rect.height() / 2.0 {
            return id;
        }
    }
    None
}

/// The `.ido-task-card` elements directly under `container` (a column), in DOM
/// order — which matches the board's current sort, since cards render in that
/// order.
fn cards_in(container: &web_sys::Element) -> Vec<web_sys::Element> {
    let mut out = Vec::new();
    let Ok(list) = container.query_selector_all(".ido-task-card") else {
        return out;
    };
    for i in 0..list.length() {
        if let Some(card) = list
            .get(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        {
            out.push(card);
        }
    }
    out
}

/// Give keyboard focus to the `idx`-th card in `container`, if it has one.
fn focus_card(container: &web_sys::Element, idx: usize) {
    if let Some(el) = cards_in(container)
        .into_iter()
        .nth(idx)
        .and_then(|c| c.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = el.focus();
    }
}

/// Roving focus within a column (`ArrowUp`/`ArrowDown`): move from the
/// focused card (`ev`'s current target, id `id`) to the sibling `delta` rows
/// away (`-1`/`+1`), if one exists. Mirrors `drop_before_at`: reads the live
/// DOM via `data-task-id`, no reactive state.
pub(super) fn focus_row(ev: &web_sys::KeyboardEvent, id: &str, delta: i32) {
    let Some(card) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    let Some(column) = card.closest(".ido-column").ok().flatten() else {
        return;
    };
    let ids: Vec<Option<String>> = cards_in(&column)
        .iter()
        .map(|c| c.get_attribute("data-task-id"))
        .collect();
    let Some(row) = ids.iter().position(|c| c.as_deref() == Some(id)) else {
        return;
    };
    let next = row as i32 + delta;
    if next < 0 {
        return;
    }
    focus_card(&column, next as usize);
}

/// Roving focus across columns (`ArrowLeft`/`ArrowRight`): move from the
/// focused card (`ev`'s current target, id `id`) to the card at the same row
/// in the next non-empty column `delta` steps away (`-1`/`+1`), clamped to
/// that column's last card when it's shorter. Empty columns are skipped
/// entirely; stops at the board's edge. Walks `.ido-column` siblings directly
/// (a column's only siblings under `.ido-board`) rather than re-querying the
/// whole board.
pub(super) fn focus_column(ev: &web_sys::KeyboardEvent, id: &str, delta: i32) {
    let Some(card) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return;
    };
    let Some(column) = card.closest(".ido-column").ok().flatten() else {
        return;
    };
    let ids: Vec<Option<String>> = cards_in(&column)
        .iter()
        .map(|c| c.get_attribute("data-task-id"))
        .collect();
    let Some(row) = ids.iter().position(|c| c.as_deref() == Some(id)) else {
        return;
    };

    let mut cur = column;
    loop {
        let next = if delta < 0 {
            cur.previous_element_sibling()
        } else {
            cur.next_element_sibling()
        };
        let Some(next) = next else {
            return;
        };
        cur = next;
        let cards = cards_in(&cur);
        if cards.is_empty() {
            continue;
        }
        focus_card(&cur, row.min(cards.len() - 1));
        return;
    }
}

/// The id of the goal chip the cursor would drop *before* — the first chip whose
/// horizontal midpoint is right of the cursor (the goals bar is a horizontal
/// strip). `None` = past the last chip (append). The dragged chip is skipped.
pub(super) fn goal_drop_before_at(ev: &web_sys::DragEvent, dragged: &str) -> Option<String> {
    let x = ev.client_x() as f64;
    let bar = ev.current_target()?.dyn_into::<web_sys::Element>().ok()?;
    let chips = bar.query_selector_all("[data-goal-id]").ok()?;
    for i in 0..chips.length() {
        let chip = chips.get(i)?.dyn_into::<web_sys::Element>().ok()?;
        let id = chip.get_attribute("data-goal-id");
        if id.as_deref() == Some(dragged) {
            continue;
        }
        let rect = chip.get_bounding_client_rect();
        if x < rect.left() + rect.width() / 2.0 {
            return id;
        }
    }
    None
}

/// Priority sort rank (high first, none last).
pub(super) fn priority_rank(p: &str) -> u8 {
    match p {
        "high" => 0,
        "normal" => 1,
        "low" => 2,
        _ => 3,
    }
}

/// Due-date sort key — non-empty dates first (chronological), empties last.
pub(super) fn due_key(d: &str) -> (u8, &str) {
    if d.is_empty() {
        (1, d)
    } else {
        (0, d)
    }
}

/// Order `tasks` per the chosen sort (ties fall back to manual `order`).
pub(super) fn sorted(mut tasks: Vec<Task>, sort: TaskSort) -> Vec<Task> {
    match sort {
        TaskSort::Manual => tasks.sort_by_key(|t| t.order),
        TaskSort::Priority => tasks.sort_by(|a, b| {
            priority_rank(&a.priority)
                .cmp(&priority_rank(&b.priority))
                .then(a.order.cmp(&b.order))
        }),
        TaskSort::Due => tasks.sort_by(|a, b| {
            due_key(&a.due)
                .cmp(&due_key(&b.due))
                .then(a.order.cmp(&b.order))
        }),
    }
    tasks
}

pub(super) fn parse_sort(v: &str) -> TaskSort {
    match v {
        "priority" => TaskSort::Priority,
        "due" => TaskSort::Due,
        _ => TaskSort::Manual,
    }
}

// --- tag autocomplete (P2.5) ----------------------------------------------

/// Distinct tags across non-archived `tasks`, in first-seen order, with the
/// **first-seen casing** kept for display — case-insensitively deduped (`bug`
/// on one task and `Bug` on another collapse to whichever was seen first).
/// Backs [`super::tagsinput::TagsInput`]'s vocabulary, built as a `Memo` over
/// `state.tasks` so it only recomputes when the task list actually changes.
pub(super) fn tag_vocabulary(tasks: &[Task]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for t in tasks.iter().filter(|t| !t.archived) {
        for tag in &t.tags {
            if seen.insert(tag.to_lowercase()) {
                out.push(tag.clone());
            }
        }
    }
    out
}

/// Split a tags field's raw text on its last comma into `(the earlier tags —
/// already committed, trimmed, blanks dropped; the fragment currently being
/// typed after it, trimmed)`. The fragment drives suggestion filtering; the
/// earlier tags are excluded from them and kept verbatim by
/// [`accept_suggestion`].
pub(super) fn split_fragment(text: &str) -> (Vec<String>, String) {
    let mut parts: Vec<&str> = text.split(',').collect();
    let fragment = parts.pop().unwrap_or("").trim().to_string();
    let existing = parts
        .into_iter()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (existing, fragment)
}

/// Rank `vocabulary` against the fragment currently being typed (text after
/// the last comma, already trimmed — see [`split_fragment`]) for the tags
/// autocomplete popover: case-insensitive **prefix** matches first, then
/// case-insensitive **substring** matches, alphabetical within each band;
/// tags already present in `existing` (the field's earlier comma-separated
/// tags) are excluded. An empty fragment yields no suggestions — the popover
/// only opens once typing starts (P2.5's spec).
pub(super) fn tag_suggestions(
    vocabulary: &[String],
    fragment: &str,
    existing: &[String],
) -> Vec<String> {
    if fragment.is_empty() {
        return Vec::new();
    }
    let f = fragment.to_lowercase();
    let existing_lower: std::collections::HashSet<String> =
        existing.iter().map(|t| t.to_lowercase()).collect();
    let mut prefix = Vec::new();
    let mut substring = Vec::new();
    for tag in vocabulary {
        let lower = tag.to_lowercase();
        if existing_lower.contains(&lower) {
            continue;
        }
        if lower.starts_with(&f) {
            prefix.push(tag.clone());
        } else if lower.contains(&f) {
            substring.push(tag.clone());
        }
    }
    prefix.sort_by_key(|t| t.to_lowercase());
    substring.sort_by_key(|t| t.to_lowercase());
    prefix.into_iter().chain(substring).collect()
}

/// The tags field's new raw text after accepting `canonical` for the
/// fragment being typed: the earlier tags (from [`split_fragment`]) are kept,
/// re-joined `", "`-separated, `canonical` replaces the fragment, and a
/// trailing `", "` is left so typing continues straight into the next tag.
/// Pure so the reconstruction is host-testable independent of the DOM.
pub(super) fn accept_suggestion(text: &str, canonical: &str) -> String {
    let (existing, _) = split_fragment(text);
    let mut out = existing.join(", ");
    if !out.is_empty() {
        out.push_str(", ");
    }
    out.push_str(canonical);
    out.push_str(", ");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal task for the overdue-count tests.
    fn task(status: &str, due: &str, archived: bool) -> Task {
        Task {
            id: "t".into(),
            title: "t".into(),
            status: status.into(),
            priority: String::new(),
            tags: Vec::new(),
            order: 0,
            due: due.into(),
            goal: String::new(),
            repeat: String::new(),
            archived,
            completed: String::new(),
            checks_done: 0,
            checks_total: 0,
            body: String::new(),
        }
    }

    #[test]
    fn overdue_count_counts_only_true_overdue() {
        let today = "2026-07-07";
        let tasks = vec![
            // Overdue: past due, open column, not archived.
            task("todo", "2026-07-06", false),
            // Due today isn't overdue.
            task("todo", "2026-07-07", false),
            // Past due but sitting in the done column doesn't count.
            task("done", "2026-07-01", false),
            // Past due but archived doesn't count.
            task("todo", "2026-01-01", true),
            // No due date at all doesn't count.
            task("todo", "", false),
        ];
        assert_eq!(overdue_count(&tasks, "done", today), 1);
    }

    #[test]
    fn overdue_count_compares_the_date_prefix() {
        let today = "2026-07-07";
        // A `YYYY-MM-DD HH:MM` due value still compares by its date prefix.
        let past = task("todo", "2026-07-06 09:00", false);
        let future = task("todo", "2026-07-08 09:00", false);
        assert_eq!(overdue_count(&[past], "done", today), 1);
        assert_eq!(overdue_count(&[future], "done", today), 0);
    }

    #[test]
    fn due_state_ranks_against_today() {
        let today = "2026-07-07";
        assert_eq!(due_state("2026-07-06", today, false), "overdue");
        assert_eq!(due_state("2026-07-07", today, false), "due-today");
        assert_eq!(due_state("2026-07-08", today, false), "");
        assert_eq!(due_state("", today, false), "");
        // A done card is never urgent, however old its date.
        assert_eq!(due_state("2020-01-01", today, true), "");
    }

    #[test]
    fn due_state_compares_only_the_date_prefix_for_timed_dues() {
        // A trap: comparing the full `YYYY-MM-DD HH:MM` string against a plain
        // `YYYY-MM-DD` today would misflag every timed due — a timed due today
        // reads as lexicographically greater ("2026-07-09 14:00" > "2026-07-09"),
        // so the naive comparison would lose `due-today` entirely.
        let today = "2026-07-09";
        assert_eq!(due_state("2026-07-09 14:00", today, false), "due-today");
        assert_eq!(due_state("2026-07-08 23:59", today, false), "overdue");
        assert_eq!(due_state("2026-07-10 00:00", today, false), "");
    }

    #[test]
    fn tag_ok_matches_exact_case_sensitive() {
        let mut t = task("todo", "", false);
        t.tags = vec!["ui".into(), "bug".into()];
        assert!(tag_ok(&t, &None));
        assert!(tag_ok(&t, &Some("ui".into())));
        // Case-sensitive: tags render as typed, no normalisation.
        assert!(!tag_ok(&t, &Some("UI".into())));
        assert!(!tag_ok(&t, &Some("missing".into())));
    }

    #[test]
    fn format_due_is_day_first_and_elides_this_year() {
        assert_eq!(format_due("2026-07-07", 2026), "7 Jul");
        assert_eq!(format_due("2027-01-30", 2026), "30 Jan 2027");
        assert_eq!(format_due("not-a-date", 2026), "not-a-date");
        assert_eq!(format_due("", 2026), "");
    }

    #[test]
    fn format_due_appends_the_time_when_present() {
        assert_eq!(format_due("2026-07-09 14:00", 2026), "9 Jul 14:00");
        assert_eq!(format_due("2027-01-30 09:05", 2026), "30 Jan 2027 09:05");
    }

    #[test]
    fn due_on_day_matches_the_date_prefix() {
        assert!(due_on_day("2026-07-09 09:00", "2026-07-09"));
        assert!(due_on_day("2026-07-09", "2026-07-09"));
        assert!(!due_on_day("2026-07-10 09:00", "2026-07-09"));
        assert!(!due_on_day("2026-07-09", "2026-07-10"));
        assert!(!due_on_day("", "2026-07-09"));
    }

    #[test]
    fn reschedule_preserves_time_of_day() {
        assert_eq!(
            reschedule("2026-07-09 09:00", "2026-07-10"),
            "2026-07-10 09:00"
        );
        assert_eq!(reschedule("2026-07-09", "2026-07-10"), "2026-07-10");
        assert_eq!(reschedule("", "2026-07-10"), "2026-07-10");
    }

    #[test]
    fn sorted_by_due_orders_date_only_and_timed_dues_chronologically() {
        // Lexicographic order on `YYYY-MM-DD` (optionally ` HH:MM`) already
        // equals chronological order — pin it rather than assume it, since
        // `sorted`/`due_key` is the only place that would silently regress.
        let t1 = task("todo", "2026-07-09", false);
        let t2 = task("todo", "2026-07-08 09:00", false);
        let t3 = task("todo", "2026-07-08", false);
        let t4 = task("todo", "2026-07-08 14:00", false);
        let out = sorted(vec![t1, t2, t3, t4], TaskSort::Due);
        let dues: Vec<&str> = out.iter().map(|t| t.due.as_str()).collect();
        assert_eq!(
            dues,
            [
                "2026-07-08",
                "2026-07-08 09:00",
                "2026-07-08 14:00",
                "2026-07-09"
            ]
        );
    }

    #[test]
    fn tag_vocabulary_dedupes_case_insensitively_first_seen_casing_wins() {
        let mut t1 = task("todo", "", false);
        t1.tags = vec!["bug".into(), "ui".into()];
        let mut t2 = task("todo", "", false);
        // "Bug" collides with "bug" case-insensitively — "bug" (seen first) wins.
        t2.tags = vec!["Bug".into(), "docs".into()];
        let mut t3 = task("todo", "", true);
        // Archived task's tags never enter the vocabulary.
        t3.tags = vec!["archived-only".into()];
        let vocab = tag_vocabulary(&[t1, t2, t3]);
        assert_eq!(
            vocab,
            vec!["bug".to_string(), "ui".to_string(), "docs".to_string()]
        );
    }

    #[test]
    fn split_fragment_separates_earlier_tags_from_the_typed_fragment() {
        assert_eq!(
            split_fragment("bug, u"),
            (vec!["bug".to_string()], "u".to_string())
        );
        assert_eq!(split_fragment("b"), (Vec::new(), "b".to_string()));
        assert_eq!(
            split_fragment("bug, ui, "),
            (vec!["bug".to_string(), "ui".to_string()], String::new())
        );
        assert_eq!(split_fragment(""), (Vec::new(), String::new()));
    }

    #[test]
    fn tag_suggestions_ranks_prefix_before_substring_alphabetically_within_band() {
        let vocab = vec!["bug".to_string(), "debug".to_string(), "buggy".to_string()];
        // "bug"/"buggy" prefix-match "bu"; "debug" only contains it mid-string.
        let out = tag_suggestions(&vocab, "bu", &[]);
        assert_eq!(
            out,
            vec!["bug".to_string(), "buggy".to_string(), "debug".to_string()]
        );
    }

    #[test]
    fn tag_suggestions_is_case_insensitive() {
        let vocab = vec!["bug".to_string()];
        // Canonical casing is returned even though the fragment is uppercase.
        assert_eq!(tag_suggestions(&vocab, "B", &[]), vec!["bug".to_string()]);
    }

    #[test]
    fn tag_suggestions_excludes_tags_already_present_in_the_field() {
        let vocab = vec!["bug".to_string(), "build".to_string()];
        let out = tag_suggestions(&vocab, "b", &["bug".to_string()]);
        assert_eq!(out, vec!["build".to_string()]);
    }

    #[test]
    fn tag_suggestions_empty_fragment_yields_nothing() {
        let vocab = vec!["bug".to_string(), "ui".to_string()];
        assert_eq!(tag_suggestions(&vocab, "", &[]), Vec::<String>::new());
    }

    #[test]
    fn accept_suggestion_keeps_earlier_tags_and_trails_a_separator() {
        assert_eq!(accept_suggestion("b", "bug"), "bug, ");
        assert_eq!(accept_suggestion("bug, u", "ui"), "bug, ui, ");
        // No earlier tags at all — still no leading separator.
        assert_eq!(accept_suggestion("", "bug"), "bug, ");
    }
}
