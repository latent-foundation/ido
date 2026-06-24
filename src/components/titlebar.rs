//! The custom window title bar (native decorations are off): a drag region with
//! minimize / maximize / close controls.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::state::State;

/// The title bar. The bar itself is a `data-tauri-drag-region` (drag to move the
/// window); the controls call Rust window commands. Maximize only shows in the
/// editor, where the window is resizable.
#[component]
pub fn TitleBar() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-titlebar" data-tauri-drag-region="">
            <div class="ido-titlebar-controls">
                <button
                    class="ido-win-btn"
                    title="Minimize"
                    on:click=move |_| state.window_cmd("win_minimize")
                >
                    <Icon name="minus" size=15 />
                </button>
                {move || {
                    state
                        .well
                        .get()
                        .is_some()
                        .then(|| {
                            view! {
                                <button
                                    class="ido-win-btn"
                                    title="Maximize"
                                    on:click=move |_| state.window_cmd("win_toggle_maximize")
                                >
                                    <Icon name="square" size=13 />
                                </button>
                            }
                        })
                }}
                <button
                    class="ido-win-btn ido-win-close"
                    title="Close"
                    on:click=move |_| state.window_cmd("win_close")
                >
                    <Icon name="x" size=15 />
                </button>
            </div>
        </div>
    }
}
