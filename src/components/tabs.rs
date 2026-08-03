//! The main-pane tab strip — the open entries in one editor pane.
//!
//! A quiet horizontal strip atop each pane. Each tab carries a section icon, the
//! entry name, and a close affordance on hover; the active tab takes the accent
//! underline, a preview tab is italic. Tabs are click-to-focus, middle-click /
//! `×` to close, double-click to pin, and drag-reorderable — within a strip, to
//! the other pane's strip (split view), or to the editor's split zone to open a
//! split. An insertion line marks where a drop lands. Keyboard switching lives in
//! [`crate::components::workspace`].

use latent_ui::Icon;
use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::state::{MenuTarget, Pane, State, Tab, TabDrop};

/// The strip of open tabs for `pane` (index `idx`); nothing when it has none.
#[component]
pub fn TabStrip(pane: Pane, idx: usize) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            let tabs = pane.tabs.get();
            (!tabs.is_empty())
                .then(|| {
                    let n = tabs.len();
                    view! {
                        <div
                            class="ido-tabs"
                            class=(
                                "drop-end",
                                move || {
                                    state.tab_drop.get()
                                        == Some(TabDrop::Strip {
                                            pane: idx,
                                            index: n,
                                        })
                                },
                            )
                            on:dragover=move |ev| {
                                if state.tab_drag.get_untracked().is_some() {
                                    ev.prevent_default();
                                    let index = tab_drop_index(&ev);
                                    state.tab_drop.set(Some(TabDrop::Strip { pane: idx, index }));
                                }
                            }
                            on:drop=move |ev| {
                                ev.prevent_default();
                                state.commit_tab_drop();
                            }
                        >
                            {tabs
                                .into_iter()
                                .enumerate()
                                .map(|(i, tab)| {
                                    view! { <TabItem pane=pane pane_idx=idx idx=i tab=tab /> }
                                })
                                .collect_view()}
                        </div>
                    }
                })
        }}
    }
}

/// A single tab. Click focuses; double-click pins a preview; middle-click / `×`
/// closes; the row is draggable (reorder / move between panes / split).
#[component]
fn TabItem(pane: Pane, pane_idx: usize, idx: usize, tab: Tab) -> impl IntoView {
    let state = expect_context::<State>();
    let icon = tab.target.icon();
    let id = tab.target.entry_id();
    let name = id.rsplit('/').next().unwrap_or(id).to_string();
    view! {
        <div
            class="ido-tab"
            class:active=move || pane.active_tab.get() == Some(idx)
            class=("preview", move || pane.tabs.get().get(idx).map(|t| t.preview).unwrap_or(false))
            class:dragging=move || state.tab_drag.get() == Some((pane_idx, idx))
            class=(
                "drop-before",
                move || {
                    state.tab_drop.get()
                        == Some(TabDrop::Strip {
                            pane: pane_idx,
                            index: idx,
                        })
                },
            )
            draggable="true"
            title=name.clone()
            on:click=move |_| state.activate_tab(pane_idx, idx)
            on:dblclick=move |_| state.pin_in(pane, idx)
            on:contextmenu=move |ev| {
                ev.prevent_default();
                ev.stop_propagation();
                state
                    .open_menu(
                        ev.client_x(),
                        ev.client_y(),
                        MenuTarget::Tab {
                            pane: pane_idx,
                            idx,
                        },
                    );
            }
            on:mousedown=move |ev| {
                if ev.button() == 1 {
                    ev.prevent_default();
                    state.close_tab(pane_idx, idx);
                }
            }
            on:dragstart=move |_| state.start_tab_drag(pane_idx, idx)
            on:dragend=move |_| state.clear_tab_drag()
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
                    state.close_tab(pane_idx, idx);
                }
            >
                <Icon name="x" size=12 />
            </button>
        </div>
    }
}

/// The insertion index for a tab dropped on a strip — the number of tabs whose
/// horizontal midpoint sits left of the cursor (`current_target` is the strip).
fn tab_drop_index(ev: &web_sys::DragEvent) -> usize {
    let Some(strip) = ev
        .current_target()
        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
    else {
        return 0;
    };
    let x = ev.client_x() as f64;
    let Ok(tabs) = strip.query_selector_all(".ido-tab") else {
        return 0;
    };
    let mut index = 0;
    for i in 0..tabs.length() {
        let Some(el) = tabs
            .get(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let rect = el.get_bounding_client_rect();
        if x > rect.left() + rect.width() / 2.0 {
            index += 1;
        } else {
            break;
        }
    }
    index
}
