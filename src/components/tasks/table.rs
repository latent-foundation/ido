//! The tasks section's table view: one flat, sortable table of every task
//! (backlog rows included), for review — "everything at once", which the board
//! and calendar are bad at. Same filter pipeline as the board (not archived +
//! toolbar text filter + tag scope + goal scope; `hide done` drops done-column
//! rows), but no sort toolbar and no backlog panel — the table sorts itself and
//! already lists un-columned tasks.
//!
//! Sort state is local to the view (a `(TableCol, bool)` signal, `bool` =
//! ascending), independent of the board's [`TaskSort`](crate::state::TaskSort):
//! clicking a header sorts by that column (toggling asc/desc on the active one,
//! defaulting a freshly-picked one to ascending). Every column is sortable; the
//! comparators are the pure functions [`sort_rows`] / [`status_rank`] /
//! [`cmp_present`] below (host-tested). Rows open the detail drawer on click /
//! Enter / Space and carry the shared task context menu.

use std::cmp::Ordering;

use leptos::prelude::*;

use super::logic::*;
use crate::dates::{this_year, today_ymd};
use crate::icon::Icon;
use crate::model::Task;
use crate::state::{MenuTarget, State};

/// A sortable table column. Local to this view (not the board's `TaskSort`).
#[derive(Clone, Copy, PartialEq, Debug)]
enum TableCol {
    Title,
    Status,
    Priority,
    Tags,
    Due,
    Goal,
    Completed,
}

impl TableCol {
    /// Every column, in display (left-to-right) order.
    const ALL: [TableCol; 7] = [
        TableCol::Title,
        TableCol::Status,
        TableCol::Priority,
        TableCol::Tags,
        TableCol::Due,
        TableCol::Goal,
        TableCol::Completed,
    ];

    /// The header label (lowercase; CSS upcases it, matching the board's heads).
    fn label(self) -> &'static str {
        match self {
            TableCol::Title => "title",
            TableCol::Status => "status",
            TableCol::Priority => "priority",
            TableCol::Tags => "tags",
            TableCol::Due => "due",
            TableCol::Goal => "goal",
            TableCol::Completed => "completed",
        }
    }
}

/// One table row: a task plus its resolved goal *title* (the goal column shows
/// the title, not the slug — so it must be resolved before sorting).
#[derive(Clone)]
struct Row {
    task: Task,
    goal_title: String,
}

/// Status-column sort key: a real board column sorts by its position; the
/// backlog (empty status) sorts after every column; an orphan status
/// (non-empty but not a column) sorts after the backlog, then alphabetically —
/// i.e. "columns in order, backlog last, orphans after that".
fn status_rank(status: &str, columns: &[String]) -> (usize, String) {
    if let Some(i) = columns.iter().position(|c| c == status) {
        (i, String::new())
    } else if status.is_empty() {
        (columns.len(), String::new())
    } else {
        (columns.len() + 1, status.to_lowercase())
    }
}

/// Compare two rank keys, reversing for descending.
fn cmp_by<K: Ord>(a: K, b: K, asc: bool) -> Ordering {
    if asc { a.cmp(&b) } else { b.cmp(&a) }
}

/// Compare two text values keeping *absent* (empty) entries last in **both**
/// directions — `asc` reverses only the ordering among present values, so an
/// unset due / completed / goal / tag never floats to the top on a desc sort.
/// Present values compare case-insensitively (correct for text; a harmless
/// no-op for the digit-only date columns, where lexicographic order already
/// equals chronological — same basis as [`due_key`]).
fn cmp_present(a: &str, b: &str, asc: bool) -> Ordering {
    match (a.is_empty(), b.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => {
            let (a, b) = (a.to_lowercase(), b.to_lowercase());
            if asc { a.cmp(&b) } else { b.cmp(&a) }
        }
    }
}

/// Sort `rows` by `col` in `asc`/desc order, with a stable case-insensitive
/// title tiebreak so equal keys stay deterministic. Pure — host-tested below.
fn sort_rows(mut rows: Vec<Row>, sort: (TableCol, bool), columns: &[String]) -> Vec<Row> {
    let (col, asc) = sort;
    rows.sort_by(|a, b| {
        let primary = match col {
            TableCol::Title => cmp_by(
                a.task.title.to_lowercase(),
                b.task.title.to_lowercase(),
                asc,
            ),
            TableCol::Status => cmp_by(
                status_rank(&a.task.status, columns),
                status_rank(&b.task.status, columns),
                asc,
            ),
            TableCol::Priority => cmp_by(
                priority_rank(&a.task.priority),
                priority_rank(&b.task.priority),
                asc,
            ),
            TableCol::Tags => cmp_present(&a.task.tags.join(", "), &b.task.tags.join(", "), asc),
            TableCol::Due => cmp_present(&a.task.due, &b.task.due, asc),
            TableCol::Goal => cmp_present(&a.goal_title, &b.goal_title, asc),
            TableCol::Completed => cmp_present(&a.task.completed, &b.task.completed, asc),
        };
        primary.then_with(|| {
            a.task
                .title
                .to_lowercase()
                .cmp(&b.task.title.to_lowercase())
        })
    });
    rows
}

/// The table view: a scrollable, sticky-headed table of every non-archived task
/// passing the board's filter pipeline (backlog rows included), sorted by the
/// clicked header. Rendered by `TaskBoard` in place of the board when
/// `task_view == Table`; the goals bar and detail drawers stay mounted around it.
#[component]
pub(crate) fn TableView() -> impl IntoView {
    let state = expect_context::<State>();
    // Sort state, local to the view (not persisted). Default: due ascending.
    let sort: RwSignal<(TableCol, bool)> = RwSignal::new((TableCol::Due, true));

    view! {
        <div class="ido-table-wrap">
            <table class="ido-table">
                <thead>
                    <tr>
                        {TableCol::ALL
                            .into_iter()
                            .map(|c| {
                                view! {
                                    <th
                                        class="ido-th"
                                        class:sorted=move || sort.get().0 == c
                                        on:click=move |_| {
                                            sort.update(|(cur, asc)| {
                                                if *cur == c {
                                                    *asc = !*asc;
                                                } else {
                                                    *cur = c;
                                                    *asc = true;
                                                }
                                            })
                                        }
                                    >
                                        <span class="ido-th-inner">
                                            {c.label()}
                                            {move || {
                                                let (cur, asc) = sort.get();
                                                (cur == c)
                                                    .then(|| {
                                                        let name = if asc { "chevron-up" } else { "chevron-down" };
                                                        view! { <Icon name=name size=12 /> }
                                                    })
                                            }}
                                        </span>
                                    </th>
                                }
                            })
                            .collect_view()}
                    </tr>
                </thead>
                <tbody>
                    {move || {
                        let cols = state.columns.get();
                        let done_col = cols.last().cloned().unwrap_or_default();
                        let f = state.task_filter.get();
                        let gf = state.goal_filter.get();
                        let tf = state.tag_filter.get();
                        let hide_done = state.hide_done.get();
                        let goals = state.goals.get();
                        let rows: Vec<Row> = state
                            .tasks
                            .get()
                            .into_iter()
                            .filter(|t| {
                                !t.archived && matches(t, &f) && tag_ok(t, &tf) && goal_ok(t, &gf)
                                    && !(hide_done && t.status == done_col)
                            })
                            .map(|t| {
                                let goal_title = goals
                                    .iter()
                                    .find(|g| g.id == t.goal)
                                    .map(|g| g.title.clone())
                                    .unwrap_or_default();
                                Row { task: t, goal_title }
                            })
                            .collect();
                        let rows = sort_rows(rows, sort.get(), &cols);
                        if rows.is_empty() {
                            return view! {
                                <tr>
                                    <td class="ido-table-empty" colspan="7">
                                        "no tasks"
                                    </td>
                                </tr>
                            }
                                .into_any();
                        }
                        rows.into_iter()
                            .map(|r| {
                                let is_done = !done_col.is_empty() && r.task.status == done_col;
                                view! {
                                    <TableRow task=r.task goal_title=r.goal_title is_done=is_done />
                                }
                            })
                            .collect_view()
                            .into_any()
                    }}
                </tbody>
            </table>
        </div>
    }
}

/// One table row. Click / Enter / Space open the task's drawer; right-click
/// opens the shared task context menu; tags are click-to-filter (like the
/// board's, `stop_propagation` so the row click doesn't also fire).
#[component]
fn TableRow(task: Task, goal_title: String, is_done: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let Task {
        id,
        title,
        status,
        priority,
        tags,
        due,
        completed,
        ..
    } = task;
    let id_click = id.clone();
    let id_menu = id.clone();
    let id_kbd = id.clone();
    let id_active = id;
    let pri_class = priority.clone();
    let year = this_year();
    let status_label = if status.is_empty() {
        "(backlog)".to_string()
    } else {
        col_label(&status)
    };
    let is_backlog = status.is_empty();
    let due_urgency = due_state(&due, &today_ymd(), is_done);
    let due_class = if due_urgency.is_empty() {
        "ido-task-due".to_string()
    } else {
        format!("ido-task-due {due_urgency}")
    };
    view! {
        <tr
            class="ido-table-row"
            tabindex="0"
            class:active=move || state.active_task.get().as_deref() == Some(id_active.as_str())
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
            on:keydown=move |ev| {
                match ev.key().as_str() {
                    "Enter" => state.open_task(id_kbd.clone()),
                    " " => {
                        ev.prevent_default();
                        state.open_task(id_kbd.clone());
                    }
                    _ => {}
                }
            }
        >
            <td class="ido-td-title">{title}</td>
            <td>
                <span class="ido-table-status" class:backlog=is_backlog>
                    {status_label}
                </span>
            </td>
            <td>
                {(!priority.is_empty())
                    .then(|| {
                        view! {
                            <span class=format!(
                                "ido-task-pri ido-pri-{pri_class}",
                            )>{priority}</span>
                        }
                    })}
            </td>
            <td>
                <div class="ido-td-tags">
                    {tags
                        .into_iter()
                        .map(|t| {
                            let t_click = t.clone();
                            let t_active = t.clone();
                            view! {
                                <span
                                    class="ido-task-tag"
                                    class:active=move || {
                                        state.tag_filter.get().as_deref() == Some(t_active.as_str())
                                    }
                                    on:click=move |ev| {
                                        ev.stop_propagation();
                                        state.toggle_tag_filter(t_click.clone());
                                    }
                                >
                                    {t}
                                </span>
                            }
                        })
                        .collect_view()}
                </div>
            </td>
            <td>
                {(!due.is_empty())
                    .then(move || {
                        view! {
                            <span class=due_class>
                                {(due_urgency == "overdue")
                                    .then(|| view! { <Icon name="clock" size=10 /> })}
                                {format_due(&due, year)}
                            </span>
                        }
                    })}
            </td>
            <td class="ido-td-goal">
                {if goal_title.is_empty() { "—".to_string() } else { goal_title }}
            </td>
            <td class="ido-td-completed">{format_due(&completed, year)}</td>
        </tr>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A row for the sort tests.
    fn row(
        title: &str,
        status: &str,
        priority: &str,
        tags: &[&str],
        due: &str,
        goal_title: &str,
        completed: &str,
    ) -> Row {
        Row {
            task: Task {
                id: title.to_lowercase().replace(' ', "-"),
                title: title.into(),
                status: status.into(),
                priority: priority.into(),
                tags: tags.iter().map(|s| s.to_string()).collect(),
                order: 0,
                due: due.into(),
                goal: String::new(),
                repeat: String::new(),
                archived: false,
                completed: completed.into(),
                checks_done: 0,
                checks_total: 0,
                body: String::new(),
            },
            goal_title: goal_title.into(),
        }
    }

    fn titles(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.task.title.clone()).collect()
    }

    fn cols() -> Vec<String> {
        vec!["todo".into(), "doing".into(), "done".into()]
    }

    #[test]
    fn title_sorts_case_insensitively_both_directions() {
        let rows = vec![
            row("banana", "", "", &[], "", "", ""),
            row("Apple", "", "", &[], "", "", ""),
            row("cherry", "", "", &[], "", "", ""),
        ];
        let asc = sort_rows(rows.clone(), (TableCol::Title, true), &cols());
        assert_eq!(titles(&asc), ["Apple", "banana", "cherry"]);
        let desc = sort_rows(rows, (TableCol::Title, false), &cols());
        assert_eq!(titles(&desc), ["cherry", "banana", "Apple"]);
    }

    #[test]
    fn status_orders_by_column_position_then_backlog_then_orphan() {
        let rows = vec![
            row("d", "done", "", &[], "", "", ""),
            row("orphan", "weird", "", &[], "", "", ""),
            row("b", "", "", &[], "", "", ""),
            row("a", "todo", "", &[], "", "", ""),
            row("c", "doing", "", &[], "", "", ""),
        ];
        let asc = sort_rows(rows, (TableCol::Status, true), &cols());
        // columns in order (todo, doing, done), then backlog (empty), then orphan.
        assert_eq!(titles(&asc), ["a", "c", "d", "b", "orphan"]);
    }

    #[test]
    fn priority_sorts_by_rank_both_directions() {
        let rows = vec![
            row("none-pri", "", "", &[], "", "", ""),
            row("high-pri", "", "high", &[], "", "", ""),
            row("low-pri", "", "low", &[], "", "", ""),
            row("normal-pri", "", "normal", &[], "", "", ""),
        ];
        let asc = sort_rows(rows.clone(), (TableCol::Priority, true), &cols());
        assert_eq!(
            titles(&asc),
            ["high-pri", "normal-pri", "low-pri", "none-pri"]
        );
        let desc = sort_rows(rows, (TableCol::Priority, false), &cols());
        assert_eq!(
            titles(&desc),
            ["none-pri", "low-pri", "normal-pri", "high-pri"]
        );
    }

    #[test]
    fn due_groups_empties_last_in_both_directions() {
        let rows = vec![
            row("has-late", "", "", &[], "2026-07-10", "", ""),
            row("no-due", "", "", &[], "", "", ""),
            row("has-early", "", "", &[], "2026-07-01", "", ""),
        ];
        let asc = sort_rows(rows.clone(), (TableCol::Due, true), &cols());
        assert_eq!(titles(&asc), ["has-early", "has-late", "no-due"]);
        // desc reverses the dated rows but keeps the empty one last.
        let desc = sort_rows(rows, (TableCol::Due, false), &cols());
        assert_eq!(titles(&desc), ["has-late", "has-early", "no-due"]);
    }

    #[test]
    fn completed_sorts_like_due_empties_last() {
        let rows = vec![
            row("finished-early", "", "", &[], "", "", "2026-07-01"),
            row("open", "", "", &[], "", "", ""),
            row("finished-late", "", "", &[], "", "", "2026-07-09"),
        ];
        // "what did I finish this week": desc → most recent first, open last.
        let desc = sort_rows(rows.clone(), (TableCol::Completed, false), &cols());
        assert_eq!(titles(&desc), ["finished-late", "finished-early", "open"]);
        let asc = sort_rows(rows, (TableCol::Completed, true), &cols());
        assert_eq!(titles(&asc), ["finished-early", "finished-late", "open"]);
    }

    #[test]
    fn tags_and_goal_group_empties_last() {
        let tag_rows = vec![
            row("t-b", "", "", &["ui"], "", "", ""),
            row("t-none", "", "", &[], "", "", ""),
            row("t-a", "", "", &["api"], "", "", ""),
        ];
        let asc = sort_rows(tag_rows, (TableCol::Tags, true), &cols());
        assert_eq!(titles(&asc), ["t-a", "t-b", "t-none"]);

        let goal_rows = vec![
            row("g-b", "", "", &[], "", "Ship v1", ""),
            row("g-none", "", "", &[], "", "", ""),
            row("g-a", "", "", &[], "", "Alpha", ""),
        ];
        let asc = sort_rows(goal_rows, (TableCol::Goal, true), &cols());
        assert_eq!(titles(&asc), ["g-a", "g-b", "g-none"]);
    }

    #[test]
    fn every_column_reverses_cleanly_both_directions() {
        // Rows with distinct, all-present values in every column, so asc
        // reversed must exactly equal desc (no empties-last / tiebreak nuance).
        let columns = vec!["c0".into(), "c1".into(), "c2".into()];
        let rows = vec![
            row(
                "aaa",
                "c0",
                "high",
                &["a"],
                "2026-01-01",
                "alpha",
                "2026-01-01",
            ),
            row(
                "bbb",
                "c1",
                "normal",
                &["b"],
                "2026-02-01",
                "beta",
                "2026-02-01",
            ),
            row(
                "ccc",
                "c2",
                "low",
                &["c"],
                "2026-03-01",
                "gamma",
                "2026-03-01",
            ),
        ];
        for col in TableCol::ALL {
            let asc = titles(&sort_rows(rows.clone(), (col, true), &columns));
            let mut desc = titles(&sort_rows(rows.clone(), (col, false), &columns));
            desc.reverse();
            assert_eq!(asc, desc, "column {col:?} should reverse cleanly");
        }
    }
}
