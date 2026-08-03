//! The board proper: the toolbar-topped [`TaskBoard`], its `status` [`Column`]s
//! of draggable [`Card`]s, and the [`Backlog`] of un-columned tasks below,
//! itself drag-reorderable exactly like a column (see [`Backlog`]'s doc for
//! how orphan-status rows sit in that ordering). The toolbar's view toggle
//! swaps the board for the month [`CalendarView`].

use latent_ui::Icon;
use leptos::prelude::*;

use super::archive::ArchivePanel;
use super::calendar::CalendarView;
use super::drawers::{GoalDrawer, TaskDrawer};
use super::goals::GoalsBar;
use super::logic::*;
use super::table::TableView;
use super::views::ViewsMenu;
use crate::dates::{this_year, today_ymd};
use crate::model::Task;
use crate::state::{MenuTarget, State, TaskSort, TaskView};

/// Toolbar + board (or calendar) + backlog, with the detail drawer overlaid.
#[component]
pub fn TaskBoard() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-board-wrap">
            <div class="ido-board-toolbar">
                <input
                    class="ido-board-search"
                    placeholder="filter…"
                    prop:value=move || state.task_filter.get()
                    on:input=move |ev| state.task_filter.set(event_target_value(&ev))
                />
                {move || {
                    state
                        .tag_filter
                        .get()
                        .map(|t| {
                            view! {
                                <button
                                    class="ido-board-tagchip"
                                    title="Clear tag filter"
                                    on:click=move |_| state.tag_filter.set(None)
                                >
                                    {format!("tag: {t}")}
                                    <Icon name="x" size=11 />
                                </button>
                            }
                        })
                }}
                // Sort is a within-column ordering — meaningless on the calendar.
                {move || {
                    (state.task_view.get() == TaskView::Board)
                        .then(|| {
                            view! {
                                <label class="ido-board-control">
                                    "sort"
                                    <select
                                        class="ido-board-select"
                                        prop:value=move || {
                                            match state.task_sort.get() {
                                                TaskSort::Manual => "manual",
                                                TaskSort::Priority => "priority",
                                                TaskSort::Due => "due",
                                            }
                                        }
                                        on:change=move |ev| {
                                            state.task_sort.set(parse_sort(&event_target_value(&ev)))
                                        }
                                    >
                                        <option value="manual">"manual"</option>
                                        <option value="priority">"priority"</option>
                                        <option value="due">"due"</option>
                                    </select>
                                </label>
                            }
                        })
                }}
                <label class="ido-board-control">
                    <input
                        type="checkbox"
                        prop:checked=move || state.hide_done.get()
                        on:change=move |ev| state.hide_done.set(event_target_checked(&ev))
                    />
                    "hide done"
                </label>
                <ViewsMenu />
                <div class="ido-view-toggle">
                    <button
                        class="ido-view-btn"
                        class:active=move || state.task_view.get() == TaskView::Board
                        title="Board view"
                        on:click=move |_| state.task_view.set(TaskView::Board)
                    >
                        <Icon name="square-kanban" size=14 />
                    </button>
                    <button
                        class="ido-view-btn"
                        class:active=move || state.task_view.get() == TaskView::Calendar
                        title="Calendar view"
                        on:click=move |_| state.task_view.set(TaskView::Calendar)
                    >
                        <Icon name="calendar-days" size=14 />
                    </button>
                    <button
                        class="ido-view-btn"
                        class:active=move || state.task_view.get() == TaskView::Table
                        title="Table view"
                        on:click=move |_| state.task_view.set(TaskView::Table)
                    >
                        <Icon name="table" size=14 />
                    </button>
                </div>
                <button
                    class="ido-board-archive"
                    class:active=move || state.show_archive.get()
                    title="Archive"
                    on:click=move |_| state.toggle_archive()
                >
                    <Icon name="archive" size=14 />
                    "archive"
                </button>
            </div>

            {move || {
                if state.show_archive.get() {
                    view! { <ArchivePanel /> }.into_any()
                } else if state.task_view.get() == TaskView::Calendar {
                    view! {
                        <GoalsBar />
                        <CalendarView />
                    }
                        .into_any()
                } else if state.task_view.get() == TaskView::Table {
                    // The table already lists backlog tasks, so no <Backlog />.
                    view! {
                        <GoalsBar />
                        <TableView />
                    }
                        .into_any()
                } else {
                    view! {
                        <GoalsBar />
                        <div class="ido-board">
                            {move || {
                                let cols = state.columns.get();
                                let last = cols.len().saturating_sub(1);
                                cols.into_iter()
                                    .enumerate()
                                    .map(|(i, col)| view! { <Column name=col is_done=i == last /> })
                                    .collect_view()
                            }}
                        </div>
                        <Backlog />
                    }
                        .into_any()
                }
            }}

            <TaskDrawer />
            <GoalDrawer />
        </div>
    }
}

/// One kanban column: header with a (filtered) count, its cards (a drop target
/// that highlights while hovered), and a quick-add footer.
#[component]
fn Column(name: String, is_done: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let n_over = name.clone();
    let n_dragover = name.clone();
    let n_end = name.clone();
    let n_cards = name.clone();
    let n_count = name.clone();
    let n_add = name.clone();
    view! {
        <div
            class="ido-column"
            class:drop=move || state.task_drag_over.get().as_deref() == Some(n_over.as_str())
            on:dragover=move |ev| {
                let Some(dragged) = state.task_drag.get_untracked() else {
                    return;
                };
                ev.prevent_default();
                state.task_drag_over.set(Some(n_dragover.clone()));
                let before = (state.task_sort.get_untracked() == TaskSort::Manual)
                    .then(|| drop_before_at(&ev, &dragged, ".ido-task-card"))
                    .flatten();
                state.task_drop_before.set(before);
            }
            on:drop=move |ev| {
                ev.prevent_default();
                state.commit_card_drop();
            }
        >
            <div class="ido-column-head">
                <span class="ido-column-name">{col_label(&name)}</span>
                <span class="ido-column-count">
                    {move || {
                        let f = state.task_filter.get();
                        let gf = state.goal_filter.get();
                        let tf = state.tag_filter.get();
                        state
                            .tasks
                            .get()
                            .iter()
                            .filter(|t| {
                                t.status == n_count && !t.archived && matches(t, &f)
                                    && goal_ok(t, &gf) && tag_ok(t, &tf)
                            })
                            .count()
                    }}
                </span>
            </div>
            <div
                class="ido-column-cards"
                class=(
                    "drop-end",
                    move || {
                        state.task_drag_over.get().as_deref() == Some(n_end.as_str())
                            && state.task_drop_before.get().is_none()
                            && state.task_sort.get() == TaskSort::Manual
                    },
                )
            >
                {move || {
                    if is_done && state.hide_done.get() {
                        return ().into_any();
                    }
                    let f = state.task_filter.get();
                    let gf = state.goal_filter.get();
                    let tf = state.tag_filter.get();
                    let sort = state.task_sort.get();
                    let tasks = sorted(
                        state
                            .tasks
                            .get()
                            .into_iter()
                            .filter(|t| {
                                t.status == n_cards && !t.archived && matches(t, &f)
                                    && goal_ok(t, &gf) && tag_ok(t, &tf)
                            })
                            .collect(),
                        sort,
                    );
                    tasks
                        .into_iter()
                        .map(|t| view! { <Card task=t is_done=is_done /> })
                        .collect_view()
                        .into_any()
                }}
            </div>
            <QuickAdd status=n_add />
        </div>
    }
}

/// The column footer: a quiet "add" button that swaps to a capture input.
/// Enter creates a task with the typed title and keeps the input open (and
/// focused) for the next one; Esc closes; blur commits any typed text
/// (failsafe, like the block editor) and closes.
#[component]
fn QuickAdd(status: String) -> impl IntoView {
    let state = expect_context::<State>();
    let open = RwSignal::new(false);
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Focus the input when it appears (the ref turns `Some` on mount).
    Effect::new(move |_| {
        if open.get()
            && let Some(el) = input_ref.get()
        {
            let _ = el.focus();
        }
    });

    // Create a task from the input's text (if any) and clear it for the next.
    let submit = Callback::new(move |_: ()| {
        let Some(el) = input_ref.get_untracked() else {
            return;
        };
        let title = el.value();
        el.set_value("");
        if !title.trim().is_empty() {
            state.add_task_quick(status.clone(), title, None);
        }
    });

    view! {
        {move || {
            if open.get() {
                view! {
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
                                    open.set(false);
                                }
                                _ => {}
                            }
                        }
                        on:blur=move |_| {
                            submit.run(());
                            open.set(false);
                        }
                    />
                }
                    .into_any()
            } else {
                view! {
                    <button class="ido-column-add" on:click=move |_| open.set(true)>
                        <Icon name="file-plus" size=14 />
                        "add"
                    </button>
                }
                    .into_any()
            }
        }}
    }
}

/// A task card: title plus a quiet metadata row (priority / tags / due, the
/// due date flagged when today or past — unless this is the done column).
#[component]
fn Card(task: Task, is_done: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let Task {
        id,
        title,
        priority,
        tags,
        due,
        repeat,
        checks_done,
        checks_total,
        ..
    } = task;
    let id_click = id.clone();
    let id_drag = id.clone();
    let id_active = id.clone();
    let id_dragging = id.clone();
    let id_line = id.clone();
    let id_data = id.clone();
    let id_menu = id.clone();
    let id_kbd = id.clone();
    let pri_class = priority.clone();
    let has_meta = !priority.is_empty()
        || !tags.is_empty()
        || !due.is_empty()
        || !repeat.is_empty()
        || checks_total > 0;
    view! {
        <div
            class="ido-task-card"
            data-task-id=id_data
            tabindex="0"
            class:active=move || state.active_task.get().as_deref() == Some(id_active.as_str())
            class:dragging=move || state.task_drag.get().as_deref() == Some(id_dragging.as_str())
            class=(
                "drop-before",
                move || { state.task_drop_before.get().as_deref() == Some(id_line.as_str()) },
            )
            draggable="true"
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
            on:keydown=move |ev| {
                match ev.key().as_str() {
                    "Enter" => state.open_task(id_kbd.clone()),
                    " " => {
                        ev.prevent_default();
                        state.open_task(id_kbd.clone());
                    }
                    "ArrowUp" => focus_row(&ev, &id_kbd, -1),
                    "ArrowDown" => focus_row(&ev, &id_kbd, 1),
                    "ArrowLeft" => focus_column(&ev, &id_kbd, -1),
                    "ArrowRight" => focus_column(&ev, &id_kbd, 1),
                    _ => {}
                }
            }
        >
            <div class="ido-task-title">{title}</div>
            {has_meta
                .then(|| {
                    view! {
                        <div class="ido-task-meta">
                            {(!priority.is_empty())
                                .then(|| {
                                    view! {
                                        <span class=format!(
                                            "ido-task-pri ido-pri-{pri_class}",
                                        )>{priority}</span>
                                    }
                                })}
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
                            {(!due.is_empty())
                                .then(|| view! { <DueTag due=due is_done=is_done /> })}
                            {(!repeat.is_empty())
                                .then(|| {
                                    view! {
                                        <span class="ido-task-repeat" title=repeat.clone()>
                                            <Icon name="repeat" size=10 />
                                        </span>
                                    }
                                })}
                            {(checks_total > 0)
                                .then(|| {
                                    view! { <ChecksTag done=checks_done total=checks_total /> }
                                })}
                        </div>
                    }
                })}
        </div>
    }
}

/// The checklist rollup chip (`3/7`): the count of checked task-list items in
/// the body, out of the total — the cheapest "subtasks" signal. Muted further
/// once complete (`done == total`). Shared by [`Card`] and [`BacklogRow`].
#[component]
fn ChecksTag(done: u32, total: u32) -> impl IntoView {
    view! {
        <span class="ido-task-checks" class:complete=done == total>
            <Icon name="square-check" size=10 />
            {format!("{done}/{total}")}
        </span>
    }
}

/// A card's due date: day-first ("7 Jul"), flagged `overdue` (accent + clock)
/// or `due-today` — except on done-column cards, where the date is just quiet.
#[component]
fn DueTag(due: String, is_done: bool) -> impl IntoView {
    let urgency = due_state(&due, &today_ymd(), is_done);
    let class = if urgency.is_empty() {
        "ido-task-due".to_string()
    } else {
        format!("ido-task-due {urgency}")
    };
    view! {
        <span class=class title=due.clone()>
            {(urgency == "overdue").then(|| view! { <Icon name="clock" size=10 /> })}
            {format_due(&due, this_year())}
        </span>
    }
}

/// The backlog: tasks not assigned to any board column (status empty, or a
/// status that no longer matches a column). Drop a card here to un-column it,
/// or drag within it to reorder.
///
/// **Orphan rows** (a non-empty `status` that no column matches — e.g. a
/// column renamed/deleted out from under it) render alongside true-backlog
/// tasks (empty `status`) but are a distinct pool: `reorder_column`'s manual
/// order is a per-status sequence, so only true-backlog rows are renumbered
/// on a backlog drop (`commit_card_drop` filters `t.status == col`, i.e.
/// `""`) — an orphan's own `order` (inherited from its old column) is left
/// alone, and it keeps whatever position that gives it in the merged list.
/// Consequently an orphan row can never be a drop *target* either: the
/// backlog's `dragover` measures only `.ido-backlog-row[data-orphan="false"]`
/// (see [`drop_before_at`]), so dropping "before" an orphan resolves to the
/// next true-backlog row after it in DOM order (or the end, if it's last) —
/// the nearest position that's actually representable.
#[component]
fn Backlog() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div
            class="ido-backlog"
            class:drop=move || state.task_drag_over.get().as_deref() == Some("")
            on:dragover=move |ev| {
                let Some(dragged) = state.task_drag.get_untracked() else {
                    return;
                };
                ev.prevent_default();
                state.task_drag_over.set(Some(String::new()));
                let before = drop_before_at(
                    &ev,
                    &dragged,
                    ".ido-backlog-row[data-orphan=\"false\"]",
                );
                state.task_drop_before.set(before);
            }
            on:drop=move |ev| {
                ev.prevent_default();
                state.commit_card_drop();
            }
        >
            <div class="ido-backlog-head">
                <Icon name="square" size=12 />
                "backlog"
            </div>
            <div
                class="ido-backlog-list"
                class=(
                    "drop-end",
                    move || {
                        state.task_drag_over.get().as_deref() == Some("")
                            && state.task_drop_before.get().is_none()
                    },
                )
            >
                {move || {
                    let cols = state.columns.get();
                    let f = state.task_filter.get();
                    let gf = state.goal_filter.get();
                    let tf = state.tag_filter.get();
                    let items: Vec<Task> = state
                        .tasks
                        .get()
                        .into_iter()
                        .filter(|t| {
                            !cols.contains(&t.status) && !t.archived && matches(t, &f)
                                && goal_ok(t, &gf) && tag_ok(t, &tf)
                        })
                        .collect();
                    if items.is_empty() {
                        view! { <div class="ido-backlog-empty">"nothing in the backlog"</div> }
                            .into_any()
                    } else {
                        items
                            .into_iter()
                            .map(|t| view! { <BacklogRow task=t /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// A compact backlog row: name, an orphan-status hint, priority + due. Drag
/// mechanics mirror [`Card`] exactly (`data-task-id`, `dragging`,
/// `drop-before`), plus `data-orphan` on orphan-status rows — the marker
/// [`drop_before_at`]'s selector excludes so they're never a drop target
/// (see the orphan-rows note on [`Backlog`]).
#[component]
fn BacklogRow(task: Task) -> impl IntoView {
    let state = expect_context::<State>();
    let Task {
        id,
        title,
        priority,
        due,
        status,
        repeat,
        checks_done,
        checks_total,
        ..
    } = task;
    let is_orphan = !status.is_empty();
    let id_click = id.clone();
    let id_drag = id.clone();
    let id_dragging = id.clone();
    let id_line = id.clone();
    let id_data = id.clone();
    let id_menu = id.clone();
    let id_kbd = id;
    let pri_class = priority.clone();
    view! {
        <div
            class="ido-backlog-row"
            data-task-id=id_data
            data-orphan=if is_orphan { "true" } else { "false" }
            tabindex="0"
            draggable="true"
            class:dragging=move || state.task_drag.get().as_deref() == Some(id_dragging.as_str())
            class=(
                "drop-before",
                move || { state.task_drop_before.get().as_deref() == Some(id_line.as_str()) },
            )
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
            <span class="ido-backlog-name">{title}</span>
            <span class="ido-backlog-meta">
                {(!status.is_empty())
                    .then(|| {
                        view! { <span class="ido-backlog-orphan">{col_label(&status)}</span> }
                    })}
                {(!priority.is_empty())
                    .then(|| {
                        view! {
                            <span class=format!(
                                "ido-task-pri ido-pri-{pri_class}",
                            )>{priority}</span>
                        }
                    })} {(!due.is_empty()).then(|| view! { <DueTag due=due is_done=false /> })}
                {(!repeat.is_empty())
                    .then(|| {
                        view! {
                            <span class="ido-task-repeat" title=repeat.clone()>
                                <Icon name="repeat" size=10 />
                            </span>
                        }
                    })}
                {(checks_total > 0)
                    .then(|| view! { <ChecksTag done=checks_done total=checks_total /> })}
            </span>
        </div>
    }
}
