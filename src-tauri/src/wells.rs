//! Opening and creating wells, and the native folder picker that feeds them.

use std::fs;
use std::path::Path;

use tauri_plugin_dialog::DialogExt;

use crate::model::{Section, SectionFlags, WellManifest, WellRef};
use crate::paths::{unique_name, valid_name, well_ref};
use crate::registry::register_well;

/// The reserved subfolders + manifest every well has. Idempotent: re-creating
/// existing folders is a no-op, and the manifest is written only when absent
/// (its presence is what marks a well as already migrated).
fn scaffold(well: &Path) -> std::io::Result<()> {
    for section in Section::ALL {
        fs::create_dir_all(well.join(section.dir()))?;
    }
    fs::create_dir_all(well.join("tasks").join("epics"))?;
    let ido = well.join(".ido");
    fs::create_dir_all(&ido)?;
    let manifest = ido.join("well.toml");
    if !manifest.exists() {
        let m = WellManifest {
            schema: 1,
            sections: SectionFlags {
                notes: true,
                tasks: true,
                wiki: true,
            },
        };
        fs::write(manifest, toml::to_string(&m).unwrap_or_default())?;
    }
    Ok(())
}

/// Root-level `.md` files and non-reserved folders of a legacy (pre-sections)
/// well — the entries migration sweeps into `notes/`. Returns `(name, path)`.
fn legacy_entries(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let reserved = ["notes", "tasks", "wiki", ".ido"];
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if name.starts_with('.') || reserved.contains(&name) {
            continue;
        }
        if path.is_dir() || path.extension().and_then(|e| e.to_str()) == Some("md") {
            out.push((name.to_string(), path));
        }
    }
    out
}

/// Bring a well up to the sectioned layout: scaffold the reserved folders and,
/// for a legacy well (no `.ido/well.toml`), move its root notes into `notes/`.
/// Idempotent — a migrated well returns early, so it is safe on every open.
#[tauri::command]
pub fn migrate_well(well: String) -> Result<(), String> {
    let root = Path::new(&well);
    if root.join(".ido").join("well.toml").exists() {
        return Ok(());
    }
    // Snapshot legacy entries before scaffolding, so the reserved folders we
    // are about to create are never swept into notes/.
    let legacy = legacy_entries(root);
    scaffold(root).map_err(|e| e.to_string())?;
    let notes = root.join("notes");
    for (name, path) in legacy {
        let p = Path::new(&name);
        let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(&name);
        let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("");
        let unique = unique_name(&notes, stem, ext);
        let dest = if ext.is_empty() {
            unique
        } else {
            format!("{unique}.{ext}")
        };
        let _ = fs::rename(path, notes.join(dest));
    }
    Ok(())
}

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

/// Create `parent/name` as a new well, scaffold its sections, seed a welcome
/// note into `notes/` if it's empty, and record it as recent.
#[tauri::command]
pub fn create_well(app: tauri::AppHandle, parent: String, name: String) -> Result<WellRef, String> {
    let name = valid_name(&name)?;
    let path = Path::new(&parent).join(name);
    fs::create_dir_all(&path).map_err(|e| e.to_string())?;
    scaffold(&path).map_err(|e| e.to_string())?;
    let notes = path.join("notes");
    if !has_md(&notes) {
        let _ = fs::write(
            notes.join("welcome.md"),
            "# Welcome to your well\n\nNotes are plain markdown files in this folder. \
             Create one, write, and it saves itself.\n",
        );
    }
    let path = path.to_string_lossy().into_owned();
    register_well(&app, &path);
    Ok(well_ref(&path))
}

// `migrate_well` takes a plain path (no `AppHandle`), so it's directly testable
// against a temp-dir well — same approach as the `notes` tests.
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn migrate_sweeps_legacy_notes_and_is_idempotent() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        // A legacy well: a root note plus a folder with a note.
        fs::write(dir.path().join("hello.md"), "# hi").unwrap();
        fs::create_dir_all(dir.path().join("stuff")).unwrap();
        fs::write(dir.path().join("stuff").join("bar.md"), "bar").unwrap();

        migrate_well(well.clone()).unwrap();

        // Structure exists.
        assert!(dir.path().join(".ido/well.toml").exists());
        assert!(dir.path().join("notes").is_dir());
        assert!(dir.path().join("tasks/epics").is_dir());
        assert!(dir.path().join("wiki").is_dir());
        // Legacy entries moved under notes/ (and gone from the root).
        assert!(dir.path().join("notes/hello.md").exists());
        assert!(dir.path().join("notes/stuff/bar.md").exists());
        assert!(!dir.path().join("hello.md").exists());
        assert!(!dir.path().join("stuff").exists());

        // Idempotent: the manifest now exists, so a second call is a no-op —
        // a note added at the root afterwards is NOT swept.
        fs::write(dir.path().join("late.md"), "late").unwrap();
        migrate_well(well).unwrap();
        assert!(dir.path().join("late.md").exists());
        assert!(!dir.path().join("notes/late.md").exists());
    }

    #[test]
    fn migrate_empty_well_just_scaffolds() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        migrate_well(well).unwrap();
        assert!(dir.path().join(".ido/well.toml").exists());
        for s in ["notes", "tasks", "wiki"] {
            assert!(dir.path().join(s).is_dir(), "{s} missing");
        }
    }
}
