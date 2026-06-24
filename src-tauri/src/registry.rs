//! The recent-wells registry — a `wells.json` list of well paths kept in the
//! app data dir, most-recent first.

use std::fs;
use std::path::PathBuf;

use tauri::Manager;

use crate::model::WellRef;
use crate::paths::well_ref;

/// Path to `wells.json`, creating the app data dir if needed.
fn registry_path(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app.path().app_data_dir().map_err(|e| e.to_string())?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.join("wells.json"))
}

/// The raw list of remembered well paths (most-recent first), or empty.
fn read_registry(app: &tauri::AppHandle) -> Vec<String> {
    registry_path(app)
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

/// Record `path` as the most-recently-used well (deduped, capped at 10).
pub(crate) fn register_well(app: &tauri::AppHandle, path: &str) {
    let mut list = read_registry(app);
    list.retain(|p| p != path);
    list.insert(0, path.to_string());
    list.truncate(10);
    if let Ok(p) = registry_path(app) {
        let _ = fs::write(p, serde_json::to_string_pretty(&list).unwrap_or_default());
    }
}

/// Known wells, most-recent first, filtered to those that still exist on disk.
#[tauri::command]
pub fn recent_wells(app: tauri::AppHandle) -> Vec<WellRef> {
    read_registry(&app)
        .into_iter()
        .filter(|p| std::path::Path::new(p).is_dir())
        .map(|p| well_ref(&p))
        .collect()
}
