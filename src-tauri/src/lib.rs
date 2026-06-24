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
//! - [`wells`] — opening / creating wells + the folder picker
//! - [`notes`] — the tree and note/folder CRUD (unit-tested)
//! - [`window`] — custom-chrome window control
//! - [`external`] — opening URLs in the OS

mod external;
mod model;
mod notes;
mod paths;
mod registry;
mod wells;
mod window;

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
            notes::list_tree,
            notes::read_note,
            notes::write_note,
            notes::create_note,
            notes::create_folder,
            notes::rename_entry,
            notes::delete_entry,
            notes::move_entry,
            window::apply_window,
            window::show_window,
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_close,
            external::open_external
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
