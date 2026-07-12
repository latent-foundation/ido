//! Image attachments: pulling image files out of a paste/drop `DataTransfer`,
//! saving each into the well's shared `assets/` folder, and splicing a
//! `![](assets/…)` reference into the editor at the cursor.
//!
//! [`on_paste`] / [`on_dragover`] / [`on_drop`] build the `on:` handlers shared
//! by every markdown textarea — [`super::source::SourceEditor`] and the block
//! textareas in [`super::block`] (and, through those shared components, the
//! task/goal drawer's `DocEditor`).

use leptos::prelude::*;
use leptos::task::spawn_local;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;
use web_sys::{ClipboardEvent, DataTransfer, DragEvent, File, HtmlTextAreaElement};

use crate::components::contextmenu::document_fn;
use crate::state::State;

/// Image files carried by a paste/drop `DataTransfer`, in order. A paste
/// usually carries at most one; a drop from the OS file manager may carry many.
/// `None` when there are none, so callers can fall through to normal handling.
fn images_in(dt: &DataTransfer) -> Option<Vec<File>> {
    let mut out = Vec::new();
    if let Some(files) = dt.files() {
        for i in 0..files.length() {
            if let Some(file) = files.get(i) {
                if file.type_().starts_with("image/") {
                    out.push(file);
                }
            }
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The `HtmlTextAreaElement` an event targeted, if any.
fn target_textarea(ev: &web_sys::Event) -> Option<HtmlTextAreaElement> {
    ev.target()?.dyn_into::<HtmlTextAreaElement>().ok()
}

/// Read `file`'s bytes (via `Blob::array_buffer`); empty on failure.
async fn read_bytes(file: &File) -> Vec<u8> {
    match JsFuture::from(file.array_buffer()).await {
        Ok(buf) => js_sys::Uint8Array::new(&buf).to_vec(),
        Err(_) => Vec::new(),
    }
}

/// Insert `text` at the focused element's cursor via `execCommand('insertText',
/// …)` — the same reflection trick as the context menu's paste action
/// (`contextmenu::document_fn`, promoted `pub(crate)` for this reuse). Keeps the
/// cursor in place and fires a real `input` event, so Source mode's autosave
/// picks it up without any extra propagation; Live-mode block textareas read the
/// live DOM value on their next commit (blur/Enter/navigate) regardless.
fn insert_text(text: &str) {
    if let Some((doc, f)) = document_fn("execCommand") {
        let _ = f.call3(
            &doc,
            &JsValue::from_str("insertText"),
            &JsValue::FALSE,
            &JsValue::from_str(text),
        );
    }
}

/// Save each of `files` into `well`'s `assets/` folder and insert a `![](…)`
/// reference for each at `ta`'s cursor, one per line. `ta` is (re-)focused
/// before every insert, since an `await` between saves could otherwise lose
/// focus to something else (e.g. a toast).
async fn insert_images(well: String, files: Vec<File>, ta: HtmlTextAreaElement) {
    for file in files {
        let name = file.name();
        let bytes = read_bytes(&file).await;
        if let Some(id) = crate::ipc::save_asset(well.clone(), name, bytes).await {
            let _ = ta.focus();
            insert_text(&format!("![]({id})\n"));
        }
    }
}

/// `on:paste` handler shared by every markdown textarea: extract image files
/// from the clipboard, save each into the well's `assets/` folder, and splice a
/// `![](…)` reference in. Falls through to the browser's normal text paste when
/// the clipboard carries no image.
pub(super) fn on_paste(state: State) -> impl Fn(ClipboardEvent) + 'static {
    move |ev: ClipboardEvent| {
        let Some(dt) = ev.clipboard_data() else {
            return;
        };
        let Some(files) = images_in(&dt) else {
            return;
        };
        let (Some(well), Some(ta)) = (state.well.get_untracked(), target_textarea(&ev)) else {
            return;
        };
        ev.prevent_default();
        spawn_local(insert_images(well.path, files, ta));
    }
}

/// `on:dragover` handler shared by every markdown textarea — must call
/// `prevent_default` for the element to accept a subsequent `drop` at all.
pub(super) fn on_dragover(ev: DragEvent) {
    ev.prevent_default();
}

/// `on:drop` handler shared by every markdown textarea. Always prevents the
/// browser's default file-drop behaviour first (which would otherwise insert
/// the raw file path as text — see `workspace`'s global fallback for drops
/// outside any editor), then saves + splices in any images found.
pub(super) fn on_drop(state: State) -> impl Fn(DragEvent) + 'static {
    move |ev: DragEvent| {
        ev.prevent_default();
        let Some(dt) = ev.data_transfer() else {
            return;
        };
        let Some(files) = images_in(&dt) else {
            return;
        };
        let (Some(well), Some(ta)) = (state.well.get_untracked(), target_textarea(&ev)) else {
            return;
        };
        spawn_local(insert_images(well.path, files, ta));
    }
}
