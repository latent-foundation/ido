//! The slide-in detail drawers for a task and a goal: editable fields plus the
//! shared three-mode body editor ([`DocEditor`]) on a local buffer.
//!
//! **Keyed on the id, not a task snapshot.** The outer closures read only
//! `active_task` / `active_goal`, so `DrawerInner` / `GoalDrawerInner` are
//! recreated on open / switch / rename (each re-points the active id) — but a
//! plain board refresh (which follows every `set_task_field` / `update_task_body`)
//! does **not** re-run them, so a focused input, its scroll, or an in-progress
//! body edit is never dropped by a background reload. Inside, the editable
//! inputs are **uncontrolled** — seeded once per mount from a snapshot taken at
//! open — while only display-only bits (existence → auto-close, the archived
//! flag) read `state.tasks` / `state.goals` reactively.

use latent_ui::Icon;
use leptos::prelude::*;

use super::logic::col_label;
use super::tagsinput::TagsInput;
use crate::blocks;
use crate::components::datepicker::DatePicker;
use crate::components::mainpane::DocEditor;
use crate::model::Task;
use crate::state::{Buffer, Mode, State};

/// The detail drawer — renders only while a task is selected. Keyed on the
/// selected id so a board refresh doesn't recreate the fields (see module doc).
#[component]
pub(crate) fn TaskDrawer() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            let id = state.active_task.get()?;
            Some(view! { <DrawerInner id=id /> })
        }}
    }
}

/// The drawer's editable fields for one task, keyed on `id`. The inputs are
/// **uncontrolled**, seeded once from a snapshot taken at mount; only existence
/// (auto-close) and the archived flag read `state.tasks` reactively.
#[component]
fn DrawerInner(id: String) -> impl IntoView {
    let state = expect_context::<State>();

    // Snapshot the task at open — seeds the uncontrolled inputs + body buffer.
    // The drawer is keyed on `id`, so we're recreated on open/switch/rename with
    // a fresh `state.tasks`; a background refresh never re-runs this.
    let Some(seed) = state.tasks.get_untracked().into_iter().find(|t| t.id == id) else {
        // Defensive: the outer closure only mounts us for a live `active_task`
        // that exists on the board, so this shouldn't fire.
        return ().into_any();
    };
    let Task {
        title,
        status,
        priority,
        tags,
        due,
        goal,
        repeat,
        body,
        ..
    } = seed;

    let columns = state.columns.get_untracked();
    let status_is_backlog = status.is_empty();
    // Show the current status as an extra option when it no longer matches a column.
    let status_orphan = (!status.is_empty() && !columns.contains(&status)).then(|| status.clone());

    // The built-in repeat choices; a hand-written spec (e.g. "every 3 days") is
    // valid backend vocabulary but isn't one of these, so it renders as an
    // extra selected option — the same orphan-option pattern as status.
    const REPEAT_OPTIONS: [&str; 6] = ["", "daily", "weekly", "every 2 weeks", "monthly", "yearly"];
    let repeat_orphan =
        (!repeat.is_empty() && !REPEAT_OPTIONS.contains(&repeat.as_str())).then(|| repeat.clone());

    // Reactive existence: if the task disappears from the board (deleted
    // elsewhere) close the drawer — the equivalent of the old outer `find`
    // returning None. Guarded on `active_task` so it can't clobber a rename that
    // just re-pointed the active id (the old drawer's effect may run once during
    // teardown, when `active_task` already points at the new id).
    let id_exist = id.clone();
    Effect::new(move |_| {
        let present = state.tasks.get().iter().any(|t| t.id == id_exist);
        if !present && state.active_task.get_untracked().as_deref() == Some(id_exist.as_str()) {
            state.close_task();
        }
    });

    // Reactive archived flag: the footer button flips label/icon in place while
    // the drawer stays open (archived tasks remain in `list_tasks`).
    let id_arch_memo = id.clone();
    let archived = Memo::new(move |_| {
        state
            .tasks
            .get()
            .iter()
            .find(|t| t.id == id_arch_memo)
            .map(|t| t.archived)
            .unwrap_or(false)
    });

    let id_title = id.clone();
    let id_status = id.clone();
    let id_pri = id.clone();
    let id_tags = id.clone();
    let id_due = id.clone();
    let id_repeat = id.clone();
    let id_goal = id.clone();
    let id_body = id.clone();
    let id_arch = id.clone();
    let id_del = id.clone();
    let tags_value = tags.join(", ");
    let goal_is_empty = goal.is_empty();
    // The selected goal seeds once (uncontrolled); the option LIST stays reactive
    // so goals added elsewhere appear.
    let goal_sel = goal;

    // The body reuses the notes/wiki block editor on its own buffer, created once
    // per mount and persisting through `update_task_body` (not the tab save path).
    // Because the drawer is keyed on `id`, a board refresh never recreates this
    // buffer, so in-progress body editing survives field commits + reloads.
    let body_buffer = Buffer {
        content: RwSignal::new(body.clone()),
        blocks: RwSignal::new(blocks::segment(&body)),
        // Start inert (placeholder/blocks) — don't steal focus from the fields.
        active_block: RwSignal::new(None),
        source_editor: NodeRef::new(),
        open: Signal::derive(|| true),
        save: Callback::new(move |src: String| state.update_task_body(id_body.clone(), src)),
    };
    let body_mode = RwSignal::new(Mode::Live);

    view! {
        <aside class="ido-drawer">
            <div class="ido-drawer-head">
                <input
                    class="ido-drawer-title"
                    prop:value=title
                    placeholder="task title"
                    spellcheck="false"
                    on:change=move |ev| state.rename_task(id_title.clone(), event_target_value(&ev))
                />
                <button class="ido-drawer-close" title="Close" on:click=move |_| state.close_task()>
                    <Icon name="x" size=15 />
                </button>
            </div>

            <div class="ido-drawer-fields">
                <Field label="status">
                    <select
                        class="ido-drawer-input"
                        on:change=move |ev| {
                            state.move_task_to(id_status.clone(), event_target_value(&ev))
                        }
                    >
                        <option value="" selected=status_is_backlog>
                            "(backlog)"
                        </option>
                        {status_orphan
                            .map(|s| {
                                let label = col_label(&s);
                                view! {
                                    <option value=s selected=true>
                                        {label}
                                    </option>
                                }
                            })}
                        {columns
                            .into_iter()
                            .map(|c| {
                                let selected = c == status;
                                let label = col_label(&c);
                                view! {
                                    <option value=c selected=selected>
                                        {label}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </Field>

                <Field label="priority">
                    <select
                        class="ido-drawer-input"
                        on:change=move |ev| {
                            state
                                .set_task_field(
                                    id_pri.clone(),
                                    "priority".into(),
                                    event_target_value(&ev),
                                )
                        }
                    >
                        {["", "low", "normal", "high"]
                            .into_iter()
                            .map(|p| {
                                let label = if p.is_empty() { "none" } else { p };
                                view! {
                                    <option value=p selected=p == priority>
                                        {label}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </Field>

                <Field label="tags">
                    <TagsInput
                        value=tags_value
                        on_commit=Callback::new(move |v: String| {
                            state.set_task_field(id_tags.clone(), "tags".into(), v)
                        })
                    />
                </Field>

                <Field label="due">
                    <DatePicker
                        value=due
                        on_change=Callback::new(move |v: String| {
                            state.set_task_field(id_due.clone(), "due".into(), v)
                        })
                    />
                </Field>

                <Field label="repeat">
                    <select
                        class="ido-drawer-input"
                        on:change=move |ev| {
                            state
                                .set_task_field(
                                    id_repeat.clone(),
                                    "repeat".into(),
                                    event_target_value(&ev),
                                )
                        }
                    >
                        {repeat_orphan
                            .map(|r| {
                                let label = r.clone();
                                view! {
                                    <option value=r selected=true>
                                        {label}
                                    </option>
                                }
                            })}
                        {REPEAT_OPTIONS
                            .into_iter()
                            .map(|r| {
                                let label = if r.is_empty() { "none" } else { r };
                                view! {
                                    <option value=r selected=r == repeat>
                                        {label}
                                    </option>
                                }
                            })
                            .collect_view()}
                    </select>
                </Field>

                <Field label="goal">
                    <select
                        class="ido-drawer-input"
                        on:change=move |ev| {
                            state
                                .set_task_field(
                                    id_goal.clone(),
                                    "goal".into(),
                                    event_target_value(&ev),
                                )
                        }
                    >
                        <option value="" selected=goal_is_empty>
                            "(none)"
                        </option>
                        {move || {
                            state
                                .goals
                                .get()
                                .into_iter()
                                .filter(|g| !g.archived)
                                .map(|g| {
                                    let selected = g.id == goal_sel;
                                    view! {
                                        <option value=g.id selected=selected>
                                            {g.title}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </select>
                </Field>
            </div>

            <DocEditor buffer=body_buffer mode=body_mode />

            <div class="ido-drawer-foot">
                <button
                    class="ido-drawer-archive"
                    on:click=move |_| state.archive_task(id_arch.clone(), !archived.get_untracked())
                >
                    {move || {
                        view! {
                            <Icon
                                name=if archived.get() { "rotate-ccw" } else { "archive" }
                                size=13
                            />
                        }
                    }}
                    {move || if archived.get() { "restore" } else { "archive" }}
                </button>
                <button
                    class="ido-drawer-delete"
                    on:click=move |_| state.remove_task(id_del.clone())
                >
                    <Icon name="trash" size=13 />
                    "delete task"
                </button>
            </div>
        </aside>
    }
    .into_any()
}

/// A labelled drawer field row.
#[component]
fn Field(label: &'static str, children: Children) -> impl IntoView {
    view! {
        <label class="ido-drawer-field">
            <span class="ido-drawer-label">{label}</span>
            {children()}
        </label>
    }
}

/// The goal detail drawer — renders only while a goal is selected. Keyed on the
/// selected id (same rationale as [`TaskDrawer`]).
#[component]
pub(crate) fn GoalDrawer() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            let id = state.active_goal.get()?;
            Some(view! { <GoalDrawerInner id=id /> })
        }}
    }
}

/// The goal drawer's editable fields (title, target date, body), keyed on `id`.
/// Inputs are uncontrolled + seeded once; existence and the archived flag are
/// reactive.
#[component]
fn GoalDrawerInner(id: String) -> impl IntoView {
    let state = expect_context::<State>();

    let Some(seed) = state.goals.get_untracked().into_iter().find(|g| g.id == id) else {
        return ().into_any();
    };
    let title = seed.title;
    let target = seed.target;
    let body = seed.body;

    // Auto-close if the goal disappears (deleted elsewhere); guarded so it can't
    // clobber a rename that just re-pointed `active_goal` (see [`DrawerInner`]).
    let id_exist = id.clone();
    Effect::new(move |_| {
        let present = state.goals.get().iter().any(|g| g.id == id_exist);
        if !present && state.active_goal.get_untracked().as_deref() == Some(id_exist.as_str()) {
            state.close_goal();
        }
    });

    let id_arch_memo = id.clone();
    let archived = Memo::new(move |_| {
        state
            .goals
            .get()
            .iter()
            .find(|g| g.id == id_arch_memo)
            .map(|g| g.archived)
            .unwrap_or(false)
    });

    let id_title = id.clone();
    let id_target = id.clone();
    let id_body = id.clone();
    let id_arch = id.clone();
    let id_del = id.clone();

    // The body reuses the shared three-mode editor on its own buffer, created
    // once per mount and persisting through `update_goal_body`.
    let body_buffer = Buffer {
        content: RwSignal::new(body.clone()),
        blocks: RwSignal::new(blocks::segment(&body)),
        active_block: RwSignal::new(None),
        source_editor: NodeRef::new(),
        open: Signal::derive(|| true),
        save: Callback::new(move |src: String| state.update_goal_body(id_body.clone(), src)),
    };
    let body_mode = RwSignal::new(Mode::Live);

    view! {
        <aside class="ido-drawer">
            <div class="ido-drawer-head">
                <input
                    class="ido-drawer-title"
                    prop:value=title
                    placeholder="goal title"
                    spellcheck="false"
                    on:change=move |ev| state.rename_goal(id_title.clone(), event_target_value(&ev))
                />
                <button class="ido-drawer-close" title="Close" on:click=move |_| state.close_goal()>
                    <Icon name="x" size=15 />
                </button>
            </div>

            <div class="ido-drawer-fields">
                <Field label="target">
                    <DatePicker
                        value=target
                        on_change=Callback::new(move |v: String| {
                            state.set_goal_field(id_target.clone(), "target".into(), v)
                        })
                    />
                </Field>
            </div>

            <DocEditor buffer=body_buffer mode=body_mode />

            <div class="ido-drawer-foot">
                <button
                    class="ido-drawer-archive"
                    on:click=move |_| state.archive_goal(id_arch.clone(), !archived.get_untracked())
                >
                    {move || {
                        view! {
                            <Icon
                                name=if archived.get() { "rotate-ccw" } else { "archive" }
                                size=13
                            />
                        }
                    }}
                    {move || if archived.get() { "restore" } else { "archive" }}
                </button>
                <button
                    class="ido-drawer-delete"
                    on:click=move |_| state.remove_goal(id_del.clone())
                >
                    <Icon name="trash" size=13 />
                    "delete goal"
                </button>
            </div>
        </aside>
    }
    .into_any()
}
