//! The single bridge to the Tauri backend.
//!
//! Everything that touches the backend goes through here: the raw `invoke`
//! binding, the argument structs, and one typed wrapper per command. UI/state
//! code calls the wrappers and never serialises arguments itself. Errors are
//! folded into the return type (`Option`/`Vec`/`bool`/empty), matching the
//! app's fire-and-forget style.
//!
//! Note: Tauri maps camelCase JS arg keys → snake_case Rust params, so any
//! multi-word field (e.g. `is_dir`) is `#[serde(rename_all = "camelCase")]`.

use serde::de::DeserializeOwned;
use serde::Serialize;
use wasm_bindgen::prelude::*;

use crate::model::{NoteMeta, TreeNode, WellRef};

#[wasm_bindgen]
extern "C" {
    /// Tauri's `invoke`. Resolves with the command's return value, or rejects
    /// (caught as `Err`) when the command returns `Err`.
    #[wasm_bindgen(js_namespace = ["window", "__TAURI__", "core"], catch)]
    async fn invoke(cmd: &str, args: JsValue) -> Result<JsValue, JsValue>;
}

/// Invoke a command that takes no arguments.
async fn call_bare(cmd: &str) -> Result<JsValue, JsValue> {
    invoke(cmd, JsValue::null()).await
}

/// Serialise `args` and invoke `cmd`.
async fn call<A: Serialize>(cmd: &str, args: &A) -> Result<JsValue, JsValue> {
    invoke(cmd, serde_wasm_bindgen::to_value(args).unwrap()).await
}

/// Deserialise a command result, or `None` on rejection / bad shape.
fn deser<T: DeserializeOwned>(res: Result<JsValue, JsValue>) -> Option<T> {
    res.ok()
        .and_then(|js| serde_wasm_bindgen::from_value(js).ok())
}

#[derive(Serialize)]
struct PathArg {
    path: String,
}

#[derive(Serialize)]
struct CreateWellArg {
    parent: String,
    name: String,
}

#[derive(Serialize)]
struct WellArg {
    well: String,
}

#[derive(Serialize)]
struct ReadArg {
    well: String,
    id: String,
}

#[derive(Serialize)]
struct WriteArg {
    well: String,
    id: String,
    content: String,
}

#[derive(Serialize)]
struct ParentArg {
    well: String,
    parent: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RenameArg {
    well: String,
    id: String,
    is_dir: bool,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DeleteArg {
    well: String,
    id: String,
    is_dir: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveArg {
    well: String,
    id: String,
    is_dir: bool,
    dest: String,
}

#[derive(Serialize)]
struct WindowArg {
    width: f64,
    height: f64,
    resizable: bool,
}

#[derive(Serialize)]
struct UrlArg {
    url: String,
}

// --- wells -----------------------------------------------------------------

/// Wells remembered as recently opened, most-recent first.
pub async fn recent_wells() -> Vec<WellRef> {
    deser(call_bare("recent_wells").await).unwrap_or_default()
}

/// Show the OS folder picker; `None` if the user cancels.
pub async fn pick_folder() -> Option<String> {
    call_bare("pick_folder")
        .await
        .ok()
        .and_then(|js| js.as_string())
}

/// Open `path` as a well; `None` on error.
pub async fn open_well(path: String) -> Option<WellRef> {
    deser(call("open_well", &PathArg { path }).await)
}

/// Create `parent/name` as a new well; `None` on error.
pub async fn create_well(parent: String, name: String) -> Option<WellRef> {
    deser(call("create_well", &CreateWellArg { parent, name }).await)
}

// --- notes & folders -------------------------------------------------------

/// The well's tree of folders and notes.
pub async fn list_tree(well: String) -> Vec<TreeNode> {
    deser(call("list_tree", &WellArg { well }).await).unwrap_or_default()
}

/// A note's markdown body (empty on error).
pub async fn read_note(well: String, id: String) -> String {
    call("read_note", &ReadArg { well, id })
        .await
        .ok()
        .and_then(|js| js.as_string())
        .unwrap_or_default()
}

/// A note's filesystem timestamps; `None` on error.
pub async fn note_meta(well: String, id: String) -> Option<NoteMeta> {
    deser(call("note_meta", &ReadArg { well, id }).await)
}

/// Persist a note's body.
pub async fn write_note(well: String, id: String, content: String) {
    let _ = call("write_note", &WriteArg { well, id, content }).await;
}

/// Create an empty note in `parent` (`""` = root); returns its id.
pub async fn create_note(well: String, parent: String) -> Option<String> {
    deser(call("create_note", &ParentArg { well, parent }).await)
}

/// Create a folder in `parent` (`""` = root); returns its id.
pub async fn create_folder(well: String, parent: String) -> Option<String> {
    deser(call("create_folder", &ParentArg { well, parent }).await)
}

/// Rename a note/folder in place; returns the new id, or `None` on error.
pub async fn rename_entry(well: String, id: String, is_dir: bool, name: String) -> Option<String> {
    deser(
        call(
            "rename_entry",
            &RenameArg {
                well,
                id,
                is_dir,
                name,
            },
        )
        .await,
    )
}

/// Delete a note (or empty folder); `true` on success.
pub async fn delete_entry(well: String, id: String, is_dir: bool) -> bool {
    call("delete_entry", &DeleteArg { well, id, is_dir })
        .await
        .is_ok()
}

/// Move a note/folder into `dest` (`""` = root); returns the new id.
pub async fn move_entry(well: String, id: String, is_dir: bool, dest: String) -> Option<String> {
    deser(
        call(
            "move_entry",
            &MoveArg {
                well,
                id,
                is_dir,
                dest,
            },
        )
        .await,
    )
}

// --- window ----------------------------------------------------------------

/// Resize + (un)lock the window and re-centre it.
pub async fn apply_window(width: f64, height: f64, resizable: bool) {
    let _ = call(
        "apply_window",
        &WindowArg {
            width,
            height,
            resizable,
        },
    )
    .await;
}

/// Reveal the (initially hidden) window.
pub async fn show_window() {
    let _ = call_bare("show_window").await;
}

/// Invoke a no-argument window command (`win_minimize` / `win_toggle_maximize`
/// / `win_close`).
pub async fn window_command(cmd: &str) {
    let _ = call_bare(cmd).await;
}

// --- external --------------------------------------------------------------

/// Open `url` in the OS default handler (browser, mail client, …).
pub async fn open_external(url: String) {
    let _ = call("open_external", &UrlArg { url }).await;
}
