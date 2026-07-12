//! Source mode: one uncontrolled raw-markdown textarea. See [`SourceEditor`].

use leptos::prelude::*;

use super::attachments::{on_dragover, on_drop, on_paste};
use crate::state::{Buffer, State};

// ══════════════════════════════════════════════════════ Source mode ══════════

/// Full raw-markdown `<textarea>`, uncontrolled. Seeded via the NodeRef stored in
/// `buffer.source_editor` when the element mounts or when a document is opened.
#[component]
pub(crate) fn SourceEditor(buffer: Buffer) -> impl IntoView {
    let state = expect_context::<State>();
    // Populate the textarea the moment it appears in the DOM (mode switch or
    // first open in Source mode). `buffer.source_editor.get()` is tracked, so
    // the effect fires when the element mounts. `content.get_untracked()` avoids
    // re-running (and jumping the cursor) on every subsequent content change.
    Effect::new(move |_| {
        if let Some(ta) = buffer.source_editor.get() {
            ta.set_value(&buffer.content.get_untracked());
            let _ = ta.focus();
        }
    });

    view! {
        <textarea
            class="ido-editor"
            node_ref=buffer.source_editor
            on:input=move |ev| buffer.update_source(event_target_value(&ev))
            on:paste=on_paste(state)
            on:dragover=on_dragover
            on:drop=on_drop(state)
            prop:disabled=move || !buffer.open.get()
            placeholder="open a note or page to begin writing"
        ></textarea>
    }
}
