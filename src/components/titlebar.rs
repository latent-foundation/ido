//! The custom window title bar (native decorations are off): a drag region with
//! minimize / maximize / close controls.
//!
//! The controls follow the host OS. On macOS they are three dots on the left,
//! where that platform puts them — drawn in latent's palette rather than the
//! system red/amber/green, and revealing their glyphs on hover of the cluster.
//! Everywhere else they stay a right-hand minimize / maximize / close row.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::platform::is_mac;
use crate::state::State;

/// The title bar. The bar itself is a `data-tauri-drag-region` (drag to move the
/// window); the controls call Rust window commands. The open well's name sits
/// alongside them, visible across every section.
#[component]
pub fn TitleBar() -> impl IntoView {
    let mac = is_mac();
    view! {
        <div class="ido-titlebar" class:mac=mac data-tauri-drag-region="">
            {mac.then(|| view! { <MacLights /> })}
            <WellName />
            {(!mac).then(|| view! { <WinControls /> })}
        </div>
    }
}

/// The open well's name — quiet metadata, centred on macOS (that platform's
/// title convention) and leading everywhere else.
#[component]
fn WellName() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <span class="ido-title-well" data-tauri-drag-region="">
            {move || state.well.get().map(|w| w.name).unwrap_or_default()}
        </span>
    }
}

/// macOS: close / minimize / fullscreen as three dots on the left, in that
/// order. Fullscreen is disabled on the launcher, where the window is a fixed
/// size — greyed rather than removed, so the cluster keeps its shape.
#[component]
fn MacLights() -> impl IntoView {
    let state = expect_context::<State>();
    let no_well = move || state.well.get().is_none();
    view! {
        <div class="ido-mac-lights">
            <button
                class="ido-mac-dot ido-mac-close"
                title="Close"
                on:click=move |_| state.window_cmd("win_close")
            >
                <Icon name="x" size=8 />
            </button>
            <button
                class="ido-mac-dot"
                title="Minimize"
                on:click=move |_| state.window_cmd("win_minimize")
            >
                <Icon name="minus" size=8 />
            </button>
            <button
                class="ido-mac-dot"
                title="Full Screen"
                disabled=no_well
                on:click=move |_| state.window_cmd("win_toggle_maximize")
            >
                <Icon name="plus" size=8 />
            </button>
        </div>
    }
}

/// Windows / Linux: minimize / maximize / close on the right. Maximize only
/// shows in the editor, where the window is resizable.
#[component]
fn WinControls() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
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
    }
}
