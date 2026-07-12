//! The drawer's self-contained three-mode editor. See [`DocEditor`].

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use super::block::BlockEditor;
use super::source::SourceEditor;
use crate::icon::Icon;
use crate::markdown;
use crate::state::{Buffer, Mode, State};

// ═══════════════════════════════════════════════════════ Drawer editor ════════

/// A self-contained three-mode editor for a [`Buffer`] — used by the task and
/// goal **drawers**, where there's no tab strip / breadcrumbs / backlinks. Same
/// source / live / reading surfaces as the main pane, with a compact toggle and
/// a local `mode` signal. Reading shows plain rendered markdown (links routed
/// through `State::open_link` like the main pane).
#[component]
pub fn DocEditor(buffer: Buffer, mode: RwSignal<Mode>) -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-doc-editor">
            <div class="ido-doc-modes">
                <button
                    class="ido-mode-btn"
                    class:active=move || mode.get() == Mode::Source
                    title="Source"
                    on:click=move |_| {
                        buffer.active_block.set(None);
                        mode.set(Mode::Source);
                    }
                >
                    <Icon name="code" size=13 />
                </button>
                <button
                    class="ido-mode-btn"
                    class:active=move || mode.get() == Mode::Live
                    title="Live"
                    on:click=move |_| {
                        buffer.active_block.set(None);
                        mode.set(Mode::Live);
                    }
                >
                    <Icon name="pencil" size=13 />
                </button>
                <button
                    class="ido-mode-btn"
                    class:active=move || mode.get() == Mode::Reading
                    title="Reading"
                    on:click=move |_| {
                        buffer.active_block.set(None);
                        mode.set(Mode::Reading);
                    }
                >
                    <Icon name="eye" size=13 />
                </button>
            </div>
            <div class="ido-doc-body">
                {move || match mode.get() {
                    Mode::Source => view! { <SourceEditor buffer=buffer /> }.into_any(),
                    Mode::Live => view! { <BlockEditor buffer=buffer /> }.into_any(),
                    Mode::Reading => {
                        view! {
                            <div
                                class="markdown-body ido-doc-reading"
                                on:click=move |ev| {
                                    if let Some(i) = super::checkbox_click_index(&ev) {
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
                                        &buffer.content.get(),
                                        |id| state.resolve_asset(id),
                                    )
                                }
                            />
                        }
                            .into_any()
                    }
                }}
            </div>
        </div>
    }
}
