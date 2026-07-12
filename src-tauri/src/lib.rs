//! ido backend — the local-first store and window control.
//!
//! A *well* is a folder the user chooses; notes are the markdown files within
//! it, and subfolders form the tree. A note's name is its file name (independent
//! of the body text). These Tauri commands are the only path to disk.
//!
//! Layout:
//! - [`model`] — shapes shared with the frontend (`WellRef`, `TreeNode`)
//! - [`paths`] — pure id/name helpers (unit-tested)
//! - [`registry`] — the recent-wells list (`wells.json`)
//! - [`wells`] — opening / creating / migrating wells + the folder picker
//! - [`notes`] — the tree and note/folder CRUD (unit-tested)
//! - [`wiki`] — the flat wiki page namespace + backlinks (unit-tested)
//! - [`tasks`] — the kanban task store over markdown frontmatter (unit-tested)
//! - [`repeat`] — pure recurrence math for a task's `repeat:` rule (unit-tested)
//! - [`assets`] — the shared `assets/` image-attachment store (unit-tested)
//! - [`search`] — cross-section full-text scan (unit-tested)
//! - [`frontmatter`] — the minimal `--- key: value ---` parser (unit-tested)
//! - [`session`] — the per-well open-tabs session (`.ido/session.toml`)
//! - [`window`] — custom-chrome window control
//! - [`external`] — opening URLs in the OS

mod assets;
mod external;
mod frontmatter;
mod model;
mod notes;
mod paths;
mod registry;
mod repeat;
mod search;
mod session;
mod tasks;
mod wells;
mod wiki;
mod window;

use tauri::Manager;

/// Build and run the Tauri application: register the plugins and the full
/// command surface, then hand off to the event loop.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![
            registry::recent_wells,
            wells::pick_folder,
            wells::open_well,
            wells::create_well,
            wells::migrate_well,
            wells::set_task_columns,
            wells::set_archive_days,
            wells::saved_views,
            wells::set_saved_views,
            notes::list_tree,
            notes::read_note,
            notes::note_meta,
            notes::write_note,
            notes::create_note,
            notes::create_folder,
            notes::rename_entry,
            notes::delete_entry,
            notes::move_entry,
            session::read_session,
            session::write_session,
            wiki::list_wiki,
            wiki::read_page,
            wiki::write_page,
            wiki::create_page,
            wiki::ensure_page,
            wiki::rename_page,
            wiki::delete_page,
            wiki::backlinks,
            tasks::task_columns,
            tasks::archive_days,
            tasks::list_tasks,
            tasks::create_task,
            tasks::move_task,
            tasks::reorder_column,
            tasks::set_task_field,
            tasks::update_task_body,
            tasks::rename_task,
            tasks::delete_task,
            tasks::restore_task,
            search::search,
            tasks::list_goals,
            tasks::create_goal,
            tasks::reorder_goals,
            tasks::set_goal_field,
            tasks::update_goal_body,
            tasks::rename_goal,
            tasks::delete_goal,
            tasks::restore_goal,
            assets::save_asset,
            assets::read_asset,
            window::apply_window,
            window::remember_window,
            window::restore_window,
            window::show_window,
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_close,
            external::open_external
        ])
        // Persist the window geometry on close — the reliable capture point for
        // position (the OS gives no move event the frontend can hook), covering
        // both the custom close button and Alt+F4.
        .on_window_event(|window, event| {
            if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
                window::save_geometry(window.app_handle());
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
