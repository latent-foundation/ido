//! The settings modal — theme, the open well's path, and the task board's
//! columns; a home for preferences as they appear.

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
                                {move || {
                                    state
                                        .well
                                        .get()
                                        .map(|_| {
                                            view! {
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <span class="ido-settings-label">"board columns"</span>
                                                    <input
                                                        class="ido-settings-input"
                                                        prop:value=move || state.columns.get().join(", ")
                                                        spellcheck="false"
                                                        autocomplete="off"
                                                        on:change=move |ev| {
                                                            state.set_columns(event_target_value(&ev))
                                                        }
                                                    />
                                                    <span class="ido-settings-hint">
                                                        "comma-separated, in board order — the last column counts as done. tasks from a removed column drop to the backlog."
                                                    </span>
                                                </div>
                                            }
                                        })
                                }}
                                {move || {
                                    state
                                        .well
                                        .get()
                                        .map(|_| {
                                            view! {
                                                <div class="ido-settings-row ido-settings-row-stack">
                                                    <span class="ido-settings-label">
                                                        "auto-archive done after"
                                                    </span>
                                                    <div class="ido-settings-inline">
                                                        <input
                                                            type="number"
                                                            min="1"
                                                            step="1"
                                                            class="ido-settings-input ido-settings-input-narrow"
                                                            placeholder="off"
                                                            prop:value=move || {
                                                                state
                                                                    .archive_days
                                                                    .get()
                                                                    .map(|d| d.to_string())
                                                                    .unwrap_or_default()
                                                            }
                                                            on:change=move |ev| {
                                                                let raw = event_target_value(&ev);
                                                                let days = raw
                                                                    .trim()
                                                                    .parse::<u32>()
                                                                    .ok()
                                                                    .filter(|d| *d > 0);
                                                                state.set_archive_days(days);
                                                            }
                                                        />
                                                        <span class="ido-settings-suffix">"days"</span>
                                                    </div>
                                                    <span class="ido-settings-hint">
                                                        "done tasks older than this are archived automatically (restorable from the archive view); empty = off."
                                                    </span>
                                                </div>
                                            }
                                        })
                                }}
                            </div>
                        </div>
                    }
                })
        }}
    }
}
