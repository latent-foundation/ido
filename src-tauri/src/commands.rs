//! `#[tauri::command]` wrappers over [`ido_store`] — one per store function,
//! same name/args/return shape as before the store moved out of this crate
//! (see `ido_store`'s docs), so the frontend `ipc` layer needed no changes.
//!
//! [`pick_folder`], [`open_well`], and [`create_well`] are the exceptions:
//! they need an `AppHandle` (the native folder picker, and the recent-wells
//! registry write on open/create), which `ido_store` deliberately doesn't
//! depend on. The latter two delegate their pure half to
//! [`ido_store::wells::open_well`] / [`ido_store::wells::create_well`].

use tauri_plugin_dialog::DialogExt;

use ido_store::model::{
    Goal, LinkRef, NoteMeta, SavedView, SearchHit, Session, Task, TreeNode, WellRef,
};

use crate::registry::register_well;

// --- wells --------------------------------------------------------------

/// Open the OS folder picker. Resolves to the chosen path, or `None` if the
/// user cancels. (Used for both "open a well" and "choose a location".)
#[tauri::command]
pub async fn pick_folder(app: tauri::AppHandle) -> Result<Option<String>, String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |path| {
        let _ = tx.send(path);
    });
    let picked = rx.await.map_err(|e| e.to_string())?;
    Ok(picked
        .and_then(|p| p.into_path().ok())
        .map(|p| p.to_string_lossy().into_owned()))
}

/// Open an existing folder as a well and record it as recent.
#[tauri::command]
pub fn open_well(app: tauri::AppHandle, path: String) -> Result<WellRef, String> {
    let well = ido_store::wells::open_well(path)?;
    register_well(&app, &well.path);
    Ok(well)
}

/// Create `parent/name` as a new well, scaffold its sections, seed a welcome
/// note into `notes/` if it's empty, and record it as recent.
#[tauri::command]
pub fn create_well(app: tauri::AppHandle, parent: String, name: String) -> Result<WellRef, String> {
    let well = ido_store::wells::create_well(parent, name)?;
    register_well(&app, &well.path);
    Ok(well)
}

#[tauri::command]
pub fn migrate_well(well: String) -> Result<(), String> {
    ido_store::wells::migrate_well(well)
}

#[tauri::command]
pub fn set_task_columns(well: String, columns: Vec<String>) -> Result<Vec<String>, String> {
    ido_store::wells::set_task_columns(well, columns)
}

#[tauri::command]
pub fn set_archive_days(well: String, days: Option<u32>) -> Result<Option<u32>, String> {
    ido_store::wells::set_archive_days(well, days)
}

#[tauri::command]
pub fn saved_views(well: String) -> Vec<SavedView> {
    ido_store::wells::saved_views(well)
}

#[tauri::command]
pub fn set_saved_views(well: String, views: Vec<SavedView>) -> Result<Vec<SavedView>, String> {
    ido_store::wells::set_saved_views(well, views)
}

// --- notes ----------------------------------------------------------------

#[tauri::command]
pub fn list_tree(well: String) -> Result<Vec<TreeNode>, String> {
    ido_store::notes::list_tree(well)
}

#[tauri::command]
pub fn read_note(well: String, id: String) -> Result<String, String> {
    ido_store::notes::read_note(well, id)
}

#[tauri::command]
pub fn note_meta(well: String, id: String) -> Result<NoteMeta, String> {
    ido_store::notes::note_meta(well, id)
}

#[tauri::command]
pub fn write_note(well: String, id: String, content: String) -> Result<(), String> {
    ido_store::notes::write_note(well, id, content)
}

#[tauri::command]
pub fn create_note(well: String, parent: String) -> Result<String, String> {
    ido_store::notes::create_note(well, parent)
}

#[tauri::command]
pub fn create_folder(well: String, parent: String) -> Result<String, String> {
    ido_store::notes::create_folder(well, parent)
}

#[tauri::command]
pub fn rename_entry(
    well: String,
    id: String,
    is_dir: bool,
    name: String,
) -> Result<String, String> {
    ido_store::notes::rename_entry(well, id, is_dir, name)
}

#[tauri::command]
pub fn delete_entry(well: String, id: String, is_dir: bool) -> Result<(), String> {
    ido_store::notes::delete_entry(well, id, is_dir)
}

#[tauri::command]
pub fn move_entry(well: String, id: String, is_dir: bool, dest: String) -> Result<String, String> {
    ido_store::notes::move_entry(well, id, is_dir, dest)
}

// --- session ----------------------------------------------------------------

#[tauri::command]
pub fn read_session(well: String) -> Session {
    ido_store::session::read_session(well)
}

#[tauri::command]
pub fn write_session(well: String, tabs: Vec<String>, active: Option<usize>) -> Result<(), String> {
    ido_store::session::write_session(well, tabs, active)
}

// --- wiki ----------------------------------------------------------------

#[tauri::command]
pub fn list_wiki(well: String) -> Vec<TreeNode> {
    ido_store::wiki::list_wiki(well)
}

#[tauri::command]
pub fn read_page(well: String, slug: String) -> String {
    ido_store::wiki::read_page(well, slug)
}

#[tauri::command]
pub fn write_page(well: String, slug: String, content: String) -> Result<(), String> {
    ido_store::wiki::write_page(well, slug, content)
}

#[tauri::command]
pub fn create_page(well: String, folder: String) -> Result<String, String> {
    ido_store::wiki::create_page(well, folder)
}

#[tauri::command]
pub fn ensure_page(well: String, slug: String) -> Result<(), String> {
    ido_store::wiki::ensure_page(well, slug)
}

#[tauri::command]
pub fn rename_page(well: String, slug: String, name: String) -> Result<String, String> {
    ido_store::wiki::rename_page(well, slug, name)
}

#[tauri::command]
pub fn delete_page(well: String, slug: String) -> Result<(), String> {
    ido_store::wiki::delete_page(well, slug)
}

#[tauri::command]
pub fn create_wiki_folder(well: String, parent: String) -> Result<String, String> {
    ido_store::wiki::create_wiki_folder(well, parent)
}

#[tauri::command]
pub fn rename_wiki_folder(well: String, path: String, name: String) -> Result<String, String> {
    ido_store::wiki::rename_wiki_folder(well, path, name)
}

#[tauri::command]
pub fn delete_wiki_folder(well: String, path: String) -> Result<(), String> {
    ido_store::wiki::delete_wiki_folder(well, path)
}

#[tauri::command]
pub fn move_wiki_entry(
    well: String,
    path: String,
    is_dir: bool,
    dest: String,
) -> Result<String, String> {
    ido_store::wiki::move_wiki_entry(well, path, is_dir, dest)
}

#[tauri::command]
pub fn backlinks(well: String, slug: String) -> Vec<LinkRef> {
    ido_store::wiki::backlinks(well, slug)
}

// --- tasks ----------------------------------------------------------------

#[tauri::command]
pub fn task_columns(well: String) -> Vec<String> {
    ido_store::tasks::task_columns(well)
}

#[tauri::command]
pub fn archive_days(well: String) -> Option<u32> {
    ido_store::tasks::archive_days(well)
}

#[tauri::command]
pub fn list_tasks(well: String) -> Vec<Task> {
    ido_store::tasks::list_tasks(well)
}

#[tauri::command]
pub fn create_task(
    well: String,
    status: String,
    title: String,
    due: Option<String>,
) -> Result<String, String> {
    ido_store::tasks::create_task(well, status, title, due)
}

#[tauri::command]
pub fn move_task(well: String, id: String, status: String) -> Result<(), String> {
    ido_store::tasks::move_task(well, id, status)
}

#[tauri::command]
pub fn reorder_column(well: String, status: String, ids: Vec<String>) -> Result<(), String> {
    ido_store::tasks::reorder_column(well, status, ids)
}

#[tauri::command]
pub fn set_task_field(well: String, id: String, key: String, value: String) -> Result<(), String> {
    ido_store::tasks::set_task_field(well, id, key, value)
}

#[tauri::command]
pub fn update_task_body(well: String, id: String, body: String) -> Result<(), String> {
    ido_store::tasks::update_task_body(well, id, body)
}

#[tauri::command]
pub fn rename_task(well: String, id: String, name: String) -> Result<String, String> {
    ido_store::tasks::rename_task(well, id, name)
}

#[tauri::command]
pub fn delete_task(well: String, id: String) -> Result<String, String> {
    ido_store::tasks::delete_task(well, id)
}

#[tauri::command]
pub fn restore_task(well: String, id: String, content: String) -> Result<(), String> {
    ido_store::tasks::restore_task(well, id, content)
}

#[tauri::command]
pub fn list_goals(well: String) -> Vec<Goal> {
    ido_store::tasks::list_goals(well)
}

#[tauri::command]
pub fn create_goal(well: String) -> Result<String, String> {
    ido_store::tasks::create_goal(well)
}

#[tauri::command]
pub fn reorder_goals(well: String, ids: Vec<String>) -> Result<(), String> {
    ido_store::tasks::reorder_goals(well, ids)
}

#[tauri::command]
pub fn set_goal_field(well: String, id: String, key: String, value: String) -> Result<(), String> {
    ido_store::tasks::set_goal_field(well, id, key, value)
}

#[tauri::command]
pub fn update_goal_body(well: String, id: String, body: String) -> Result<(), String> {
    ido_store::tasks::update_goal_body(well, id, body)
}

#[tauri::command]
pub fn rename_goal(well: String, id: String, name: String) -> Result<String, String> {
    ido_store::tasks::rename_goal(well, id, name)
}

#[tauri::command]
pub fn delete_goal(well: String, id: String) -> Result<String, String> {
    ido_store::tasks::delete_goal(well, id)
}

#[tauri::command]
pub fn restore_goal(well: String, id: String, content: String) -> Result<(), String> {
    ido_store::tasks::restore_goal(well, id, content)
}

// --- search ----------------------------------------------------------------

#[tauri::command]
pub fn search(well: String, query: String) -> Vec<SearchHit> {
    ido_store::search::search(well, query)
}

// --- assets ----------------------------------------------------------------

#[tauri::command]
pub fn save_asset(well: String, name: String, bytes: Vec<u8>) -> Result<String, String> {
    ido_store::assets::save_asset(well, name, bytes)
}

#[tauri::command]
pub fn read_asset(well: String, id: String) -> Result<String, String> {
    ido_store::assets::read_asset(well, id)
}
