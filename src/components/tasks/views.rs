//! The board toolbar's **saved views** dropdown: a snapshot of the six toolbar
//! controls (view / filter / tag / goal / hide-done / sort), stored per-well in
//! `.ido/well.toml`. The [`ViewsMenu`] button opens a popover listing the saved
//! views (click a name to apply it, `×` to delete) plus a "save current…" input.
//!
//! Popover conventions mirror the datepicker ([`crate::components::datepicker`]):
//! an outside-mousedown closes it, and Escape closes just the popover. Escape is
//! handled on the widget root (`.ido-views`) so it fires wherever focus sits
//! inside the widget (trigger, list, or input) and `stop_propagation`s **only
//! while open** — beating the workspace's window-level Escape (which would
//! otherwise close a drawer behind it), per the app's layered-Esc rule. A
//! non-empty input clears on the first Escape and the popover closes on the
//! second.

use leptos::prelude::*;
use wasm_bindgen::JsCast;

use crate::icon::Icon;
use crate::state::State;

/// The toolbar's "views" dropdown button + its popover.
#[component]
pub fn ViewsMenu() -> impl IntoView {
    let state = expect_context::<State>();
    let open = RwSignal::new(false);
    let root: NodeRef<leptos::html::Div> = NodeRef::new();
    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();

    // Close + reset the save input.
    let close = move || {
        open.set(false);
        if let Some(el) = input_ref.get_untracked() {
            el.set_value("");
        }
    };

    // Save whatever is typed (if anything) and close.
    let submit = move || {
        let Some(el) = input_ref.get_untracked() else {
            return;
        };
        let name = el.value();
        if !name.trim().is_empty() {
            state.save_view(name);
        }
        close();
    };

    // Outside-mousedown closes the popover (datepicker pattern).
    let handle = window_event_listener(leptos::ev::mousedown, move |ev| {
        if !open.get_untracked() {
            return;
        }
        let inside = root
            .get_untracked()
            .and_then(|el| {
                ev.target()
                    .and_then(|t| t.dyn_into::<web_sys::Node>().ok())
                    .map(|n| el.contains(Some(&n)))
            })
            .unwrap_or(false);
        if !inside {
            open.set(false);
        }
    });
    on_cleanup(move || handle.remove());

    view! {
        <div
            class="ido-views"
            node_ref=root
            on:keydown=move |ev| {
                if ev.key() != "Escape" || !open.get_untracked() {
                    return;
                }
                ev.stop_propagation();
                let typed = input_ref
                    .get_untracked()
                    .map(|el| !el.value().is_empty())
                    .unwrap_or(false);
                if typed {
                    if let Some(el) = input_ref.get_untracked() {
                        el.set_value("");
                    }
                } else {
                    close();
                }
            }
        >
            <button
                class="ido-views-btn"
                class:active=move || open.get()
                title="Saved views"
                on:click=move |_| open.update(|o| *o = !*o)
            >
                <Icon name="bookmark" size=13 />
                "views"
            </button>

            {move || {
                open.get()
                    .then(|| {
                        view! {
                            <div class="ido-views-pop">
                                <div class="ido-views-list">
                                    {move || {
                                        let views = state.saved_views.get();
                                        if views.is_empty() {
                                            view! {
                                                <div class="ido-views-empty">"no saved views yet"</div>
                                            }
                                                .into_any()
                                        } else {
                                            views
                                                .into_iter()
                                                .map(|v| {
                                                    let apply = v.clone();
                                                    let name_del = v.name.clone();
                                                    view! {
                                                        <div class="ido-views-row">
                                                            <button
                                                                class="ido-views-apply"
                                                                on:click=move |_| {
                                                                    state.apply_view(apply.clone());
                                                                    close();
                                                                }
                                                            >
                                                                {v.name.clone()}
                                                            </button>
                                                            <button
                                                                class="ido-views-del"
                                                                title="Delete view"
                                                                on:click=move |ev| {
                                                                    ev.stop_propagation();
                                                                    state.delete_view(name_del.clone());
                                                                }
                                                            >
                                                                <Icon name="x" size=12 />
                                                            </button>
                                                        </div>
                                                    }
                                                })
                                                .collect_view()
                                                .into_any()
                                        }
                                    }}
                                </div>
                                <div class="ido-views-save">
                                    <input
                                        class="ido-views-input"
                                        node_ref=input_ref
                                        placeholder="save current…"
                                        spellcheck="false"
                                        autocomplete="off"
                                        on:keydown=move |ev| {
                                            if ev.key() == "Enter" {
                                                submit();
                                            }
                                        }
                                    />
                                </div>
                            </div>
                        }
                    })
            }}
        </div>
    }
}
