//! The launch screen: create or open a well, or reopen a recent one.

use latent_ui::ThemeToggle;
use leptos::prelude::*;

use crate::state::State;

/// The launcher: brand + tagline, then either the chooser or the create form.
#[component]
pub fn Launch() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <section class="ido-launch">
            <div class="ido-launch-card">
                <div class="ido-brand ido-brand-lg">
                    <span class="ido-wordmark">"ido"</span>
                    <span class="ido-kanji">"井戸"</span>
                </div>
                <p class="ido-launch-tagline">"a quiet well for thought"</p>

                {move || {
                    if state.creating.get() {
                        view! { <CreateForm /> }.into_any()
                    } else {
                        view! { <Chooser /> }.into_any()
                    }
                }}
            </div>
            <div class="ido-launch-foot">
                <ThemeToggle />
            </div>
        </section>
    }
}

/// Create / open buttons and the list of recent wells.
#[component]
fn Chooser() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-launch-actions">
            <button class="ido-btn ido-btn-primary" on:click=move |_| state.start_create()>
                "Create a well…"
            </button>
            <button class="ido-btn" on:click=move |_| state.open_well_picker()>
                "Open a well…"
            </button>
        </div>

        {move || {
            let list = state.recents.get();
            (!list.is_empty())
                .then(move || {
                    view! {
                        <div class="ido-recents">
                            <div class="ido-recents-label">"recent wells"</div>
                            {list
                                .into_iter()
                                .map(|w| {
                                    let name = w.name.clone();
                                    let path = w.path.clone();
                                    view! {
                                        <button
                                            class="ido-recent"
                                            on:click=move |_| state.enter_well(w.clone())
                                        >
                                            <span class="ido-recent-name">{name}</span>
                                            <span class="ido-recent-path">{path}</span>
                                        </button>
                                    }
                                })
                                .collect_view()}
                        </div>
                    }
                })
        }}
    }
}

/// The create-a-well form: a name plus a chosen parent location.
#[component]
fn CreateForm() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <div class="ido-create">
            <input
                class="ido-input"
                placeholder="well name"
                autofocus
                on:input=move |ev| state.new_name.set(event_target_value(&ev))
            />
            <button class="ido-btn ido-btn-location" on:click=move |_| state.choose_location()>
                {move || state.new_parent.get().unwrap_or_else(|| "Choose location…".to_string())}
            </button>
            <div class="ido-create-row">
                <button class="ido-btn" on:click=move |_| state.cancel_create()>
                    "Cancel"
                </button>
                <button
                    class="ido-btn ido-btn-primary"
                    on:click=move |_| state.confirm_create()
                    prop:disabled=move || {
                        state.new_parent.get().is_none() || state.new_name.get().trim().is_empty()
                    }
                >
                    "Create"
                </button>
            </div>
        </div>
    }
}
