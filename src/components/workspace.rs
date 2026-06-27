//! The in-well workspace shell: the section rail, then the active section.
//!
//! For this increment only **notes** is built; tasks and wiki render a quiet
//! placeholder until their own increments land. The rail and the section view
//! are flex siblings under `.ido-app`.

use leptos::prelude::*;

use crate::components::editor::Editor;
use crate::components::rail::Rail;
use crate::state::{Section, State};

/// Rail + the active section. Mounted by `App` whenever a well is open.
#[component]
pub fn Workspace() -> impl IntoView {
    let state = expect_context::<State>();

    // Global tab shortcuts: Ctrl+Tab / Ctrl+Shift+Tab cycle, Ctrl+1…9 jump,
    // Ctrl+W close. All gated on Ctrl, so they never clash with the block
    // editor's bare-key model inside a textarea. The listener is removed when
    // the workspace unmounts (leaving the well).
    window_event_listener(leptos::ev::keydown, move |ev| {
        if !ev.ctrl_key() {
            return;
        }
        match ev.key().as_str() {
            "Tab" => {
                ev.prevent_default();
                if ev.shift_key() {
                    state.prev_tab();
                } else {
                    state.next_tab();
                }
            }
            "w" | "W" => {
                ev.prevent_default();
                if let Some(i) = state.active_tab.get_untracked() {
                    state.close_tab(i);
                }
            }
            key => {
                if let Some(n) = (key.len() == 1)
                    .then(|| key.chars().next().and_then(|c| c.to_digit(10)))
                    .flatten()
                    .filter(|&n| n >= 1)
                {
                    let idx = (n - 1) as usize;
                    if idx < state.tabs.get_untracked().len() {
                        ev.prevent_default();
                        state.activate_tab(idx);
                    }
                }
            }
        }
    });

    view! {
        <Rail />
        {move || match state.section.get() {
            Section::Notes => view! { <Editor /> }.into_any(),
            Section::Tasks => view! { <SectionStub label="tasks" /> }.into_any(),
            Section::Wiki => view! { <SectionStub label="wiki" /> }.into_any(),
        }}
    }
}

/// Placeholder surface for a not-yet-built section — the 井戸 mark + a hint.
#[component]
fn SectionStub(label: &'static str) -> impl IntoView {
    view! {
        <main class="ido-main">
            <div class="ido-empty">
                <span class="ido-empty-mark">"井戸"</span>
                <span class="ido-empty-text">{format!("{label} — coming soon")}</span>
            </div>
        </main>
    }
}
