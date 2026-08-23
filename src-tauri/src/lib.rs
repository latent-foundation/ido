//! ido backend — thin Tauri command wrappers over the tauri-free `ido-store`
//! crate, plus window control and OS hand-offs that need real Tauri types.
//!
//! The local-first store (wells, notes, wiki, tasks, search, session) lives in
//! [`ido_store`] — see that crate's docs for the data model and layout. This
//! crate only adds:
//! - [`commands`] — one `#[tauri::command]` wrapper per store function (same
//!   name, args, and return shape as before the store moved out of this
//!   crate), plus the three well-opening commands that need an `AppHandle`
//!   (the native folder picker and the recent-wells registry write)
//! - [`registry`] — the recent-wells list (`wells.json`)
//! - [`window`] — custom-chrome window control
//! - [`external`] — opening URLs in the OS
//! - [`mcp`] — resolving the bundled `ido-mcp` sidecar and building the
//!   settings pane's copy-paste MCP client config
//! - [`semantic`] — in-app semantic search: the model download, the index
//!   build, and hybrid queries (docs/mcp-server.md P3)

mod commands;
mod external;
mod mcp;
mod registry;
mod semantic;
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
            commands::pick_folder,
            commands::open_well,
            commands::create_well,
            commands::migrate_well,
            commands::set_task_columns,
            commands::set_archive_days,
            commands::saved_views,
            commands::set_saved_views,
            commands::list_tree,
            commands::read_note,
            commands::note_meta,
            commands::write_note,
            commands::create_note,
            commands::create_folder,
            commands::rename_entry,
            commands::delete_entry,
            commands::move_entry,
            commands::read_session,
            commands::write_session,
            commands::list_wiki,
            commands::read_page,
            commands::write_page,
            commands::create_page,
            commands::ensure_page,
            commands::rename_page,
            commands::delete_page,
            commands::create_wiki_folder,
            commands::rename_wiki_folder,
            commands::delete_wiki_folder,
            commands::move_wiki_entry,
            commands::backlinks,
            commands::task_columns,
            commands::archive_days,
            commands::list_tasks,
            commands::create_task,
            commands::move_task,
            commands::reorder_column,
            commands::set_task_field,
            commands::update_task_body,
            commands::rename_task,
            commands::delete_task,
            commands::restore_task,
            commands::list_goals,
            commands::create_goal,
            commands::reorder_goals,
            commands::set_goal_field,
            commands::update_goal_body,
            commands::rename_goal,
            commands::delete_goal,
            commands::restore_goal,
            commands::save_asset,
            commands::read_asset,
            window::apply_window,
            window::remember_window,
            window::restore_window,
            window::show_window,
            window::win_minimize,
            window::win_toggle_maximize,
            window::win_close,
            external::open_external,
            mcp::mcp_info,
            semantic::semantic_info,
            semantic::download_model,
            semantic::build_index,
            semantic::job_status,
            semantic::search_hybrid
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
