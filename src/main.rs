//! ido frontend — a Leptos (CSR → WASM) desktop UI over the Tauri backend.
//!
//! Layout:
//! - [`model`] — data shapes shared with the backend
//! - [`ipc`] — the single bridge to Tauri (typed wrappers over `invoke`)
//! - [`state`] — reactive [`state::State`] + action methods, shared via context
//! - [`icon`] — inline icons
//! - [`components`] — the title bar, launcher, editor, tree, and settings
//! - [`app`] — the root that composes them

mod app;
mod components;
mod icon;
mod ipc;
mod model;
mod state;

use app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! { <App /> }
    })
}
