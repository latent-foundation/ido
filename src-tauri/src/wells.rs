//! Opening and creating wells, and the native folder picker that feeds them.

use std::fs;
use std::path::Path;

use tauri_plugin_dialog::DialogExt;

use crate::model::WellRef;
use crate::paths::{valid_name, well_ref};
use crate::registry::register_well;

/// Whether `dir` contains any markdown file, recursively.
fn has_md(dir: &Path) -> bool {
    let Ok(entries) = fs::read_dir(dir) else {
        return false;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if has_md(&path) {
                return true;
            }
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            return true;
        }
    }
    false
}

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
    if !Path::new(&path).is_dir() {
        return Err(format!("not a folder: {path}"));
    }
    register_well(&app, &path);
    Ok(well_ref(&path))
}

/// Create `parent/name` as a new well, seeding a welcome note if it has none,
/// and record it as recent.
#[tauri::command]
pub fn create_well(app: tauri::AppHandle, parent: String, name: String) -> Result<WellRef, String> {
    let name = valid_name(&name)?;
    let path = Path::new(&parent).join(name).to_string_lossy().into_owned();
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    if !has_md(Path::new(&path)) {
        let _ = fs::write(
            Path::new(&path).join("welcome.md"),
            "# Welcome to your well\n\nNotes are plain markdown files in this folder. \
             Create one, write, and it saves itself.\n",
        );
    }
    register_well(&app, &path);
    Ok(well_ref(&path))
}
