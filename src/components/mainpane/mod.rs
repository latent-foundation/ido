//! The shared main editing pane: the tab strip, then the active tab's document
//! in one of three modes. Mounted once by the workspace and independent of the
//! section rail/sidebar — it always shows the active tab (a note or a wiki page).
//!
//! ## Three-mode system
//!
//! Three icon buttons in the header switch between modes (`state.mode`):
//!
//! - **Source** (`</>`): single raw-markdown `<textarea>`, uncontrolled — seeded
//!   imperatively on open/switch so the cursor never jumps. Saves on every input.
//! - **Live** (pencil): block live-preview. Each top-level block is rendered as
//!   HTML; clicking it swaps it for a `<textarea>` that auto-grows with content.
//!   **Double-Enter** (blank line) commits the current block and opens the next.
//!   **↑/↓** navigate between blocks when the cursor is at the first/last line.
//!   **Backspace at column 0** merges the block with the one above.
//!   Escape cancels without committing; blur commits (failsafe).
//! - **Reading** (eye): fully rendered, read-only. Link clicks route through
//!   `State::open_link` — `[[wikilinks]]` open the page, others open in the OS
//!   browser; the webview never navigates.
//!
//! The surfaces live in submodules: [`source`] (raw textarea), [`block`] (the
//! live block editor + cursor math), [`reading`] (metadata + backlinks), and
//! [`doc`] (the drawer's compact [`DocEditor`]). This module hosts [`MainPane`].

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use self::block::BlockEditor;
use self::reading::{BacklinksPanel, NoteMetaRow};
use self::source::SourceEditor;
use super::tabs::TabStrip;
use crate::icon::Icon;
use crate::markdown;
use crate::state::{MenuTarget, Mode, Pane, State, TabDrop};

mod attachments;
mod block;
mod doc;
mod reading;
mod source;

pub use self::doc::DocEditor;

/// If `ev`'s target is a rendered task-list checkbox (see `markdown::render`),
/// its document-order index — the argument for `markdown::toggle_checkbox`.
/// Shared by every surface that shows rendered markdown (reading views, live
/// blocks, the drawer editor).
pub(crate) fn checkbox_click_index(ev: &web_sys::MouseEvent) -> Option<usize> {
    let el = ev.target()?.dyn_into::<web_sys::Element>().ok()?;
    if !el.class_list().contains("ido-check") {
        return None;
    }
    el.get_attribute("data-check-idx")?.parse().ok()
}

/// One editor pane: tab strip + editor head (breadcrumbs + mode toggle + split)
/// + body. `pane`/`idx` make it independent, so split view mounts two side by
/// side. Clicking anywhere in the pane focuses it.
#[component]
pub fn MainPane(pane: Pane, idx: usize) -> impl IntoView {
    let state = expect_context::<State>();
    let buffer = state.pane_buffer(pane);
    // The active tab's entry id (note path / wiki slug), for the breadcrumbs.
    let active_id = move || {
        let i = pane.active_tab.get()?;
        pane.tabs
            .get()
            .get(i)
            .map(|t| t.target.entry_id().to_string())
    };
    view! {
        <main
            class="ido-main"
            class:focused=move || state.split.get() && state.focused.get() == idx
            on:mousedown=move |_| state.focus_pane(idx)
        >
            <TabStrip pane=pane idx=idx />
            <div class="ido-editor-head">
                <div class="ido-editor-crumbs">
                    {move || {
                        active_id()
                            .map(|id| {
                                let segs: Vec<String> = id.split('/').map(String::from).collect();
                                let last = segs.len().saturating_sub(1);
                                segs.into_iter()
                                    .enumerate()
                                    .map(|(i, seg)| {
                                        let class = if i == last {
                                            "ido-crumb ido-crumb-leaf"
                                        } else {
                                            "ido-crumb"
                                        };
                                        view! {
                                            {(i > 0)
                                                .then(|| {
                                                    view! { <span class="ido-crumb-sep">"›"</span> }
                                                })}
                                            <span class=class>{seg}</span>
                                        }
                                    })
                                    .collect_view()
                            })
                    }}
                </div>

                // Three-way mode toggle + split toggle + help hint.
                <div class="ido-head-controls">
                    <div class="ido-mode-buttons">
                        <button
                            class="ido-mode-btn"
                            class:active=move || pane.mode.get() == Mode::Source
                            title="Source"
                            on:click=move |_| state.set_mode(pane, Mode::Source)
                        >
                            <Icon name="code" size=14 />
                        </button>
                        <button
                            class="ido-mode-btn"
                            class:active=move || pane.mode.get() == Mode::Live
                            title="Live"
                            on:click=move |_| state.set_mode(pane, Mode::Live)
                        >
                            <Icon name="pencil" size=14 />
                        </button>
                        <button
                            class="ido-mode-btn"
                            class:active=move || pane.mode.get() == Mode::Reading
                            title="Reading"
                            on:click=move |_| state.set_mode(pane, Mode::Reading)
                        >
                            <Icon name="eye" size=14 />
                        </button>
                    </div>
                    <button
                        class="ido-mode-btn ido-split-btn"
                        title=move || if state.split.get() { "close split" } else { "split editor" }
                        on:click=move |_| {
                            if state.split.get_untracked() {
                                state.close_split();
                            } else {
                                state.open_split();
                            }
                        }
                    >
                        <Icon name="columns-2" size=14 />
                    </button>
                    <HelpTip />
                </div>
            </div>

            <div
                class="ido-editor-body"
                on:contextmenu=move |ev| {
                    ev.prevent_default();
                    ev.stop_propagation();
                    state.open_menu(ev.client_x(), ev.client_y(), MenuTarget::Editor { pane: idx });
                }
            >
                {move || match pane.mode.get() {
                    Mode::Source => view! { <SourceEditor buffer=buffer /> }.into_any(),
                    Mode::Live => view! { <BlockEditor buffer=buffer /> }.into_any(),
                    Mode::Reading => {
                        view! {
                            <div class="ido-reading">
                                <NoteMetaRow pane=pane />
                                <div
                                    class="markdown-body"
                                    on:click=move |ev| {
                                        if let Some(i) = checkbox_click_index(&ev) {
                                            ev.prevent_default();
                                            let src = buffer.content.get_untracked();
                                            if let Some(new_src) = markdown::toggle_checkbox(&src, i) {
                                                buffer.update_source(new_src);
                                            }
                                            return;
                                        }
                                        let anchor = ev
                                            .target()
                                            .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                                            .and_then(|el| el.closest("a").ok().flatten());
                                        if let Some(a) = anchor {
                                            if let Some(href) = a.get_attribute("href") {
                                                ev.prevent_default();
                                                state.open_link(href);
                                            }
                                        }
                                    }
                                    inner_html=move || {
                                        markdown::render(
                                            &pane.content.get(),
                                            |id| state.resolve_asset(id),
                                        )
                                    }
                                />
                                <BacklinksPanel pane=pane />
                            </div>
                        }
                            .into_any()
                    }
                }}
                // Drop a dragged tab on the right edge to open a split (only shown
                // while dragging, when not already split).
                {move || {
                    (state.tab_drag.get().is_some() && !state.split.get())
                        .then(|| {
                            view! {
                                <div
                                    class="ido-split-zone"
                                    class:over=move || {
                                        state.tab_drop.get() == Some(TabDrop::NewSplit)
                                    }
                                    on:dragover=move |ev| {
                                        ev.prevent_default();
                                        state.tab_drop.set(Some(TabDrop::NewSplit));
                                    }
                                    on:drop=move |ev| {
                                        ev.prevent_default();
                                        state.commit_tab_drop();
                                    }
                                ></div>
                            }
                        })
                }}
            </div>
        </main>
    }
}

// ══════════════════════════════════════════════════════ Help tooltip ═════════

/// A `?` button that reveals a CSS-only shortcut cheatsheet on hover/focus.
#[component]
fn HelpTip() -> impl IntoView {
    view! {
        <div class="ido-help-wrap">
            <button class="ido-help-btn" aria-label="Editor help">
                <Icon name="circle-help" size=14 />
            </button>
            <div class="ido-help-tip" role="tooltip">
                <div class="ido-help-head">"live mode"</div>
                <dl class="ido-shortcuts">
                    <div>
                        <dt>"click"</dt>
                        <dd>"edit block"</dd>
                    </div>
                    <div>
                        <dt>"↑ / ↓"</dt>
                        <dd>"navigate"</dd>
                    </div>
                    <div>
                        <dt>"↵ on blank line"</dt>
                        <dd>"new block / split"</dd>
                    </div>
                    <div>
                        <dt>"⌫ at start"</dt>
                        <dd>"merge up"</dd>
                    </div>
                    <div>
                        <dt>"esc"</dt>
                        <dd>"cancel"</dd>
                    </div>
                </dl>
            </div>
        </div>
    }
}
