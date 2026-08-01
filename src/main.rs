//! ido frontend — a Leptos (CSR → WASM) desktop UI over the Tauri backend.
//!
//! Layout:
//! - [`model`] — data shapes shared with the backend
//! - [`ipc`] — the single bridge to Tauri (typed wrappers over `invoke`)
//! - [`state`] — reactive [`state::State`] + action methods, shared via context
//! - [`blocks`] — markdown block segmentation for the live editor
//! - [`markdown`] — markdown → HTML for per-block and reading-view rendering
//! - [`dates`] — shared date math (parsing, formatting, calendar arithmetic)
//! - [`icon`] — inline icons
//! - [`platform`] — which OS we're running on (the title bar follows it)
//! - [`components`] — the title bar, launcher, editor, tree, and settings
//! - [`app`] — the root that composes them

mod app;
mod blocks;
mod components;
mod dates;
mod icon;
mod ipc;
mod markdown;
mod model;
mod platform;
mod state;

use app::App;
use leptos::prelude::*;

fn main() {
    console_error_panic_hook::set_once();
    mount_to_body(|| {
        view! { <App /> }
    })
}
