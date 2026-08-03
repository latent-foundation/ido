//! The notes section sidebar: a header, the new-note/folder toolbar, and the
//! recursive folder tree. The editing surface is the shared
//! [`crate::components::mainpane`]; this contributes only the left sidebar.

use latent_ui::Icon;
use leptos::prelude::*;

use crate::components::tree::Tree;
use crate::state::State;

/// The notes sidebar (`<aside>`), shown when the notes section is active.
#[component]
pub fn NotesSidebar() -> impl IntoView {
    let state = expect_context::<State>();
    view! {
        <aside class="ido-sidebar">
            <div class="ido-section-head">
                <Icon name="file-text" size=14 />
                <span class="ido-section-label">"notes"</span>
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
        </aside>
    }
}
