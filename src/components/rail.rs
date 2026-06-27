//! The left section rail: switch between a well's sections, and open settings.
//!
//! A narrow vertical column on the far left of the workspace. Lucide icons only
//! (inlined via [`Icon`]); the active section carries the accent. The 井戸 mark
//! at the top switches wells (back to the launcher); settings sit at the bottom.

use leptos::prelude::*;

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
                <RailItem section=Section::Tasks icon="square-kanban" label="tasks" />
                <RailItem section=Section::Wiki icon="book" label="wiki" />
            </div>
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

/// One rail button — highlights while its section is the active one.
#[component]
fn RailItem(section: Section, icon: &'static str, label: &'static str) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <button
            class="ido-rail-item"
            class:active=move || state.section.get() == section
            title=label
            on:click=move |_| state.section.set(section)
        >
            <Icon name=icon size=18 />
        </button>
    }
}
