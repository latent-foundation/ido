//! The task drawer's tags field: a plain text input plus a suggestion popover
//! (datepicker-style — no native `<datalist>`, un-themeable) filtered by
//! prefix as you type after the last comma.
//!
//! [`TagsInput`] preserves the plain input it replaces exactly: it's
//! **uncontrolled**, seeded once from `value`, and commits the full
//! comma-separated field via `on_commit` on `change`/blur — unchanged from
//! today (see `drawers.rs`'s module doc on why fields stay uncontrolled).
//! Accepting a suggestion only rewrites the fragment being typed; it never
//! commits early.

use leptos::prelude::*;

use super::logic::{accept_suggestion, split_fragment, tag_suggestions, tag_vocabulary};
use crate::state::State;

/// The drawer's tags input. `value` seeds the uncontrolled `<input>` once;
/// `on_commit` fires on `change`/blur with the full field, exactly like the
/// plain input this replaces.
#[component]
pub(super) fn TagsInput(value: String, on_commit: Callback<String>) -> impl IntoView {
    let state = expect_context::<State>();
    // Case-insensitively deduped, first-seen casing, non-archived tasks only
    // (`tag_vocabulary`); a `Memo` so it only recomputes when `state.tasks`
    // actually changes, not on every keystroke.
    let vocabulary = Memo::new(move |_| tag_vocabulary(&state.tasks.get()));

    let input_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    let suggestions = RwSignal::new(Vec::<String>::new());
    let active = RwSignal::new(0usize);
    // The popover shows exactly when there's a suggestion list to show —
    // empty (including an empty typed fragment) means closed.
    let open = Signal::derive(move || !suggestions.get().is_empty());

    // Recompute the suggestion list from the field's current raw text.
    let recompute = move |text: &str| {
        let (existing, fragment) = split_fragment(text);
        let list = tag_suggestions(&vocabulary.get_untracked(), &fragment, &existing);
        active.set(0);
        suggestions.set(list);
    };

    // Replace the fragment being typed with `canonical`'s casing: write the
    // result straight into the DOM (uncontrolled — the same imperative
    // `set_value` idiom the column quick-add uses), keep focus + move the
    // caret to the end, and close the popover. Deliberately does **not**
    // call `on_commit` — commit stays on change/blur, unchanged from today.
    let accept = move |canonical: String| {
        let Some(el) = input_ref.get_untracked() else {
            return;
        };
        let new_value = accept_suggestion(&el.value(), &canonical);
        el.set_value(&new_value);
        let _ = el.focus();
        let len = new_value.encode_utf16().count() as u32;
        let _ = el.set_selection_range(len, len);
        suggestions.set(Vec::new());
    };

    view! {
        <div class="ido-tags-field">
            <input
                class="ido-drawer-input"
                node_ref=input_ref
                prop:value=value
                placeholder="comma, separated"
                spellcheck="false"
                autocomplete="off"
                on:input=move |ev| recompute(&event_target_value(&ev))
                on:change=move |ev| on_commit.run(event_target_value(&ev))
                on:blur=move |_| suggestions.set(Vec::new())
                on:keydown=move |ev| {
                    if !open.get_untracked() {
                        return;
                    }
                    match ev.key().as_str() {
                        "Escape" => {
                            ev.stop_propagation();
                            suggestions.set(Vec::new());
                        }
                        "ArrowDown" => {
                            ev.prevent_default();
                            let n = suggestions.get_untracked().len();
                            active.update(|a| *a = (*a + 1) % n);
                        }
                        "ArrowUp" => {
                            ev.prevent_default();
                            let n = suggestions.get_untracked().len();
                            active.update(|a| *a = (*a + n - 1) % n);
                        }
                        "Enter" | "Tab" => {
                            let list = suggestions.get_untracked();
                            if let Some(pick) = list.get(active.get_untracked()).cloned() {
                                ev.prevent_default();
                                ev.stop_propagation();
                                accept(pick);
                            }
                        }
                        _ => {}
                    }
                }
            />
            {move || {
                open.get()
                    .then(|| {
                        view! {
                            <div class="ido-tags-pop">
                                {suggestions
                                    .get()
                                    .into_iter()
                                    .enumerate()
                                    .map(|(i, tag)| {
                                        let tag_click = tag.clone();
                                        view! {
                                            <button
                                                type="button"
                                                class="ido-tags-item"
                                                class:active=move || active.get() == i
                                                // The blur race: a plain click on this
                                                // button would blur the input *first*
                                                // (firing `on:change` with the half-typed
                                                // fragment) before the click handler ever
                                                // ran. `prevent_default` on `mousedown` —
                                                // the event that actually triggers the
                                                // browser's default focus-shift — stops
                                                // that shift from happening at all, so the
                                                // input never blurs and `change` never
                                                // fires early; doing the insert here (on
                                                // `mousedown`, not `click`) is what makes
                                                // that safe to rely on.
                                                on:mousedown=move |ev| {
                                                    ev.prevent_default();
                                                    accept(tag_click.clone());
                                                }
                                            >
                                                {tag}
                                            </button>
                                        }
                                    })
                                    .collect_view()}
                            </div>
                        }
                    })
            }}
        </div>
    }
}
