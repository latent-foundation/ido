//! The application root: set up theme + state, then compose the title bar, the
//! active screen (launcher or editor), and the settings overlay.

use latent_ui::theme::{initial_theme, setup_theme_effect};
use leptos::prelude::*;

use crate::components::editor::Editor;
use crate::components::launch::Launch;
use crate::components::settings::Settings;
use crate::components::titlebar::TitleBar;
use crate::state::State;

/// The root component. Provides the theme signal (read by `ThemeToggle`) and the
/// shared [`State`] via context, then gates the launcher vs the editor on
/// whether a well is open.
#[component]
pub fn App() -> impl IntoView {
    // Theme: one RwSignal<bool> in context, consumed by `ThemeToggle`.
    let is_dark = RwSignal::new(initial_theme());
    provide_context(is_dark);
    setup_theme_effect(is_dark);

    // App state in context; kick off startup (reopen last well, reveal window).
    let state = State::new();
    provide_context(state);
    state.start();

    view! {
        <div class="ido-shell">
            <TitleBar />
            <div class="ido-app">
                {move || {
                    if state.well.get().is_none() {
                        view! { <Launch /> }.into_any()
                    } else {
                        view! { <Editor /> }.into_any()
                    }
                }}
            </div>
            <Settings />
        </div>
    }
}
