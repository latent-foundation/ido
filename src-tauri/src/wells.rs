//! Opening and creating wells, and the native folder picker that feeds them.

use std::fs;
use std::path::Path;

use tauri_plugin_dialog::DialogExt;

use crate::model::{SavedView, Section, SectionFlags, WellManifest, WellRef};
use crate::paths::{slugify, unique_name, valid_name, well_ref};
use crate::registry::register_well;

/// The reserved subfolders + manifest every well has. Idempotent: re-creating
/// existing folders is a no-op, and the manifest is written only when absent
/// (its presence is what marks a well as already migrated).
fn scaffold(well: &Path) -> std::io::Result<()> {
    for section in Section::ALL {
        fs::create_dir_all(well.join(section.dir()))?;
    }
    fs::create_dir_all(well.join("tasks").join("goals"))?;
    fs::create_dir_all(well.join(crate::assets::ASSETS_DIR))?;
    let ido = well.join(".ido");
    fs::create_dir_all(&ido)?;
    let manifest = ido.join("well.toml");
    if !manifest.exists() {
        fs::write(
            manifest,
            toml::to_string(&fresh_manifest()).unwrap_or_default(),
        )?;
    }
    Ok(())
}

/// A default manifest for a freshly scaffolded well (all sections on, default
/// columns). Also the fallback when an existing `well.toml` can't be read.
fn fresh_manifest() -> WellManifest {
    WellManifest {
        schema: 1,
        sections: SectionFlags {
            notes: true,
            tasks: true,
            wiki: true,
        },
        columns: crate::model::default_columns(),
        archive_done_after_days: None,
        views: Vec::new(),
    }
}

/// Read a well's `.ido/well.toml`, falling back to a default manifest.
pub(crate) fn read_manifest(well: &str) -> WellManifest {
    let path = Path::new(well).join(".ido").join("well.toml");
    fs::read_to_string(path)
        .ok()
        .and_then(|s| toml::from_str(&s).ok())
        .unwrap_or_else(fresh_manifest)
}

/// Root-level `.md` files and non-reserved folders of a legacy (pre-sections)
/// well — the entries migration sweeps into `notes/`. Returns `(name, path)`.
fn legacy_entries(root: &Path) -> Vec<(String, std::path::PathBuf)> {
    let reserved = ["notes", "tasks", "wiki", ".ido", crate::assets::ASSETS_DIR];
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
/// Also upgrades a legacy column set and runs the auto-archive sweep
/// ([`crate::tasks::sweep_archive`]) — so a well opened after time away never
/// shows tasks that should already have aged into the archive. Idempotent —
/// safe on every open.
#[tauri::command]
pub fn migrate_well(well: String) -> Result<(), String> {
    let root = Path::new(&well);
    if !root.join(".ido").join("well.toml").exists() {
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
    }
    upgrade_columns(&well);
    crate::tasks::sweep_archive(&well);
    Ok(())
}

/// Whether `cols` is the original default column set (before `planning` /
/// `in-progress` were introduced).
fn is_legacy_columns(cols: &[String]) -> bool {
    cols.iter()
        .map(String::as_str)
        .eq(["todo", "doing", "done"])
}

/// Upgrade a well still on the original `todo / doing / done` columns to the
/// current default set. Tasks with the now-removed `doing` status become
/// un-columned and surface in the backlog. A no-op once upgraded.
fn upgrade_columns(well: &str) {
    let mut manifest = read_manifest(well);
    if is_legacy_columns(&manifest.columns) {
        manifest.columns = crate::model::default_columns();
        let path = Path::new(well).join(".ido").join("well.toml");
        let _ = fs::write(path, toml::to_string(&manifest).unwrap_or_default());
    }
}

/// Replace the board's columns (task `status` values) in `.ido/well.toml`.
/// Names are slugified — matching how columns are stored (`In Progress` →
/// `in-progress`) — with blanks dropped and duplicates removed, order kept.
/// Errs when nothing valid remains. Returns the stored set. Tasks whose status
/// no longer matches a column simply surface in the backlog (no data changes).
#[tauri::command]
pub fn set_task_columns(well: String, columns: Vec<String>) -> Result<Vec<String>, String> {
    let mut cols: Vec<String> = Vec::new();
    for c in columns {
        let slug = slugify(&c);
        if !slug.is_empty() && !cols.contains(&slug) {
            cols.push(slug);
        }
    }
    if cols.is_empty() {
        return Err("at least one column is needed".into());
    }
    let mut manifest = read_manifest(&well);
    manifest.columns = cols.clone();
    let path = Path::new(&well).join(".ido").join("well.toml");
    fs::write(path, toml::to_string(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(cols)
}

/// Set (or, with `None`, turn off) the well's auto-archive-done-after-N-days
/// setting in `.ido/well.toml`, then immediately run the sweep
/// ([`crate::tasks::sweep_archive`]) — so newly setting it, or lowering the
/// threshold, can archive tasks right away instead of waiting for the well's
/// next open. Returns the stored value.
#[tauri::command]
pub fn set_archive_days(well: String, days: Option<u32>) -> Result<Option<u32>, String> {
    let mut manifest = read_manifest(&well);
    manifest.archive_done_after_days = days;
    let path = Path::new(&well).join(".ido").join("well.toml");
    fs::write(path, toml::to_string(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    crate::tasks::sweep_archive(&well);
    Ok(days)
}

/// The well's saved tasks-toolbar views (`.ido/well.toml`), in stored order.
/// A read like [`crate::tasks::task_columns`] / [`crate::tasks::archive_days`].
#[tauri::command]
pub fn saved_views(well: String) -> Vec<SavedView> {
    read_manifest(&well).views
}

/// Replace the well's saved views wholesale (mirrors [`set_task_columns`]):
/// views whose trimmed name is blank are dropped, the rest persist in order.
/// Returns the stored list.
#[tauri::command]
pub fn set_saved_views(well: String, views: Vec<SavedView>) -> Result<Vec<SavedView>, String> {
    let views: Vec<SavedView> = views
        .into_iter()
        .filter(|v| !v.name.trim().is_empty())
        .collect();
    let mut manifest = read_manifest(&well);
    manifest.views = views.clone();
    let path = Path::new(&well).join(".ido").join("well.toml");
    fs::write(path, toml::to_string(&manifest).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    Ok(views)
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
        assert!(dir.path().join("tasks/goals").is_dir());
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
    fn set_task_columns_slugs_dedupes_and_persists() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        migrate_well(well.clone()).unwrap();
        let stored = set_task_columns(
            well.clone(),
            vec![
                "Backburner".into(),
                "In Progress".into(),
                "  ".into(),
                "in-progress".into(), // duplicate after slugging
                "Done".into(),
            ],
        )
        .unwrap();
        assert_eq!(stored, vec!["backburner", "in-progress", "done"]);
        // Persisted — a fresh manifest read agrees.
        assert_eq!(read_manifest(&well).columns, stored);
        // Nothing valid → an error, and the manifest is untouched.
        assert!(set_task_columns(well.clone(), vec!["!!!".into()]).is_err());
        assert_eq!(read_manifest(&well).columns, stored);
    }

    #[test]
    fn set_archive_days_persists_and_sweeps_immediately() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        migrate_well(well.clone()).unwrap();

        let old = crate::tasks::create_task(well.clone(), "done".into(), "Old task".into(), None)
            .unwrap();
        crate::tasks::set_task_field(
            well.clone(),
            old.clone(),
            "completed".into(),
            "2020-01-01".into(),
        )
        .unwrap();

        // A month-old done task is untouched while the setting is off (the
        // default), then archived the moment the setting is turned on — no
        // need to wait for the well's next open.
        let stored = set_archive_days(well.clone(), Some(30)).unwrap();
        assert_eq!(stored, Some(30));
        assert_eq!(read_manifest(&well).archive_done_after_days, Some(30));
        let tasks = crate::tasks::list_tasks(well.clone());
        assert!(tasks.iter().find(|t| t.id == old).unwrap().archived);

        // Turning it back off persists `None` (already-archived tasks stay
        // archived — this only stops future sweeps).
        let stored = set_archive_days(well.clone(), None).unwrap();
        assert_eq!(stored, None);
        assert_eq!(read_manifest(&well).archive_done_after_days, None);
    }

    #[test]
    fn saved_views_round_trip_through_well_toml() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        migrate_well(well.clone()).unwrap();

        // A fresh well starts with no views.
        assert!(saved_views(well.clone()).is_empty());

        let views = vec![
            SavedView {
                name: "this week".into(),
                view: "calendar".into(),
                filter: "spec".into(),
                tag: Some("ui".into()),
                goal: None,
                hide_done: true,
                sort: "due".into(),
            },
            SavedView {
                name: "review".into(),
                view: "table".into(),
                filter: String::new(),
                tag: None,
                goal: Some("v1".into()),
                hide_done: false,
                sort: "priority".into(),
            },
        ];
        let stored = set_saved_views(well.clone(), views).unwrap();
        assert_eq!(stored.len(), 2);

        // Emitted as `[[views]]` array-of-tables, with the rest of the manifest
        // (scalars + `[sections]`) intact around them.
        let raw = fs::read_to_string(dir.path().join(".ido").join("well.toml")).unwrap();
        assert!(raw.contains("[[views]]"), "no views tables in:\n{raw}");
        assert!(raw.contains("[sections]"), "sections lost in:\n{raw}");

        // A fresh manifest read agrees — order + every field intact (including
        // the skipped `None` scopes).
        let read = saved_views(well.clone());
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].name, "this week");
        assert_eq!(read[0].view, "calendar");
        assert_eq!(read[0].filter, "spec");
        assert_eq!(read[0].tag.as_deref(), Some("ui"));
        assert_eq!(read[0].goal, None);
        assert!(read[0].hide_done);
        assert_eq!(read[0].sort, "due");
        assert_eq!(read[1].name, "review");
        assert_eq!(read[1].goal.as_deref(), Some("v1"));
        assert_eq!(read[1].tag, None);

        // The rest of the manifest still parses alongside the views.
        assert_eq!(
            read_manifest(&well).columns,
            crate::model::default_columns()
        );

        // Writing an empty list clears the views without corrupting the file.
        let stored = set_saved_views(well.clone(), Vec::new()).unwrap();
        assert!(stored.is_empty());
        assert!(saved_views(well.clone()).is_empty());
        // And the manifest is still readable after the empty write.
        assert_eq!(
            read_manifest(&well).columns,
            crate::model::default_columns()
        );
    }

    #[test]
    fn set_saved_views_drops_blank_names() {
        let dir = tempdir().unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        migrate_well(well.clone()).unwrap();

        let stored = set_saved_views(
            well.clone(),
            vec![
                SavedView {
                    name: "  ".into(),
                    view: "board".into(),
                    filter: String::new(),
                    tag: None,
                    goal: None,
                    hide_done: false,
                    sort: "manual".into(),
                },
                SavedView {
                    name: "keep".into(),
                    view: "board".into(),
                    filter: String::new(),
                    tag: None,
                    goal: None,
                    hide_done: false,
                    sort: "manual".into(),
                },
            ],
        )
        .unwrap();
        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].name, "keep");
        assert_eq!(saved_views(well).len(), 1);
    }

    #[test]
    fn manifest_without_views_still_parses() {
        // A well.toml written before the `views` field existed must round-trip:
        // the missing key defaults to an empty vec (and `saved_views` reads it).
        let dir = tempdir().unwrap();
        let ido = dir.path().join(".ido");
        fs::create_dir_all(&ido).unwrap();
        fs::write(
            ido.join("well.toml"),
            "schema = 1\ncolumns = [\"todo\", \"done\"]\n\n[sections]\nnotes = true\ntasks = true\nwiki = true\n",
        )
        .unwrap();
        let well = dir.path().to_string_lossy().into_owned();
        let manifest = read_manifest(&well);
        assert_eq!(manifest.columns, vec!["todo", "done"]);
        assert!(manifest.views.is_empty());
        assert!(saved_views(well).is_empty());
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
