//! The in-well workspace shell: the section rail, the active section's sidebar,
//! and the shared main editing pane.
//!
//! The rail/sidebar are for *browsing* (which section you're in); the main pane
//! is for *editing* (the active tab) and is independent of the section — so a
//! note and a wiki page can be open together regardless of which sidebar shows.
//! Tasks replaces the sidebar + pane with a full-width board (its editing is a
//! drawer, not the main pane).

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::components::contextmenu::ContextMenu;
use crate::components::mainpane::MainPane;
use crate::components::notes::NotesSidebar;
use crate::components::rail::Rail;
use crate::components::search::SearchPalette;
use crate::components::tasks::TaskBoard;
use crate::components::toast::ToastBar;
use crate::components::wiki::WikiSidebar;
use crate::state::{Section, State};

/// Rail + section sidebar + main pane. Mounted by `App` whenever a well is open.
#[component]
pub fn Workspace() -> impl IntoView {
    let state = expect_context::<State>();

    // Global tab shortcuts: Ctrl+Tab / Ctrl+Shift+Tab cycle, Ctrl+1…9 jump,
    // Ctrl+W close. Gated on Ctrl (or Cmd on macOS), so they never clash with
    // the block editor's bare-key model inside a textarea. The listener is
    // removed when the workspace unmounts (leaving the well).
    window_event_listener(leptos::ev::keydown, move |ev| {
        if !(ev.ctrl_key() || ev.meta_key()) {
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
                let f = state.focused.get_untracked();
                if let Some(i) = state.cur().active_tab.get_untracked() {
                    state.close_tab(f, i);
                }
            }
            "k" | "K" => {
                ev.prevent_default();
                state.open_search();
            }
            "\\" => {
                ev.prevent_default();
                if state.split.get_untracked() {
                    state.close_split();
                } else {
                    state.open_split();
                }
            }
            key => {
                if let Some(n) = (key.len() == 1)
                    .then(|| key.chars().next().and_then(|c| c.to_digit(10)))
                    .flatten()
                    .filter(|&n| n >= 1)
                {
                    let idx = (n - 1) as usize;
                    if idx < state.cur().tabs.get_untracked().len() {
                        ev.prevent_default();
                        state.activate_tab(state.focused.get_untracked(), idx);
                    }
                }
            }
        }
    });

    // Bare Escape: close the active task/goal drawer — but only once nothing
    // more local has already claimed the key. Layered, in order:
    //  1. the search palette and the context menu are `State` signals
    //     (`search_open`, `menu`); they self-handle Escape (their own
    //     handlers `stop_propagation`/check the signal directly), so this
    //     just yields when either is set.
    //  2. the calendar's day/overdue popovers and the datepicker popover are
    //     component-local signals invisible to `State` — detected instead by
    //     DOM presence, which is order-independent (unlike relying on
    //     window-listener registration order across components).
    //  3. the Live block editor's own Escape (deactivate a block, see
    //     `mainpane/block.rs`) `stop_propagation`s, so an active block edit
    //     never reaches this handler at all: one Esc leaves the block, a
    //     second (nothing left active) closes the drawer.
    // Not Ctrl-gated — Escape has no modifier form — but that's safe here
    // since every self-handling surface above owns its Escape by stopping
    // propagation or being checked first, and a drawer field input has no
    // Escape handler of its own, so Esc there falls through and closes the
    // drawer (fields commit on `change`, so nothing is lost).
    window_event_listener(leptos::ev::keydown, move |ev| {
        if ev.key() != "Escape" {
            return;
        }
        if state.search_open.get_untracked() || state.menu.get_untracked().is_some() {
            return;
        }
        let popover_open = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| {
                d.query_selector(".ido-cal-pop, .ido-cal-overdue-pop, .ido-date-pop")
                    .ok()
                    .flatten()
            })
            .is_some();
        if popover_open {
            return;
        }
        if state.active_task.get_untracked().is_some() {
            state.close_task();
        } else if state.active_goal.get_untracked().is_some() {
            state.close_goal();
        }
    });

    // Suppress the native webview context menu, except over text editors and
    // rendered prose (where select / copy / paste are wanted). ido's own menus
    // open from each surface's `on:contextmenu` instead.
    window_event_listener(leptos::ev::contextmenu, move |ev| {
        let allow = ev
            .target()
            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
            .map(|el| {
                let tag = el.tag_name();
                tag == "TEXTAREA"
                    || tag == "INPUT"
                    || el.closest(".markdown-body").ok().flatten().is_some()
            })
            .unwrap_or(false);
        if !allow {
            ev.prevent_default();
        }
    });

    // Prevent the OS's default "navigate to the dropped file" behaviour
    // anywhere a drag/drop isn't explicitly handled — the markdown editors call
    // `prevent_default` themselves for image drops (see `mainpane::attachments`);
    // this is the fallback for everywhere else (sidebar, board, rail, …), since
    // an unhandled file drop would otherwise navigate the webview and
    // white-screen the app.
    window_event_listener(leptos::ev::dragover, |ev| ev.prevent_default());
    window_event_listener(leptos::ev::drop, |ev| ev.prevent_default());

    // Remember the editor window's size when the user resizes it (debounced —
    // resize fires rapidly during a drag). The workspace is only mounted while a
    // well is open, i.e. the resizable editor window.
    let resize_gen = RwSignal::new(0u32);
    window_event_listener(leptos::ev::resize, move |_| {
        let g = resize_gen.get_untracked().wrapping_add(1);
        resize_gen.set(g);
        set_timeout(
            move || {
                if resize_gen.get_untracked() == g {
                    state.remember_window();
                }
            },
            std::time::Duration::from_millis(500),
        );
    });

    // Notes/wiki: section sidebar + the editor pane(s). Tasks: a full-width
    // board (its own editing happens in a drawer, not the main pane).
    view! {
        <Rail />
        {move || match state.section.get() {
            Section::Notes => {
                view! {
                    <NotesSidebar />
                    <EditorPanes />
                }
                    .into_any()
            }
            Section::Wiki => {
                view! {
                    <WikiSidebar />
                    <EditorPanes />
                }
                    .into_any()
            }
            Section::Tasks => view! { <TaskBoard /> }.into_any(),
        }}
        <SearchPalette />
        <ToastBar />
        <ContextMenu />
    }
}

/// The editor area: the primary pane, plus the secondary pane when split.
#[component]
fn EditorPanes() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <MainPane pane=state.panes[0] idx=0 />
        {move || { state.split.get().then(|| view! { <MainPane pane=state.panes[1] idx=1 /> }) }}
    }
}
