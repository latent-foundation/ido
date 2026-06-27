//! The main-pane tab strip — the open entries across the well.
//!
//! A quiet horizontal strip atop the main pane. Each tab carries a section icon,
//! the entry name, and a close affordance on hover; the active tab takes the
//! accent underline. Tabs are click-to-focus, middle-click / `×` to close, and
//! drag-reorderable. Keyboard switching lives in [`crate::components::workspace`].
//!
//! For this increment every tab is a note, so the strip lives inside the notes
//! main pane; it moves to the shared workspace shell when a second section lands.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::state::{State, Tab, TabTarget};

/// The strip of open tabs; renders nothing when none are open.
#[component]
pub fn TabStrip() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            let tabs = state.tabs.get();
            (!tabs.is_empty())
                .then(|| {
                    view! {
                        <div class="ido-tabs">
                            {tabs
                                .into_iter()
                                .enumerate()
                                .map(|(i, tab)| view! { <TabItem idx=i tab=tab /> })
                                .collect_view()}
                        </div>
                    }
                })
        }}
    }
}

/// The icon name for a tab's target (a note today; wiki/task get their own).
fn target_icon(target: &TabTarget) -> &'static str {
    match target {
        TabTarget::Note(_) => "file-text",
    }
}

/// A single tab: section icon, name, close button. Click focuses; middle-click
/// and the `×` close; the row is draggable to reorder.
#[component]
fn TabItem(idx: usize, tab: Tab) -> impl IntoView {
    let state = expect_context::<State>();
    let icon = target_icon(&tab.target);
    let name = tab
        .target
        .note_id()
        .map(|id| id.rsplit('/').next().unwrap_or(id).to_string())
        .unwrap_or_default();
    view! {
        <div
            class="ido-tab"
            class:active=move || state.active_tab.get() == Some(idx)
            class:dragging=move || state.tab_drag.get() == Some(idx)
            draggable="true"
            title=name.clone()
            on:click=move |_| state.activate_tab(idx)
            on:mousedown=move |ev| {
                if ev.button() == 1 {
                    ev.prevent_default();
                    state.close_tab(idx);
                }
            }
            on:dragstart=move |_| state.tab_drag.set(Some(idx))
            on:dragend=move |_| state.tab_drag.set(None)
            on:dragover=move |ev| {
                if state.tab_drag.get_untracked().is_some() {
                    ev.prevent_default();
                }
            }
            on:drop=move |ev| {
                ev.prevent_default();
                if let Some(from) = state.tab_drag.get_untracked() {
                    state.reorder_tabs(from, idx);
                }
            }
        >
            <span class="ido-tab-icon">
                <Icon name=icon size=13 />
            </span>
            <span class="ido-tab-name">{name.clone()}</span>
            <button
                class="ido-tab-close"
                title="close"
                on:click=move |ev| {
                    ev.stop_propagation();
                    state.close_tab(idx);
                }
            >
                <Icon name="x" size=12 />
            </button>
        </div>
    }
}
