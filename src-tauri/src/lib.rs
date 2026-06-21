// ido backend — the local-first store. Notes are plain markdown files under the
// app data directory; these commands are the only way the frontend touches disk.

use std::fs;
use std::path::PathBuf;

use serde::Serialize;
use tauri::Manager;

#[derive(Serialize)]
struct NoteMeta {
    /// File stem — the stable id used to read/write the note.
    id: String,
    /// First non-empty line (leading `#` stripped), or the id if the note is empty.
    title: String,
}

/// `<app_data_dir>/notes`, created on first use.
fn notes_dir(app: &tauri::AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| e.to_string())?
        .join("notes");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn title_from(content: &str, id: &str) -> String {
    content
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.trim_start_matches('#').trim().to_string())
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| id.to_string())
}

#[tauri::command]
fn list_notes(app: tauri::AppHandle) -> Result<Vec<NoteMeta>, String> {
    let dir = notes_dir(&app)?;
    let mut notes = Vec::new();
    for entry in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("md") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or_default()
            .to_string();
        let content = fs::read_to_string(&path).unwrap_or_default();
        let title = title_from(&content, &id);
        notes.push(NoteMeta { id, title });
    }
    notes.sort_by_key(|n| n.title.to_lowercase());
    Ok(notes)
}

#[tauri::command]
fn read_note(app: tauri::AppHandle, id: String) -> Result<String, String> {
    let path = notes_dir(&app)?.join(format!("{id}.md"));
    fs::read_to_string(path).map_err(|e| e.to_string())
}

#[tauri::command]
fn write_note(app: tauri::AppHandle, id: String, content: String) -> Result<(), String> {
    let path = notes_dir(&app)?.join(format!("{id}.md"));
    fs::write(path, content).map_err(|e| e.to_string())
}

#[tauri::command]
fn create_note(app: tauri::AppHandle) -> Result<NoteMeta, String> {
    let dir = notes_dir(&app)?;
    let mut n = 1;
    let id = loop {
        let candidate = if n == 1 {
            "untitled".to_string()
        } else {
            format!("untitled-{n}")
        };
        if !dir.join(format!("{candidate}.md")).exists() {
            break candidate;
        }
        n += 1;
    };
    fs::write(dir.join(format!("{id}.md")), "# Untitled\n\n").map_err(|e| e.to_string())?;
    Ok(NoteMeta {
        id,
        title: "Untitled".to_string(),
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            list_notes,
            read_note,
            write_note,
            create_note
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
