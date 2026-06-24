//! The settings modal — currently theme and the open well's path; a home for
//! preferences as they appear.

use latent_ui::ThemeToggle;
use leptos::prelude::*;

use crate::icon::Icon;
use crate::state::State;

/// A centred modal over the app, shown while `settings_open` is set. Clicking
/// the backdrop (or close) dismisses it; clicking the panel does not.
#[component]
pub fn Settings() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            state
                .settings_open
                .get()
                .then(|| {
                    view! {
                        <div
                            class="ido-modal-backdrop"
                            on:click=move |_| state.settings_open.set(false)
                        >
                            <div class="ido-modal" on:click=move |ev| ev.stop_propagation()>
                                <div class="ido-modal-head">
                                    <span class="ido-modal-title">"settings"</span>
                                    <button
                                        class="ido-modal-close"
                                        on:click=move |_| state.settings_open.set(false)
                                    >
                                        <Icon name="x" size=15 />
                                    </button>
                                </div>
                                <div class="ido-settings-row">
                                    <span class="ido-settings-label">"appearance"</span>
                                    <ThemeToggle />
                                </div>
                                <div class="ido-settings-row">
                                    <span class="ido-settings-label">"well"</span>
                                    <span class="ido-settings-value">
                                        {move || {
                                            state.well.get().map(|w| w.path).unwrap_or_default()
                                        }}
                                    </span>
                                </div>
                            </div>
                        </div>
                    }
                })
        }}
    }
}
