//! The tasks section's calendar view: a Monday-first month grid of task chips
//! (bucketed by `due`'s date prefix — `due_on_day` — so a P2.1 timed due
//! `YYYY-MM-DD HH:MM` still lands in its day cell) and goal target bands (by
//! `target`), with drag-to-reschedule (preserving a timed due's time-of-day —
//! `reschedule`), a per-day popover (the full list plus a quick-add into the
//! first column), an overdue rollup in the header, and an unschedule tray.
//! Same filter pipeline as the board (not archived + toolbar filter + goal
//! scope; `hide done` hides done-column chips); sort and the backlog don't
//! apply here — days are sets, not sequences.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::logic::*;
use crate::dates::{self, MONTHS, WEEKDAYS};
use crate::icon::Icon;
use crate::model::Task;
use crate::state::{MenuTarget, State};

/// How many content rows (goal bands + chips) a day cell shows before folding
/// the rest into a "+N more" row.
const CELL_ROWS: usize = 4;

/// The whole Monday-first weeks covering month `m0` of year `y`, as
/// `(year, month0, day)` triples: from the Monday on/before the 1st to the
/// Sunday on/after the month's last day. The length is always a multiple of 7
/// and every day of the month is present; leading/trailing entries belong to
/// the adjacent months (rendered dimmed but live).
fn grid_days(y: i32, m0: u32) -> Vec<(i32, u32, u32)> {
    // Monday-first column of the 1st — `weekday` has 0 = Sunday, so shift.
    let lead = (dates::weekday(y, m0 + 1, 1) + 6) % 7;
    let n = dates::days_in_month(y, m0);
    let total = (lead + n).div_ceil(7) * 7;
    (0..total)
        .map(|i| dates::add_days(y, m0, 1, i as i32 - lead as i32))
        .collect()
}

/// Whether `task` appears on the calendar at all: not archived, passing the
/// toolbar filter, the goal scope, and the tag scope, and — when `hide_done`
/// — not in the done column (the board's hide-done toggle, applied to chips).
fn cal_visible(
    t: &Task,
    filter: &str,
    goal_filter: &Option<String>,
    tag_filter: &Option<String>,
    hide_done: bool,
    done_col: &str,
) -> bool {
    !t.archived
        && matches(t, filter)
        && goal_ok(t, goal_filter)
        && tag_ok(t, tag_filter)
        && !(hide_done && t.status == done_col)
}

/// A day's chips in display order: priority first (high → none), then title.
fn day_order(mut tasks: Vec<Task>) -> Vec<Task> {
    tasks.sort_by(|a, b| {
        priority_rank(&a.priority)
            .cmp(&priority_rank(&b.priority))
            .then_with(|| a.title.cmp(&b.title))
    });
    tasks
}

/// The month calendar: header (navigation + today + overdue rollup), weekday
/// labels, the day-cell grid, the unschedule tray, and the day popover.
/// Rendered by `TaskBoard` in place of the board when `task_view == Calendar`;
/// the goals bar and the detail drawers stay mounted around it.
#[component]
pub(crate) fn CalendarView() -> impl IntoView {
    let state = expect_context::<State>();
    // Popover state, local to the view (session-only UI).
    let day_open: RwSignal<Option<String>> = RwSignal::new(None);
    let overdue_open = RwSignal::new(false);

    // Esc closes whichever popover is open (the quick-add input's own Escape
    // handler also runs — same target, both idempotent).
    let esc = window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() != "Escape" {
            return;
        }
        if day_open.get_untracked().is_some() {
            day_open.set(None);
        }
        if overdue_open.get_untracked() {
            overdue_open.set(false);
        }
    });
    on_cleanup(move || esc.remove());

    // Close the overdue popover when a click lands outside it (the datepicker's
    // outside-mousedown pattern).
    let od_root: NodeRef<leptos::html::Div> = NodeRef::new();
    let outside = window_event_listener(leptos::ev::mousedown, move |ev| {
        if !overdue_open.get_untracked() {
            return;
        }
        let inside = od_root
            .get_untracked()
            .and_then(|el| {
                ev.target()
                    .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    .map(|n| el.contains(Some(&n)))
            })
            .unwrap_or(false);
        if !inside {
            overdue_open.set(false);
        }
    });
    on_cleanup(move || outside.remove());

    // Overdue = dated strictly before today, not in the done column, and
    // passing the same not-archived + filter + goal-scope pipeline as the
    // cells. Sorted oldest first, then by priority.
    let overdue_tasks = move || {
        let today = dates::today_ymd();
        let f = state.task_filter.get();
        let gf = state.goal_filter.get();
        let tf = state.tag_filter.get();
        let done_col = state.columns.get().last().cloned().unwrap_or_default();
        let mut list: Vec<Task> = state
            .tasks
            .get()
            .into_iter()
            .filter(|t| {
                !t.archived
                    && matches(t, &f)
                    && goal_ok(t, &gf)
                    && tag_ok(t, &tf)
                    && t.status != done_col
                    // Same date-prefix rule as the day cells (P2.1: a timed
                    // `due` must compare by its date part, not the full string).
                    && due_state(&t.due, &today, false) == "overdue"
            })
            .collect();
        list.sort_by(|a, b| {
            a.due
                .cmp(&b.due)
                .then_with(|| priority_rank(&a.priority).cmp(&priority_rank(&b.priority)))
        });
        list
    };

    view! {
        <div class="ido-cal">
            <div class="ido-cal-head">
                <button
                    class="ido-date-navbtn"
                    title="Previous month"
                    on:click=move |_| state.cal_shift_month(-1)
                >
                    <Icon name="chevron-left" size=15 />
                </button>
                <span class="ido-cal-title">
                    {move || {
                        let (y, m0) = state.cal_month.get();
                        format!("{} {y}", MONTHS[m0 as usize])
                    }}
                </span>
                <button
                    class="ido-date-navbtn"
                    title="Next month"
                    on:click=move |_| state.cal_shift_month(1)
                >
                    <Icon name="chevron-right" size=15 />
                </button>
                <button class="ido-cal-todaybtn" on:click=move |_| state.cal_go_today()>
                    "today"
                </button>
                <div class="ido-cal-overdue-wrap" node_ref=od_root>
                    {move || {
                        let n = overdue_tasks().len();
                        (n > 0)
                            .then(|| {
                                view! {
                                    <button
                                        class="ido-cal-overduechip"
                                        class:open=move || overdue_open.get()
                                        on:click=move |_| overdue_open.update(|o| *o = !*o)
                                    >
                                        <Icon name="clock" size=11 />
                                        {format!("overdue {n}")}
                                    </button>
                                }
                            })
                    }}
                    {move || {
                        (overdue_open.get() && !overdue_tasks().is_empty())
                            .then(|| {
                                view! {
                                    <div class="ido-cal-overdue-pop">
                                        {overdue_tasks()
                                            .into_iter()
                                            .map(|t| {
                                                view! {
                                                    // Overdue rows are never done-column tasks
                                                    // (filtered above), so is_done is fixed.
                                                    <PopRow
                                                        task=t
                                                        is_done=false
                                                        close=Callback::new(move |_| overdue_open.set(false))
                                                    />
                                                }
                                            })
                                            .collect_view()}
                                    </div>
                                }
                            })
                    }}
                </div>
            </div>

            <div class="ido-cal-weekdays">
                {WEEKDAYS.iter().map(|w| view! { <span>{*w}</span> }).collect_view()}
            </div>

            <div class="ido-cal-grid">
                {move || {
                    let (y, m0) = state.cal_month.get();
                    let (ty, tm0, td) = dates::today();
                    let today = dates::ymd(ty, tm0, td);
                    let f = state.task_filter.get();
                    let gf = state.goal_filter.get();
                    let tf = state.tag_filter.get();
                    let hide_done = state.hide_done.get();
                    let done_col = state.columns.get().last().cloned().unwrap_or_default();
                    let tasks = state.tasks.get();
                    let goals = state.goals.get();
                    grid_days(y, m0)
                        .into_iter()
                        .map(|(cy, cm0, cd)| {
                            let day = dates::ymd(cy, cm0, cd);
                            let bands: Vec<(String, String, bool)> = goals
                                .iter()
                                .filter(|g| !g.archived && g.target == day)
                                .map(|g| {
                                    let total = tasks
                                        .iter()
                                        .filter(|t| t.goal == g.id && !t.archived)
                                        .count();
                                    let done = tasks
                                        .iter()
                                        .filter(|t| {
                                            t.goal == g.id && t.status == done_col && !t.archived
                                        })
                                        .count();
                                    (
                                        g.id.clone(),
                                        g.title.clone(),
                                        goal_overdue(&g.target, done, total),
                                    )
                                })
                                .collect();
                            let chips: Vec<(Task, &'static str, bool)> = day_order(
                                    tasks
                                        .iter()
                                        .filter(|t| {
                                            due_on_day(&t.due, &day)
                                                && cal_visible(t, &f, &gf, &tf, hide_done, &done_col)
                                        })
                                        .cloned()
                                        .collect(),
                                )
                                .into_iter()
                                .map(|t| {
                                    let is_done = t.status == done_col;
                                    let urgency = due_state(&t.due, &today, is_done);
                                    (t, urgency, is_done)
                                })
                                .collect();
                            let in_month = cy == y && cm0 == m0;
                            let is_today = (cy, cm0, cd) == (ty, tm0, td);
                            // Goal target bands, with progress derived exactly as
                            // the goals bar does (done = the last column).
                            view! {
                                <DayCell
                                    day=day
                                    daynum=cd
                                    in_month=in_month
                                    is_today=is_today
                                    bands=bands
                                    chips=chips
                                    day_open=day_open
                                />
                            }
                        })
                        .collect_view()
                }}
            </div>

            <UnscheduleTray />

            {move || day_open.get().map(|d| view! { <DayPopover day=d day_open=day_open /> })}
        </div>
    }
}

/// One day cell: the day-number button (opens the day popover), goal target
/// bands, capped task chips, and a "+N more" overflow row. A live drop target
/// while a card drag is under way — the highlight is a reactive class keyed on
/// `State::cal_drag_day`, never a list re-render (which would cancel the
/// native drag, same rule as the board's insertion line).
#[component]
fn DayCell(
    day: String,
    daynum: u32,
    in_month: bool,
    is_today: bool,
    bands: Vec<(String, String, bool)>,
    chips: Vec<(Task, &'static str, bool)>,
    day_open: RwSignal<Option<String>>,
) -> impl IntoView {
    let state = expect_context::<State>();
    let slots = CELL_ROWS.saturating_sub(bands.len());
    let (visible, extra) = if chips.len() > slots {
        // The "+N more" row takes the last visible slot.
        let shown = slots.saturating_sub(1);
        (shown, chips.len() - shown)
    } else {
        (chips.len(), 0)
    };
    let day_hl = day.clone();
    let day_over = day.clone();
    let day_drop = day.clone();
    let day_num = day.clone();
    let day_more = day;
    view! {
        <div
            class="ido-cal-cell"
            class:outside=!in_month
            class:today=is_today
            class:drop=move || state.cal_drag_day.get().as_deref() == Some(day_hl.as_str())
            on:dragover=move |ev| {
                if state.task_drag.get_untracked().is_none() {
                    return;
                }
                ev.prevent_default();
                if state.cal_drag_day.get_untracked().as_deref() != Some(day_over.as_str()) {
                    state.cal_drag_day.set(Some(day_over.clone()));
                }
            }
            on:drop=move |ev| {
                ev.prevent_default();
                if let Some(id) = state.task_drag.get_untracked() {
                    let current = state
                        .tasks
                        .get_untracked()
                        .into_iter()
                        .find(|t| t.id == id)
                        .map(|t| t.due)
                        .unwrap_or_default();
                    state.set_task_field(id, "due".into(), reschedule(&current, &day_drop));
                }
                state.clear_task_drag();
            }
        >
            <button class="ido-cal-daynum" on:click=move |_| day_open.set(Some(day_num.clone()))>
                {daynum}
            </button>
            {bands
                .into_iter()
                .map(|(gid, title, overdue)| {
                    view! { <CalGoalBand id=gid title=title overdue=overdue /> }
                })
                .collect_view()}
            {chips
                .into_iter()
                .take(visible)
                .map(|(t, urgency, is_done)| {
                    view! { <CalChip task=t urgency=urgency is_done=is_done /> }
                })
                .collect_view()}
            {(extra > 0)
                .then(|| {
                    view! {
                        <button
                            class="ido-cal-more"
                            on:click=move |_| day_open.set(Some(day_more.clone()))
                        >
                            {format!("+{extra} more")}
                        </button>
                    }
                })}
        </div>
    }
}

/// One task chip on a day cell: priority dot + ellipsized title, accented by
/// `due_state` urgency (muted when done); draggable to another day — or the
/// unschedule tray — to reschedule.
#[component]
fn CalChip(task: Task, urgency: &'static str, is_done: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let Task {
        id,
        title,
        priority,
        repeat,
        ..
    } = task;
    let mut class = String::from("ido-cal-chip");
    if !urgency.is_empty() {
        class.push(' ');
        class.push_str(urgency);
    }
    if is_done {
        class.push_str(" done");
    }
    let id_click = id.clone();
    let id_drag = id.clone();
    let id_dragging = id.clone();
    let id_menu = id;
    let title_attr = title.clone();
    view! {
        <div
            class=class
            title=title_attr
            draggable="true"
            class:dragging=move || state.task_drag.get().as_deref() == Some(id_dragging.as_str())
            on:click=move |_| state.open_task(id_click.clone())
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Task {
                            id: id_menu.clone(),
                        },
                    );
            }
            on:dragstart=move |_| state.task_drag.set(Some(id_drag.clone()))
            on:dragend=move |_| state.clear_task_drag()
        >
            {(!priority.is_empty())
                .then(|| {
                    view! { <span class=format!("ido-cal-pridot ido-pri-{priority}")></span> }
                })}
            {(urgency == "overdue").then(|| view! { <Icon name="clock" size=10 /> })}
            <span class="ido-cal-chip-title">{title}</span>
            {(!repeat.is_empty())
                .then(|| {
                    view! {
                        <span class="ido-task-repeat" title=repeat.clone()>
                            <Icon name="repeat" size=10 />
                        </span>
                    }
                })}
        </div>
    }
}

/// A goal's target band on its target-date cell; overdue goals (target past,
/// not complete) get the accent. Click opens the goal drawer.
#[component]
fn CalGoalBand(id: String, title: String, overdue: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let title_attr = title.clone();
    view! {
        <div
            class="ido-cal-goal"
            class:overdue=overdue
            title=title_attr
            on:click=move |_| state.open_goal(id.clone())
        >
            <Icon name="target" size=10 />
            <span class="ido-cal-goal-title">{title}</span>
        </div>
    }
}

/// One compact popover row (day + overdue popovers): priority dot, title, and
/// the formatted due date with its urgency accent. Click opens the task's
/// drawer, closing the popover through `close`.
#[component]
fn PopRow(task: Task, is_done: bool, close: Callback<()>) -> impl IntoView {
    let state = expect_context::<State>();
    let Task {
        id,
        title,
        priority,
        due,
        repeat,
        ..
    } = task;
    let urgency = due_state(&due, &dates::today_ymd(), is_done);
    let due_class = if urgency.is_empty() {
        "ido-task-due".to_string()
    } else {
        format!("ido-task-due {urgency}")
    };
    let id_click = id;
    view! {
        <div
            class="ido-cal-row"
            on:click=move |_| {
                close.run(());
                state.open_task(id_click.clone());
            }
        >
            {(!priority.is_empty())
                .then(|| {
                    view! { <span class=format!("ido-cal-pridot ido-pri-{priority}")></span> }
                })}
            <span class="ido-cal-row-title" class:done=is_done>
                {title}
            </span>
            {(!repeat.is_empty())
                .then(|| {
                    view! {
                        <span class="ido-task-repeat" title=repeat.clone()>
                            <Icon name="repeat" size=10 />
                        </span>
                    }
                })}
            {(!due.is_empty())
                .then(|| {
                    view! {
                        <span class=due_class>
                            {(urgency == "overdue")
                                .then(|| view! { <Icon name="clock" size=10 /> })}
                            {format_due(&due, dates::this_year())}
                        </span>
                    }
                })}
        </div>
    }
}

/// The day popover: a centered panel over the calendar (backdrop click / Esc
/// dismisses) listing every chip for `day` uncapped, plus a quick-add input
/// that creates tasks due that day in the **first** column — Enter adds and
/// keeps capturing, Esc discards and closes, blur commits typed text (the
/// board `QuickAdd` semantics).
#[component]
fn DayPopover(day: String, day_open: RwSignal<Option<String>>) -> impl IntoView {
    let state = expect_context::<State>();
    let title = dates::parse_ymd(&day)
        .map(|(y, m0, d)| format!("{d} {} {y}", MONTHS[m0 as usize]))
        .unwrap_or_else(|| day.clone());

    // Focus the quick-add when the popover mounts (the ref turns `Some`).
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    Effect::new(move |_| {
        if let Some(el) = input_ref.get() {
            let _ = el.focus();
        }
    });

    // Create a task from the input's text (if any) and clear it for the next.
    let day_add = day.clone();
    let submit = Callback::new(move |_: ()| {
        let Some(el) = input_ref.get_untracked() else {
            return;
        };
        let text = el.value();
        el.set_value("");
        if !text.trim().is_empty() {
            state.add_task_quick_due(text, day_add.clone());
        }
    });

    let day_list = day.clone();
    view! {
        <div class="ido-cal-backdrop" on:mousedown=move |_| day_open.set(None)>
            <div class="ido-cal-pop" on:mousedown=move |ev| ev.stop_propagation()>
                <div class="ido-cal-pop-head">{title}</div>
                <div class="ido-cal-pop-list">
                    {move || {
                        let f = state.task_filter.get();
                        let gf = state.goal_filter.get();
                        let tf = state.tag_filter.get();
                        let hide_done = state.hide_done.get();
                        let done_col = state.columns.get().last().cloned().unwrap_or_default();
                        let rows = day_order(
                            state
                                .tasks
                                .get()
                                .into_iter()
                                .filter(|t| {
                                    due_on_day(&t.due, &day_list)
                                        && cal_visible(t, &f, &gf, &tf, hide_done, &done_col)
                                })
                                .collect(),
                        );
                        if rows.is_empty() {
                            view! { <div class="ido-cal-pop-empty">"nothing due this day"</div> }
                                .into_any()
                        } else {
                            rows.into_iter()
                                .map(|t| {
                                    let is_done = t.status == done_col;
                                    view! {
                                        <PopRow
                                            task=t
                                            is_done=is_done
                                            close=Callback::new(move |_| day_open.set(None))
                                        />
                                    }
                                })
                                .collect_view()
                                .into_any()
                        }
                    }}
                </div>
                <input
                    class="ido-column-quickadd"
                    node_ref=input_ref
                    placeholder="task title…"
                    spellcheck="false"
                    autocomplete="off"
                    on:keydown=move |ev| {
                        match ev.key().as_str() {
                            "Enter" => submit.run(()),
                            "Escape" => {
                                ev.stop_propagation();
                                if let Some(el) = input_ref.get_untracked() {
                                    el.set_value("");
                                }
                                day_open.set(None);
                            }
                            _ => {}
                        }
                    }
                    on:blur=move |_| submit.run(())
                />
            </div>
        </div>
    }
}

/// The unschedule tray: a strip over the grid's bottom edge, visible only
/// while a card drag is live; dropping a chip here clears its due date. Always
/// mounted and class-toggled (never conditionally rendered), so appearing
/// mid-drag can't re-render or reflow the drag's drop targets. Its hover
/// highlight reuses `State::cal_drag_day` with `""` as the sentinel, mirroring
/// the board's `task_drag_over` backlog convention.
#[component]
fn UnscheduleTray() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div
            class="ido-cal-tray"
            class:live=move || state.task_drag.get().is_some()
            class:drop=move || state.cal_drag_day.get().as_deref() == Some("")
            on:dragover=move |ev| {
                if state.task_drag.get_untracked().is_none() {
                    return;
                }
                ev.prevent_default();
                if state.cal_drag_day.get_untracked().as_deref() != Some("") {
                    state.cal_drag_day.set(Some(String::new()));
                }
            }
            on:drop=move |ev| {
                ev.prevent_default();
                if let Some(id) = state.task_drag.get_untracked() {
                    state.set_task_field(id, "due".into(), String::new());
                }
                state.clear_task_drag();
            }
        >
            <Icon name="x" size=12 />
            "drop to clear the due date"
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal task for the bucketing tests.
    fn task(title: &str, priority: &str, status: &str, due: &str, archived: bool) -> Task {
        Task {
            id: title.to_lowercase().replace(' ', "-"),
            title: title.into(),
            status: status.into(),
            priority: priority.into(),
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
    fn grid_days_month_starting_monday_has_no_lead() {
        // 1 July 2024 is a Monday: the grid starts on the 1st itself.
        let g = grid_days(2024, 6);
        assert_eq!(g.first(), Some(&(2024, 6, 1)));
        assert_eq!(g.len(), 35);
        assert_eq!(g.last(), Some(&(2024, 7, 4))); // trails to Sunday 4 Aug
    }

    #[test]
    fn grid_days_month_starting_sunday_has_max_lead() {
        // 1 March 2026 is a Sunday: six leading February days, six weeks.
        let g = grid_days(2026, 2);
        assert_eq!(g.first(), Some(&(2026, 1, 23))); // Monday 23 Feb
        assert_eq!(g[6], (2026, 2, 1)); // the 1st sits in the Sunday column
        assert_eq!(g.len(), 42);
        assert_eq!(g.last(), Some(&(2026, 3, 5))); // Sunday 5 Apr
    }

    #[test]
    fn grid_days_covers_leap_february() {
        // 1 Feb 2024 is a Thursday; 2024 is a leap year.
        let g = grid_days(2024, 1);
        assert_eq!(g.first(), Some(&(2024, 0, 29))); // Monday 29 Jan
        assert!(g.contains(&(2024, 1, 29)));
        assert_eq!(g.len(), 35);
    }

    #[test]
    fn grid_days_invariants_hold_across_months() {
        for y in 2023..=2027 {
            for m0 in 0..12 {
                let g = grid_days(y, m0);
                // Whole weeks — four at fewest, six at most.
                assert_eq!(g.len() % 7, 0, "{y}-{m0}");
                assert!((28..=42).contains(&g.len()), "{y}-{m0}");
                // The first cell is always a Monday.
                let (fy, fm0, fd) = g[0];
                assert_eq!(dates::weekday(fy, fm0 + 1, fd), 1, "{y}-{m0}");
                // Every day of the month is present.
                for d in 1..=dates::days_in_month(y, m0) {
                    assert!(g.contains(&(y, m0, d)), "{y}-{m0}-{d}");
                }
            }
        }
    }

    #[test]
    fn day_order_ranks_priority_then_title() {
        let got = day_order(vec![
            task("b none", "", "todo", "2026-07-09", false),
            task("z high", "high", "todo", "2026-07-09", false),
            task("m low", "low", "todo", "2026-07-09", false),
            task("a normal", "normal", "todo", "2026-07-09", false),
            task("a high", "high", "todo", "2026-07-09", false),
        ]);
        let titles: Vec<&str> = got.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(titles, ["a high", "z high", "a normal", "m low", "b none"]);
    }

    #[test]
    fn cal_visible_mirrors_the_board_pipeline() {
        // hide_done drops done-column tasks; otherwise they show (muted).
        let done = task("ship the calendar", "high", "done", "2026-07-09", false);
        assert!(cal_visible(&done, "", &None, &None, false, "done"));
        assert!(!cal_visible(&done, "", &None, &None, true, "done"));
        // Archived tasks never show.
        let gone = task("old thing", "", "todo", "2026-07-09", true);
        assert!(!cal_visible(&gone, "", &None, &None, false, "done"));
        // The toolbar text filter and the goal scope both apply.
        let plain = task("write docs", "", "todo", "2026-07-09", false);
        assert!(cal_visible(&plain, "docs", &None, &None, false, "done"));
        assert!(!cal_visible(
            &plain, "calendar", &None, &None, false, "done"
        ));
        assert!(!cal_visible(
            &plain,
            "",
            &Some("v1".into()),
            &None,
            false,
            "done"
        ));
        // The tag scope applies too (exact, case-sensitive).
        let mut tagged = task("fix layout", "", "todo", "2026-07-09", false);
        tagged.tags = vec!["ui".into()];
        assert!(cal_visible(
            &tagged,
            "",
            &None,
            &Some("ui".into()),
            false,
            "done"
        ));
        assert!(!cal_visible(
            &tagged,
            "",
            &None,
            &Some("UI".into()),
            false,
            "done"
        ));
        assert!(!cal_visible(
            &tagged,
            "",
            &None,
            &Some("bug".into()),
            false,
            "done"
        ));
    }
}
