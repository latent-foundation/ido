//! The archive view (shown while the toolbar's archive toggle is on): archived
//! tasks and goals, each restorable or deletable.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::model::{Goal, Task};
use crate::state::State;

/// The archive view: archived tasks and goals, each with restore + delete.
/// Replaces the board while the toolbar's archive toggle is on.
#[component]
pub(crate) fn ArchivePanel() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-archive">
            <div class="ido-archive-group">
                <div class="ido-archive-head">"archived tasks"</div>
                {move || {
                    let items: Vec<Task> = state
                        .tasks
                        .get()
                        .into_iter()
                        .filter(|t| t.archived)
                        .collect();
                    if items.is_empty() {
                        view! { <div class="ido-archive-empty">"no archived tasks"</div> }
                            .into_any()
                    } else {
                        items
                            .into_iter()
                            .map(|t| view! { <ArchiveRow id=t.id title=t.title is_task=true /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
            <div class="ido-archive-group">
                <div class="ido-archive-head">"archived goals"</div>
                {move || {
                    let items: Vec<Goal> = state
                        .goals
                        .get()
                        .into_iter()
                        .filter(|g| g.archived)
                        .collect();
                    if items.is_empty() {
                        view! { <div class="ido-archive-empty">"no archived goals"</div> }
                            .into_any()
                    } else {
                        items
                            .into_iter()
                            .map(|g| view! { <ArchiveRow id=g.id title=g.title is_task=false /> })
                            .collect_view()
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}

/// One archived item: click the name to open its drawer; restore or delete it.
#[component]
fn ArchiveRow(id: String, title: String, is_task: bool) -> impl IntoView {
    let state = expect_context::<State>();
    let id_open = id.clone();
    let id_restore = id.clone();
    let id_del = id;
    let name = title;
    view! {
        <div class="ido-archive-row">
            <button
                class="ido-archive-name"
                on:click=move |_| {
                    if is_task {
                        state.open_task(id_open.clone());
                    } else {
                        state.open_goal(id_open.clone());
                    }
                }
            >
                {name}
            </button>
            <button
                class="ido-archive-action"
                title="Restore"
                on:click=move |_| {
                    if is_task {
                        state.archive_task(id_restore.clone(), false);
                    } else {
                        state.archive_goal(id_restore.clone(), false);
                    }
                }
            >
                <Icon name="rotate-ccw" size=13 />
                "restore"
            </button>
            <button
                class="ido-archive-action ido-archive-del"
                title="Delete permanently"
                on:click=move |_| {
                    if is_task {
                        state.remove_task(id_del.clone());
                    } else {
                        state.remove_goal(id_del.clone());
                    }
                }
            >
                <Icon name="trash" size=13 />
            </button>
        </div>
    }
}
