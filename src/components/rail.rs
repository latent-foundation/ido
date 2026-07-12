//! The left section rail: switch between a well's sections, and open settings.
//!
//! A narrow vertical column on the far left of the workspace. Lucide icons only
//! (inlined via [`Icon`]); the active section carries the accent. The 井戸 mark
//! at the top switches wells (back to the launcher); settings sit at the bottom.

use leptos::prelude::*;

use crate::components::tasks::overdue_count;
use crate::dates;
use crate::icon::Icon;
use crate::state::{Section, State};

/// The far-left rail. Reads/sets `state.section`; opens the settings modal.
#[component]
pub fn Rail() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <nav class="ido-rail">
            <button class="ido-rail-mark" title="switch well" on:click=move |_| state.leave_well()>
                "井戸"
            </button>
            <div class="ido-rail-items">
                <RailItem section=Section::Notes icon="file-text" label="notes" />
                <RailItem section=Section::Tasks icon="square-kanban" label="tasks">
                    // Reactive over `state.tasks` + `state.columns`; `today_ymd()` is read
                    // fresh on every recompute, so a "today" that goes stale right after
                    // midnight self-corrects on the next tasks/columns refresh — acceptable
                    // for a rail badge.
                    {move || {
                        let tasks = state.tasks.get();
                        let cols = state.columns.get();
                        let done_col = cols.last().map(String::as_str).unwrap_or("");
                        let n = overdue_count(&tasks, done_col, &dates::today_ymd());
                        (n > 0).then(|| view! { <span class="ido-rail-badge">{n}</span> })
                    }}
                </RailItem>
                <RailItem section=Section::Wiki icon="book" label="wiki" />
            </div>
            <button
                class="ido-rail-item"
                title="search (Ctrl+K)"
                on:click=move |_| state.open_search()
            >
                <Icon name="search" size=18 />
            </button>
            <button
                class="ido-rail-item"
                title="settings"
                on:click=move |_| state.settings_open.set(true)
            >
                <Icon name="settings" size=18 />
            </button>
        </nav>
    }
}

/// One rail button — highlights while its section is the active one. `children`
/// is an optional extra overlay (currently just the tasks badge below).
#[component]
fn RailItem(
    section: Section,
    icon: &'static str,
    label: &'static str,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <button
            class="ido-rail-item"
            class:active=move || state.section.get() == section
            title=label
            on:click=move |_| state.section.set(section)
        >
            <Icon name=icon size=18 />
            {children.map(|c| c())}
        </button>
    }
}
