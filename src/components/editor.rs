//! The editor screen: the note-tree sidebar and the markdown textarea.

use leptos::prelude::*;

use crate::components::tree::Tree;
use crate::icon::Icon;
use crate::state::State;

/// Sidebar (brand + well switch, create toolbar, tree, settings) plus the
/// editing surface. Two siblings under `.ido-app` (a flex row).
#[component]
pub fn Editor() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <aside class="ido-sidebar">
            <div class="ido-brand">
                <button
                    class="ido-well-switch"
                    title="Switch well"
                    on:click=move |_| state.leave_well()
                >
                    <span class="ido-wordmark">"ido"</span>
                </button>
                <span class="ido-well-name">
                    {move || state.well.get().map(|w| w.name).unwrap_or_default()}
                </span>
            </div>

            <div class="ido-toolbar">
                <button
                    class="ido-tool"
                    title="New note"
                    on:click=move |_| state.add_note(state.target.get_untracked())
                >
                    <Icon name="file-plus" size=15 />
                </button>
                <button
                    class="ido-tool"
                    title="New folder"
                    on:click=move |_| state.add_folder(state.target.get_untracked())
                >
                    <Icon name="folder-plus" size=15 />
                </button>
                <button
                    class="ido-tool-target"
                    title="New items go here — click for the well root"
                    on:click=move |_| state.target.set(String::new())
                >
                    {move || {
                        let t = state.target.get();
                        if t.is_empty() {
                            "root".to_string()
                        } else {
                            t.rsplit('/').next().unwrap_or(&t).to_string()
                        }
                    }}
                </button>
            </div>

            <nav
                class="ido-list"
                on:dragover=move |ev| {
                    ev.prevent_default();
                    state.drag_over.set(None);
                }
                on:drop=move |ev| {
                    ev.prevent_default();
                    state.drag_over.set(None);
                    state.move_into(String::new());
                }
            >
                {move || view! { <Tree nodes=state.tree.get() depth=0 /> }}
            </nav>

            <div class="ido-status">
                <button class="ido-settings-btn" on:click=move |_| state.settings_open.set(true)>
                    <Icon name="settings" size=15 />
                    "settings"
                </button>
            </div>
        </aside>

        <main class="ido-main">
            <textarea
                class="ido-editor"
                node_ref=state.editor
                on:input=move |ev| state.save_note(event_target_value(&ev))
                prop:disabled=move || state.active.get().is_none()
                placeholder="Select a note, or create one to begin writing…"
            ></textarea>
        </main>
    }
}
