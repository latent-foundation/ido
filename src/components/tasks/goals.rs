//! The goals bar above the board: an "all" chip plus one chip per goal (with
//! progress + target date), draggable to reorder. A chip scopes the board.

use leptos::prelude::*;

use super::logic::*;
use crate::dates::this_year;
use crate::icon::Icon;
use crate::model::Goal;
use crate::state::{MenuTarget, State};

/// The goals bar above the board: an "all" chip, one chip per goal (with
/// progress + target), and a "+" to create one. A chip filters the board.
#[component]
pub(crate) fn GoalsBar() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div
            class="ido-goals-bar"
            on:dragover=move |ev| {
                let Some(dragged) = state.goal_drag.get_untracked() else {
                    return;
                };
                ev.prevent_default();
                state.goal_drop_before.set(goal_drop_before_at(&ev, &dragged));
            }
            on:drop=move |ev| {
                ev.prevent_default();
                state.commit_goal_drop();
            }
        >
            <button
                class="ido-goal-chip ido-goal-all"
                class:active=move || state.goal_filter.get().is_none()
                on:click=move |_| state.goal_filter.set(None)
            >
                <Icon name="target" size=12 />
                "all"
            </button>
            {move || {
                let tasks = state.tasks.get();
                let done_col = state.columns.get().last().cloned().unwrap_or_default();
                let goals: Vec<Goal> = state
                    .goals
                    .get()
                    .into_iter()
                    .filter(|g| !g.archived)
                    .collect();
                let last = goals.len().saturating_sub(1);
                goals
                    .into_iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let total = tasks.iter().filter(|t| t.goal == g.id && !t.archived).count();
                        let done = tasks
                            .iter()
                            .filter(|t| t.goal == g.id && t.status == done_col && !t.archived)
                            .count();
                        view! { <GoalChip goal=g done=done total=total is_last=i == last /> }
                    })
                    .collect_view()
            }}
            <button class="ido-goal-new" title="New goal" on:click=move |_| state.add_goal()>
                <Icon name="file-plus" size=13 />
            </button>
        </div>
    }
}

/// One goal chip: name, progress (done/total), an optional target date, and an
/// edit pencil. Click scopes the board to the goal; drag reorders the bar.
#[component]
fn GoalChip(goal: Goal, done: usize, total: usize, is_last: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let Goal {
        id, title, target, ..
    } = goal;
    let overdue = goal_overdue(&target, done, total);
    let id_filter = id.clone();
    let id_active = id.clone();
    let id_edit = id.clone();
    let id_drag = id.clone();
    let id_dragging = id.clone();
    let id_line = id.clone();
    let id_data = id.clone();
    let id_menu = id;
    let name = title;
    view! {
        <div
            class="ido-goal-chip"
            data-goal-id=id_data
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Goal {
                            id: id_menu.clone(),
                        },
                    );
            }
            class:active=move || state.goal_filter.get().as_deref() == Some(id_active.as_str())
            class:dragging=move || state.goal_drag.get().as_deref() == Some(id_dragging.as_str())
            class=(
                "drop-before",
                move || state.goal_drop_before.get().as_deref() == Some(id_line.as_str()),
            )
            class=(
                "drop-after",
                move || {
                    is_last && state.goal_drag.get().is_some()
                        && state.goal_drop_before.get().is_none()
                },
            )
            draggable="true"
            on:click=move |_| state.toggle_goal_filter(id_filter.clone())
            on:dragstart=move |_| state.start_goal_drag(id_drag.clone())
            on:dragend=move |_| state.clear_goal_drag()
        >
            <span class="ido-goal-name">{name}</span>
            <span class="ido-goal-progress">{format!("{done}/{total}")}</span>
            {(!target.is_empty())
                .then(|| {
                    let target_for_title = target.clone();
                    view! {
                        <span
                            class="ido-goal-target"
                            class:overdue=overdue
                            title=move || {
                                let mut s = target_for_title.clone();
                                if overdue {
                                    s.push_str(" — overdue");
                                }
                                s
                            }
                        >
                            {overdue.then(|| view! { <Icon name="clock" size=10 /> })}
                            {format_due(&target, this_year())}
                        </span>
                    }
                })}
            <button
                class="ido-goal-edit"
                title="Edit goal"
                on:click=move |ev| {
                    ev.stop_propagation();
                    state.open_goal(id_edit.clone());
                }
            >
                <Icon name="pencil" size=11 />
            </button>
        </div>
    }
}
