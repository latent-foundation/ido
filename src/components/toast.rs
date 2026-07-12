//! A transient toast (bottom-centre) for delete-undo and error messages.

use leptos::prelude::*;

use crate::icon::Icon;
use crate::state::State;

/// Shows `state.toast` while set; "undo" restores a soft-deleted note / page.
/// Mounted once in the workspace.
#[component]
pub fn ToastBar() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        {move || {
            state
                .toast
                .get()
                .map(|t| {
                    let has_undo = t.undo.is_some();
                    view! {
                        <div class="ido-toast">
                            <span class="ido-toast-msg">{t.message}</span>
                            {has_undo
                                .then(|| {
                                    view! {
                                        <button
                                            class="ido-toast-undo"
                                            on:click=move |_| state.undo_toast()
                                        >
                                            "undo"
                                        </button>
                                    }
                                })}
                            <button
                                class="ido-toast-close"
                                title="dismiss"
                                on:click=move |_| state.dismiss_toast()
                            >
                                <Icon name="x" size=13 />
                            </button>
                        </div>
                    }
                })
        }}
    }
}
