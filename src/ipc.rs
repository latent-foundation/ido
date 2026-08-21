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

use serde::Serialize;
use serde::de::DeserializeOwned;
use wasm_bindgen::prelude::*;

use crate::model::{
    Goal, LinkRef, McpInfo, NoteMeta, SavedView, SearchHit, Session, Task, TreeNode, WellRef,
};

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

/// Invoke a command that returns `Result`, surfacing the backend's `Err` string
/// (so the UI can show why it failed, e.g. a rename collision).
async fn call_res<A: Serialize, T: DeserializeOwned>(cmd: &str, args: &A) -> Result<T, String> {
    match call(cmd, args).await {
        Ok(js) => serde_wasm_bindgen::from_value(js).map_err(|e| e.to_string()),
        Err(js) => Err(js.as_string().unwrap_or_else(|| "command failed".into())),
    }
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

#[derive(Serialize)]
struct WriteSessionArg {
    well: String,
    tabs: Vec<String>,
    active: Option<usize>,
}

#[derive(Serialize)]
struct PageArg {
    well: String,
    slug: String,
}

#[derive(Serialize)]
struct WritePageArg {
    well: String,
    slug: String,
    content: String,
}

#[derive(Serialize)]
struct RenamePageArg {
    well: String,
    slug: String,
    name: String,
}

#[derive(Serialize)]
struct CreatePageArg {
    well: String,
    folder: String,
}

#[derive(Serialize)]
struct WikiPathArg {
    well: String,
    path: String,
}

#[derive(Serialize)]
struct RenameWikiFolderArg {
    well: String,
    path: String,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MoveWikiEntryArg {
    well: String,
    path: String,
    is_dir: bool,
    dest: String,
}

#[derive(Serialize)]
struct CreateTaskArg {
    well: String,
    status: String,
    title: String,
    due: Option<String>,
}

#[derive(Serialize)]
struct TaskIdArg {
    well: String,
    id: String,
}

#[derive(Serialize)]
struct MoveTaskArg {
    well: String,
    id: String,
    status: String,
}

#[derive(Serialize)]
struct ReorderColumnArg {
    well: String,
    status: String,
    ids: Vec<String>,
}

#[derive(Serialize)]
struct TaskFieldArg {
    well: String,
    id: String,
    key: String,
    value: String,
}

#[derive(Serialize)]
struct TaskBodyArg {
    well: String,
    id: String,
    body: String,
}

#[derive(Serialize)]
struct TaskRenameArg {
    well: String,
    id: String,
    name: String,
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

/// Ensure `well` has the sectioned layout (`notes/ tasks/ wiki/ .ido/`),
/// migrating a legacy flat well on first open. Idempotent and fire-and-forget.
pub async fn migrate_well(well: String) {
    let _ = call("migrate_well", &WellArg { well }).await;
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

/// Rename a note/folder in place; returns the new id, or the error string.
pub async fn rename_entry(
    well: String,
    id: String,
    is_dir: bool,
    name: String,
) -> Result<String, String> {
    call_res(
        "rename_entry",
        &RenameArg {
            well,
            id,
            is_dir,
            name,
        },
    )
    .await
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

// --- wiki -------------------------------------------------------------------

/// The wiki's tree of folders and pages (folders first, then pages by slug).
pub async fn list_wiki(well: String) -> Vec<TreeNode> {
    deser(call("list_wiki", &WellArg { well }).await).unwrap_or_default()
}

/// A wiki page's markdown body (empty when the page doesn't exist yet).
pub async fn read_page(well: String, slug: String) -> String {
    call("read_page", &PageArg { well, slug })
        .await
        .ok()
        .and_then(|js| js.as_string())
        .unwrap_or_default()
}

/// Persist a wiki page's body.
pub async fn write_page(well: String, slug: String, content: String) {
    let _ = call(
        "write_page",
        &WritePageArg {
            well,
            slug,
            content,
        },
    )
    .await;
}

/// Create a new uniquely-named empty page inside `folder` (`""` = wiki root);
/// returns its (section-wide-unique) slug.
pub async fn create_page(well: String, folder: String) -> Option<String> {
    deser(call("create_page", &CreatePageArg { well, folder }).await)
}

/// Create an organisational folder in `parent` (`""` = wiki root); returns its
/// wiki-relative id.
pub async fn create_wiki_folder(well: String, parent: String) -> Option<String> {
    deser(call("create_wiki_folder", &ParentArg { well, parent }).await)
}

/// Rename a wiki folder in place; returns the new id, or the error string.
pub async fn rename_wiki_folder(
    well: String,
    path: String,
    name: String,
) -> Result<String, String> {
    call_res(
        "rename_wiki_folder",
        &RenameWikiFolderArg { well, path, name },
    )
    .await
}

/// Delete an empty wiki folder; `true` on success.
pub async fn delete_wiki_folder(well: String, path: String) -> bool {
    call("delete_wiki_folder", &WikiPathArg { well, path })
        .await
        .is_ok()
}

/// Move a wiki page or folder into `dest` (`""` = wiki root); returns the new id.
/// A page keeps its slug (only the file relocates), so its links never change.
pub async fn move_wiki_entry(
    well: String,
    path: String,
    is_dir: bool,
    dest: String,
) -> Option<String> {
    deser(
        call(
            "move_wiki_entry",
            &MoveWikiEntryArg {
                well,
                path,
                is_dir,
                dest,
            },
        )
        .await,
    )
}

/// Create `slug` if it doesn't exist (backs "create on click" for a wikilink).
pub async fn ensure_page(well: String, slug: String) {
    let _ = call("ensure_page", &PageArg { well, slug }).await;
}

/// Rename page `slug` to the slug of `name`; returns the new slug.
pub async fn rename_page(well: String, slug: String, name: String) -> Result<String, String> {
    call_res("rename_page", &RenamePageArg { well, slug, name }).await
}

/// Delete wiki page `slug`; `true` on success.
pub async fn delete_page(well: String, slug: String) -> bool {
    call("delete_page", &PageArg { well, slug }).await.is_ok()
}

/// Everything that links to `slug` (its backlinks) across notes + wiki.
pub async fn backlinks(well: String, slug: String) -> Vec<LinkRef> {
    deser(call("backlinks", &PageArg { well, slug }).await).unwrap_or_default()
}

// --- search -----------------------------------------------------------------

#[derive(Serialize)]
struct SearchArg {
    well: String,
    query: String,
}

/// Cross-section search hits for `query` (notes + wiki + tasks).
pub async fn search(well: String, query: String) -> Vec<SearchHit> {
    deser(call("search", &SearchArg { well, query }).await).unwrap_or_default()
}

// --- tasks ------------------------------------------------------------------

/// The board columns (task `status` values) for this well.
pub async fn task_columns(well: String) -> Vec<String> {
    deser(call("task_columns", &WellArg { well }).await).unwrap_or_default()
}

#[derive(Serialize)]
struct SetColumnsArg {
    well: String,
    columns: Vec<String>,
}

/// Replace the board's columns; returns the stored (slugified, deduped) set,
/// or the error string (e.g. no valid names).
pub async fn set_task_columns(well: String, columns: Vec<String>) -> Result<Vec<String>, String> {
    call_res("set_task_columns", &SetColumnsArg { well, columns }).await
}

/// The well's auto-archive-done-after-N-days setting; `None` = off.
pub async fn archive_days(well: String) -> Option<u32> {
    deser(call("archive_days", &WellArg { well }).await).unwrap_or_default()
}

#[derive(Serialize)]
struct SetArchiveDaysArg {
    well: String,
    days: Option<u32>,
}

/// Set (or clear, with `None`) the auto-archive-done setting; the backend
/// runs the sweep immediately, so a newly-set or lowered threshold can
/// archive tasks right away. Returns the stored value, or the error string.
pub async fn set_archive_days(well: String, days: Option<u32>) -> Result<Option<u32>, String> {
    call_res("set_archive_days", &SetArchiveDaysArg { well, days }).await
}

/// The well's saved tasks-toolbar views (the board's "views" dropdown).
pub async fn saved_views(well: String) -> Vec<SavedView> {
    deser(call("saved_views", &WellArg { well }).await).unwrap_or_default()
}

#[derive(Serialize)]
struct SetViewsArg {
    well: String,
    views: Vec<SavedView>,
}

/// Replace the well's saved views wholesale; returns the stored list (blank
/// names dropped), or the error string.
pub async fn set_saved_views(
    well: String,
    views: Vec<SavedView>,
) -> Result<Vec<SavedView>, String> {
    call_res("set_saved_views", &SetViewsArg { well, views }).await
}

/// Every task in the well, sorted by order.
pub async fn list_tasks(well: String) -> Vec<Task> {
    deser(call("list_tasks", &WellArg { well }).await).unwrap_or_default()
}

/// Create a task in column `status` titled `title` (blank = untitled), due on
/// `due` (`None`/empty = undated); returns its id (the title's slug, uniquified).
pub async fn create_task(
    well: String,
    status: String,
    title: String,
    due: Option<String>,
) -> Option<String> {
    deser(
        call(
            "create_task",
            &CreateTaskArg {
                well,
                status,
                title,
                due,
            },
        )
        .await,
    )
}

/// Move task `id` into column `status` (appended at the end).
pub async fn move_task(well: String, id: String, status: String) {
    let _ = call("move_task", &MoveTaskArg { well, id, status }).await;
}

/// Reorder column `status` to exactly `ids` (each gets `order = index`).
pub async fn reorder_column(well: String, status: String, ids: Vec<String>) {
    let _ = call("reorder_column", &ReorderColumnArg { well, status, ids }).await;
}

/// Set (or clear, when blank) a single task metadata field.
pub async fn set_task_field(well: String, id: String, key: String, value: String) {
    let _ = call(
        "set_task_field",
        &TaskFieldArg {
            well,
            id,
            key,
            value,
        },
    )
    .await;
}

/// Replace a task's markdown body.
pub async fn update_task_body(well: String, id: String, body: String) {
    let _ = call("update_task_body", &TaskBodyArg { well, id, body }).await;
}

/// Rename task `id` to the slug of `name`; returns the new id, or the error string.
pub async fn rename_task(well: String, id: String, name: String) -> Result<String, String> {
    call_res("rename_task", &TaskRenameArg { well, id, name }).await
}

/// Delete task `id`; returns its raw content (for undo), or `None` on error.
pub async fn delete_task(well: String, id: String) -> Option<String> {
    deser(call("delete_task", &TaskIdArg { well, id }).await)
}

/// Recreate task `id` from raw markdown (undo of a delete).
pub async fn restore_task(well: String, id: String, content: String) {
    let _ = call("restore_task", &WriteArg { well, id, content }).await;
}

// --- goals ------------------------------------------------------------------

/// Every goal in the well, alphabetical.
pub async fn list_goals(well: String) -> Vec<Goal> {
    deser(call("list_goals", &WellArg { well }).await).unwrap_or_default()
}

/// Create a new empty goal; returns its id.
pub async fn create_goal(well: String) -> Option<String> {
    deser(call("create_goal", &WellArg { well }).await)
}

#[derive(Serialize)]
struct ReorderGoalsArg {
    well: String,
    ids: Vec<String>,
}

/// Renumber the goals bar to match `ids` (drag-to-reorder).
pub async fn reorder_goals(well: String, ids: Vec<String>) {
    let _ = call("reorder_goals", &ReorderGoalsArg { well, ids }).await;
}

/// Set (or clear, when blank) a goal field (`target`).
pub async fn set_goal_field(well: String, id: String, key: String, value: String) {
    let _ = call(
        "set_goal_field",
        &TaskFieldArg {
            well,
            id,
            key,
            value,
        },
    )
    .await;
}

/// Replace a goal's markdown body.
pub async fn update_goal_body(well: String, id: String, body: String) {
    let _ = call("update_goal_body", &TaskBodyArg { well, id, body }).await;
}

/// Rename goal `id` to the slug of `name`; returns the new id, or the error string.
pub async fn rename_goal(well: String, id: String, name: String) -> Result<String, String> {
    call_res("rename_goal", &TaskRenameArg { well, id, name }).await
}

/// Delete goal `id`; returns its raw content (for undo), or `None` on error.
pub async fn delete_goal(well: String, id: String) -> Option<String> {
    deser(call("delete_goal", &TaskIdArg { well, id }).await)
}

/// Recreate goal `id` from raw markdown (undo of a delete).
pub async fn restore_goal(well: String, id: String, content: String) {
    let _ = call("restore_goal", &WriteArg { well, id, content }).await;
}

// --- session ---------------------------------------------------------------

/// A well's saved open-tabs session; `None` on error (treated as empty).
pub async fn read_session(well: String) -> Option<Session> {
    deser(call("read_session", &WellArg { well }).await)
}

/// Persist a well's open tabs (note ids) + active index. Fire-and-forget.
pub async fn write_session(well: String, tabs: Vec<String>, active: Option<usize>) {
    let _ = call("write_session", &WriteSessionArg { well, tabs, active }).await;
}

// --- assets ------------------------------------------------------------------

#[derive(Serialize)]
struct SaveAssetArg {
    well: String,
    name: String,
    bytes: Vec<u8>,
}

/// Save a pasted/dropped image's bytes into the well's shared `assets/` folder;
/// returns its well-relative id (`assets/<file>`) to embed as `![](…)`.
pub async fn save_asset(well: String, name: String, bytes: Vec<u8>) -> Option<String> {
    deser(call("save_asset", &SaveAssetArg { well, name, bytes }).await)
}

#[derive(Serialize)]
struct AssetArg {
    well: String,
    id: String,
}

/// Read an asset (by well-relative id, e.g. `assets/foo.png`) back as a `data:`
/// URI for inline rendering.
pub async fn read_asset(well: String, id: String) -> Option<String> {
    deser(call("read_asset", &AssetArg { well, id }).await)
}

// --- mcp ---------------------------------------------------------------------

/// Resolve the bundled `ido-mcp` sidecar for `well` and build copy-paste MCP
/// client config (a `.mcp.json` snippet + a `claude mcp add` one-liner);
/// `None` on error.
pub async fn mcp_info(well: String) -> Option<McpInfo> {
    deser(call("mcp_info", &WellArg { well }).await)
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

/// Restore the editor window to the remembered geometry (or the default).
pub async fn restore_window() {
    let _ = call_bare("restore_window").await;
}

/// Persist the editor window's current size, position, and maximized state.
pub async fn remember_window() {
    let _ = call_bare("remember_window").await;
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
