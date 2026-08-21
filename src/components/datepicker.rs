//! A custom date picker — a styled trigger plus a calendar popover — replacing
//! the native `<input type="date">`, whose popup can't be themed past its
//! background. Follows the latent. design system; emits `YYYY-MM-DD` (or `""`
//! when cleared) through `on_change` — or, since P2.1, `YYYY-MM-DD HH:MM` when
//! a time is set via the plain time input under the grid. The incoming
//! `value` is split into its date/time parts on mount ([`split_due`]); picking
//! a day recombines it with whatever time is currently set
//! ([`combine_due`]) — so a timed due stays timed across a re-pick, and the
//! calendar itself always seeds from the date part (a raw `parse_ymd` on the
//! full timed value would fail and leave the picker unseeded).

use latent_ui::Icon;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::dates::{MONTHS, MONTHS_SHORT, WEEKDAYS, days_in_month, parse_ymd, today, weekday, ymd};

/// Split a `due` value into its `(date, time)` parts on the first space —
/// `time` is `""` when `due` is date-only (or empty).
fn split_due(due: &str) -> (String, String) {
    match due.split_once(' ') {
        Some((d, t)) => (d.to_string(), t.to_string()),
        None => (due.to_string(), String::new()),
    }
}

/// Recombine a `date` (`YYYY-MM-DD`, or `""`) with a `time` (`HH:MM`, or
/// `""`) into a `due` value. No date means no due at all, regardless of any
/// typed time; no time means a plain date-only due.
fn combine_due(date: &str, time: &str) -> String {
    if date.is_empty() || time.is_empty() {
        date.to_string()
    } else {
        format!("{date} {time}")
    }
}

/// Loosely validate + normalize a typed time to zero-padded `HH:MM` (24 h):
/// accepts `H:MM` or `HH:MM` (hour `0..=23`, a two-digit minute `00..=59`).
/// Anything else — garbage, an empty string, an out-of-range value — becomes
/// `""` (the picker then falls back to a date-only due, never a malformed one).
fn normalize_time(input: &str) -> String {
    let s = input.trim();
    let Some((h, m)) = s.split_once(':') else {
        return String::new();
    };
    if h.is_empty() || h.len() > 2 || m.len() != 2 {
        return String::new();
    }
    if !h.bytes().all(|b| b.is_ascii_digit()) || !m.bytes().all(|b| b.is_ascii_digit()) {
        return String::new();
    }
    let (Ok(hh), Ok(mm)) = (h.parse::<u32>(), m.parse::<u32>()) else {
        return String::new();
    };
    if hh > 23 || mm > 59 {
        return String::new();
    }
    format!("{hh:02}:{mm:02}")
}

/// A date field: a trigger showing the chosen date (or a placeholder) and a
/// calendar popover with a plain time input under the grid. `value` is the
/// current `YYYY-MM-DD` (optionally ` HH:MM`, P2.1), or `""`.
#[component]
pub fn DatePicker(value: String, on_change: Callback<String>) -> impl IntoView {
    let (ty, tm, td) = today();
    let (date0, time0) = split_due(&value);
    let (vy, vm) = parse_ymd(&date0)
        .map(|(y, m, _)| (y, m))
        .unwrap_or((ty, tm));

    // `selected` holds just the date part; `time` the optional `HH:MM` — kept
    // apart so the calendar grid's own `sel`/seed logic never has to parse
    // through a time suffix.
    let selected = RwSignal::new(date0);
    let time = RwSignal::new(time0);
    let view_year = RwSignal::new(vy);
    let view_month = RwSignal::new(vm);
    let open = RwSignal::new(false);
    let root: NodeRef<leptos::html::Div> = NodeRef::new();

    // Close when a click lands outside the picker.
    let handle = window_event_listener(leptos::ev::mousedown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root
            .get_untracked()
            .and_then(|el| {
                ev.target()
                    .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    .map(|n| el.contains(Some(&n)))
            })
            .unwrap_or(false);
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());

    // Esc closes the popover, not whatever's behind it (e.g. a task/goal
    // drawer). The workspace's global Escape handler detects this popover by
    // DOM presence (`.ido-date-pop`) and yields to it — it registers before
    // this component ever mounts, so it always runs first and sees the
    // popover still open, then this handler closes it.
    let esc = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() == "Escape" && open.get_untracked() {
            open.set(false);
        }
    });
    on_cleanup(move || esc.remove());

    // Pick a day: recombines it with whatever time is currently set, so a
    // timed due stays timed across a re-pick (`combine_due`).
    let pick = move |ymd: String| {
        selected.set(ymd.clone());
        on_change.run(combine_due(&ymd, &time.get_untracked()));
        open.set(false);
    };

    // Shift the viewed month by `delta`, wrapping the year.
    let go = move |delta: i32| {
        let mut m = view_month.get_untracked() as i32 + delta;
        let mut y = view_year.get_untracked();
        while m < 0 {
            m += 12;
            y -= 1;
        }
        while m > 11 {
            m -= 12;
            y += 1;
        }
        view_year.set(y);
        view_month.set(m as u32);
    };

    view! {
        <div class="ido-datepicker" node_ref=root>
            <button
                type="button"
                class="ido-drawer-input ido-date-trigger"
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {move || match parse_ymd(&selected.get()) {
                    Some((y, m, d)) => {
                        let t = time.get();
                        let label = if t.is_empty() {
                            format!("{d} {} {y}", MONTHS_SHORT[m as usize])
                        } else {
                            format!("{d} {} {y} {t}", MONTHS_SHORT[m as usize])
                        };
                        view! { <span class="ido-date-value">{label}</span> }.into_any()
                    }
                    None => {
                        view! { <span class="ido-date-placeholder">"set date"</span> }.into_any()
                    }
                }}
                <Icon name="calendar" size=13 />
            </button>

            {move || {
                open.get()
                    .then(|| {
                        view! {
                            <div class="ido-date-pop" on:mousedown=move |ev| ev.stop_propagation()>
                                <div class="ido-date-nav">
                                    <button
                                        type="button"
                                        class="ido-date-navbtn"
                                        on:click=move |_| go(-1)
                                    >
                                        <Icon name="chevron-left" size=15 />
                                    </button>
                                    <span class="ido-date-title">
                                        {move || {
                                            format!(
                                                "{} {}",
                                                MONTHS[view_month.get() as usize],
                                                view_year.get(),
                                            )
                                        }}
                                    </span>
                                    <button
                                        type="button"
                                        class="ido-date-navbtn"
                                        on:click=move |_| go(1)
                                    >
                                        <Icon name="chevron-right" size=15 />
                                    </button>
                                </div>
                                <div class="ido-date-grid">
                                    {WEEKDAYS
                                        .iter()
                                        .map(|w| view! { <span class="ido-date-wd">{*w}</span> })
                                        .collect_view()}
                                    {move || {
                                        let y = view_year.get();
                                        let m = view_month.get();
                                        let lead = (weekday(y, m + 1, 1) + 6) % 7;
                                        let n = days_in_month(y, m);
                                        let sel = selected.get();
                                        let mut cells = Vec::new();
                                        for _ in 0..lead {
                                            cells
                                                .push(
                                                    // Monday-first: shift Sunday (0) to the last column.
                                                    view! { <span class="ido-date-cell ido-date-empty"></span> }
                                                        .into_any(),
                                                );
                                        }
                                        for d in 1..=n {
                                            let date = ymd(y, m, d);
                                            let is_sel = sel == date;
                                            let is_today = (y, m, d) == (ty, tm, td);
                                            cells
                                                .push(
                                                    view! {
                                                        <button
                                                            type="button"
                                                            class="ido-date-cell"
                                                            class:sel=is_sel
                                                            class:today=is_today
                                                            on:click=move |_| pick(date.clone())
                                                        >
                                                            {d}
                                                        </button>
                                                    }
                                                        .into_any(),
                                                );
                                        }
                                        cells.into_iter().collect_view()
                                    }}
                                </div>
                                <input
                                    type="text"
                                    class="ido-date-time"
                                    placeholder="HH:MM"
                                    spellcheck="false"
                                    autocomplete="off"
                                    prop:value=move || time.get()
                                    on:change=move |ev| {
                                        let norm = normalize_time(&event_target_value(&ev));
                                        time.set(norm.clone());
                                        let day = selected.get_untracked();
                                        if !day.is_empty() {
                                            on_change.run(combine_due(&day, &norm));
                                        }
                                    }
                                />
                                <div class="ido-date-foot">
                                    <button
                                        type="button"
                                        class="ido-date-action"
                                        on:click=move |_| {
                                            selected.set(String::new());
                                            time.set(String::new());
                                            on_change.run(String::new());
                                            open.set(false);
                                        }
                                    >
                                        "clear"
                                    </button>
                                    <button
                                        type="button"
                                        class="ido-date-action"
                                        on:click=move |_| {
                                            view_year.set(ty);
                                            view_month.set(tm);
                                            pick(ymd(ty, tm, td));
                                        }
                                    >
                                        "today"
                                    </button>
                                </div>
                            </div>
                        }
                    })
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_due_separates_date_and_time() {
        assert_eq!(
            split_due("2026-07-09 14:00"),
            ("2026-07-09".to_string(), "14:00".to_string())
        );
        assert_eq!(
            split_due("2026-07-09"),
            ("2026-07-09".to_string(), String::new())
        );
        assert_eq!(split_due(""), (String::new(), String::new()));
    }

    #[test]
    fn combine_due_appends_time_or_stays_date_only() {
        assert_eq!(combine_due("2026-07-09", "14:00"), "2026-07-09 14:00");
        assert_eq!(combine_due("2026-07-09", ""), "2026-07-09");
        // No date means no due, whatever the time.
        assert_eq!(combine_due("", "14:00"), "");
        assert_eq!(combine_due("", ""), "");
    }

    #[test]
    fn normalize_time_accepts_loose_h_mm_and_hh_mm() {
        assert_eq!(normalize_time("9:05"), "09:05");
        assert_eq!(normalize_time("14:30"), "14:30");
        assert_eq!(normalize_time(" 09:00 "), "09:00");
        assert_eq!(normalize_time("0:00"), "00:00");
        assert_eq!(normalize_time("23:59"), "23:59");
    }

    #[test]
    fn normalize_time_rejects_garbage_as_date_only() {
        assert_eq!(normalize_time(""), "");
        assert_eq!(normalize_time("garbage"), "");
        assert_eq!(normalize_time("25:00"), "");
        assert_eq!(normalize_time("14:60"), "");
        assert_eq!(normalize_time("14:5"), "");
        assert_eq!(normalize_time("14:600"), "");
        assert_eq!(normalize_time("abc:de"), "");
    }
}
