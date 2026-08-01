//! Notes and folders within a well: building the tree and CRUD over the markdown
//! files. A note's name is its file stem — independent of the body text.
//!
//! Every command is well-scoped: it takes the well's absolute path plus an `id`
//! (a well-relative, `/`-separated path; for notes, without `.md`).

use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::model::{NoteMeta, Section, TreeNode};
use crate::paths::{
    join_rel, note_path, parent_of, rel_path, section_dir, unique_name, valid_name,
};

/// Recursively read `dir` into [`TreeNode`]s (folders first, then notes, each
/// alphabetical). Hidden entries (dot-prefixed) and non-`.md` files are skipped.
///
/// `pub(crate)` because it's a generic "folders + `.md` files → tree" reader with
/// no notes-specific logic — the wiki section reuses it for its own tree so the
/// two stay in lockstep.
pub(crate) fn build_tree(dir: &Path, well: &Path) -> Vec<TreeNode> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let raw = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if raw.is_empty() || raw.starts_with('.') {
            continue;
        }
        if path.is_dir() {
            dirs.push(TreeNode {
                name: raw.to_string(),
                path: rel_path(well, &path),
                is_dir: true,
                children: build_tree(&path, well),
            });
        } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            files.push(TreeNode {
                name: stem,
                path: rel_path(well, &path.with_extension("")),
                is_dir: false,
                children: Vec::new(),
            });
        }
    }
    dirs.sort_by_key(|n| n.name.to_lowercase());
    files.sort_by_key(|n| n.name.to_lowercase());
    dirs.into_iter().chain(files).collect()
}

/// Whether `a` and `b` resolve to the **same** on-disk entry. On a
/// case-insensitive filesystem (Windows/macOS default) a case-only rename like
/// `Notes` → `notes` has `b.exists() == true` even though it's the very file
/// being renamed; comparing canonical paths lets that recasing through instead
/// of a false "name already taken". `pub(crate)`: the wiki section's folder
/// rename/move reuses it for the same case-insensitive-FS guard.
pub(crate) fn same_entry(a: &Path, b: &Path) -> bool {
    match (fs::canonicalize(a), fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
}

/// The notes section's whole tree of folders and notes.
#[tauri::command]
pub fn list_tree(well: String) -> Result<Vec<TreeNode>, String> {
    let root = section_dir(&well, Section::Notes);
    Ok(build_tree(&root, &root))
}

/// Read a note's markdown body.
#[tauri::command]
pub fn read_note(well: String, id: String) -> Result<String, String> {
    let root = section_dir(&well, Section::Notes);
    fs::read_to_string(note_path(&root, &id)).map_err(|e| e.to_string())
}

/// `SystemTime` as Unix-epoch milliseconds, or `None` for pre-epoch times.
fn to_millis(t: SystemTime) -> Option<u64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .map(|d| d.as_millis() as u64)
}

/// A note's filesystem timestamps (creation + last-modified).
#[tauri::command]
pub fn note_meta(well: String, id: String) -> Result<NoteMeta, String> {
    let root = section_dir(&well, Section::Notes);
    let meta = fs::metadata(note_path(&root, &id)).map_err(|e| e.to_string())?;
    Ok(NoteMeta {
        created: meta.created().ok().and_then(to_millis),
        modified: meta.modified().ok().and_then(to_millis),
    })
}

/// Write a note's markdown body, creating any missing parent folders.
#[tauri::command]
pub fn write_note(well: String, id: String, content: String) -> Result<(), String> {
    let root = section_dir(&well, Section::Notes);
    let path = note_path(&root, &id);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::write(path, content).map_err(|e| e.to_string())
}

/// Create a uniquely-named empty note in `parent` (`""` = root). Returns its id.
#[tauri::command]
pub fn create_note(well: String, parent: String) -> Result<String, String> {
    let root = section_dir(&well, Section::Notes);
    let dir = if parent.is_empty() {
        root
    } else {
        root.join(&parent)
    };
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let stem = unique_name(&dir, "untitled", "md");
    fs::write(dir.join(format!("{stem}.md")), "").map_err(|e| e.to_string())?;
    Ok(join_rel(&parent, &stem))
}

/// Create a uniquely-named folder in `parent` (`""` = root). Returns its id.
#[tauri::command]
pub fn create_folder(well: String, parent: String) -> Result<String, String> {
    let root = section_dir(&well, Section::Notes);
    let base = if parent.is_empty() {
        root
    } else {
        root.join(&parent)
    };
    let name = unique_name(&base, "new folder", "");
    fs::create_dir_all(base.join(&name)).map_err(|e| e.to_string())?;
    Ok(join_rel(&parent, &name))
}

/// Rename a note or folder in place (parent unchanged). Returns the new id.
#[tauri::command]
pub fn rename_entry(
    well: String,
    id: String,
    is_dir: bool,
    name: String,
) -> Result<String, String> {
    let name = valid_name(&name)?;
    let new_id = join_rel(&parent_of(&id), name);
    if new_id == id {
        return Ok(id);
    }
    let root = section_dir(&well, Section::Notes);
    let (old, new) = if is_dir {
        (root.join(&id), root.join(&new_id))
    } else {
        (
            root.join(format!("{id}.md")),
            root.join(format!("{new_id}.md")),
        )
    };
    // Allow a case-only recasing (same file on a case-insensitive FS) through.
    if new.exists() && !same_entry(&old, &new) {
        return Err("name already taken".into());
    }
    fs::rename(old, new).map_err(|e| e.to_string())?;
    Ok(new_id)
}

/// Delete a note, or an *empty* folder. Non-empty folders are refused so notes
/// are never destroyed implicitly.
#[tauri::command]
pub fn delete_entry(well: String, id: String, is_dir: bool) -> Result<(), String> {
    let root = section_dir(&well, Section::Notes);
    if is_dir {
        let path = root.join(&id);
        // Report the real "not empty" case as such; surface any other I/O error
        // (permissions, missing) verbatim rather than mislabelling it.
        if fs::read_dir(&path)
            .map(|mut e| e.next().is_some())
            .unwrap_or(false)
        {
            return Err("folder isn't empty".into());
        }
        fs::remove_dir(path).map_err(|e| e.to_string())
    } else {
        fs::remove_file(root.join(format!("{id}.md"))).map_err(|e| e.to_string())
    }
}

/// Move a note or folder into the `dest` folder (`""` = well root). Returns the
/// new id. Refuses to move a folder into itself or a descendant.
#[tauri::command]
pub fn move_entry(well: String, id: String, is_dir: bool, dest: String) -> Result<String, String> {
    let base = id.rsplit('/').next().unwrap_or(&id);
    let new_id = join_rel(&dest, base);
    if new_id == id {
        return Ok(id);
    }
    if is_dir && (dest == id || dest.starts_with(&format!("{id}/"))) {
        return Err("can't move a folder into itself".into());
    }
    let root = section_dir(&well, Section::Notes);
    let (old, new) = if is_dir {
        (root.join(&id), root.join(&new_id))
    } else {
        (
            root.join(format!("{id}.md")),
            root.join(format!("{new_id}.md")),
        )
    };
    if new.exists() && !same_entry(&old, &new) {
        return Err("an item with that name already exists there".into());
    }
    if let Some(parent) = new.parent() {
        let _ = fs::create_dir_all(parent);
    }
    fs::rename(old, new).map_err(|e| e.to_string())?;
    Ok(new_id)
}

// The commands take plain `String` args (no `AppHandle`/`Window`), so they are
// callable directly here against a temp-dir well — no mock runtime needed.
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::{tempdir, TempDir};

    /// A fresh empty well in a temp dir. Keep the `TempDir` alive for the test.
    fn well() -> (TempDir, String) {
        let dir = tempdir().unwrap();
        let path = dir.path().to_string_lossy().into_owned();
        (dir, path)
    }

    #[test]
    fn create_list_read_write() {
        let (_d, w) = well();
        assert!(list_tree(w.clone()).unwrap().is_empty());

        let id = create_note(w.clone(), String::new()).unwrap();
        assert_eq!(id, "untitled");
        // names are unique and independent of content
        let id2 = create_note(w.clone(), String::new()).unwrap();
        assert_eq!(id2, "untitled-2");

        let tree = list_tree(w.clone()).unwrap();
        assert_eq!(tree.len(), 2);
        assert!(tree.iter().all(|n| !n.is_dir));
        assert_eq!(tree[0].name, "untitled");
        assert_eq!(tree[0].path, "untitled");

        write_note(w.clone(), id.clone(), "# Hello\n\nbody".into()).unwrap();
        assert_eq!(read_note(w.clone(), id.clone()).unwrap(), "# Hello\n\nbody");
        // the body changed but the name (file stem) did not
        let tree = list_tree(w.clone()).unwrap();
        assert_eq!(tree.iter().find(|n| n.path == id).unwrap().name, "untitled");
    }

    #[test]
    fn note_meta_reports_timestamps() {
        let (_d, w) = well();
        let id = create_note(w.clone(), String::new()).unwrap();
        let meta = note_meta(w.clone(), id).unwrap();
        // `modified` is universally available; `created` is best-effort (some
        // filesystems don't record a birth time, so it may be `None`).
        assert!(meta.modified.is_some());
        // A missing note is an error, not empty metadata.
        assert!(note_meta(w, "nope".into()).is_err());
    }

    #[test]
    fn folders_nest_and_sort() {
        let (_d, w) = well();
        let folder = create_folder(w.clone(), String::new()).unwrap();
        assert_eq!(folder, "new folder");

        let note = create_note(w.clone(), folder.clone()).unwrap();
        assert_eq!(note, "new folder/untitled");
        // a root-level note too, to check folders sort before files
        create_note(w.clone(), String::new()).unwrap();

        let tree = list_tree(w.clone()).unwrap();
        assert_eq!(tree.len(), 2);
        assert!(tree[0].is_dir, "folders come first");
        assert_eq!(tree[0].name, "new folder");
        assert_eq!(tree[0].children.len(), 1);
        assert_eq!(tree[0].children[0].path, "new folder/untitled");
        assert!(!tree[1].is_dir);
    }

    #[test]
    fn rename_note() {
        let (_d, w) = well();
        let id = create_note(w.clone(), String::new()).unwrap();
        let new_id = rename_entry(w.clone(), id.clone(), false, "hello".into()).unwrap();
        assert_eq!(new_id, "hello");
        assert!(read_note(w.clone(), id).is_err());
        assert!(read_note(w.clone(), new_id).is_ok());

        // renaming onto an existing name is rejected
        let other = create_note(w.clone(), String::new()).unwrap();
        assert!(rename_entry(w.clone(), other, false, "hello".into()).is_err());
    }

    #[test]
    fn rename_recasing_is_allowed() {
        let (_d, w) = well();
        let id = create_note(w.clone(), String::new()).unwrap();
        // Changing only the case must not be rejected as "name already taken"
        // (on a case-insensitive filesystem the target path already "exists").
        let new = rename_entry(w.clone(), id, false, "Untitled".into()).unwrap();
        assert_eq!(new, "Untitled");
        assert!(read_note(w, "Untitled".into()).is_ok());
    }

    #[test]
    fn delete_note_and_empty_folder_only() {
        let (_d, w) = well();
        let id = create_note(w.clone(), String::new()).unwrap();
        delete_entry(w.clone(), id.clone(), false).unwrap();
        assert!(read_note(w.clone(), id).is_err());

        let folder = create_folder(w.clone(), String::new()).unwrap();
        let inside = create_note(w.clone(), folder.clone()).unwrap();
        // non-empty folder is protected
        assert!(delete_entry(w.clone(), folder.clone(), true).is_err());
        delete_entry(w.clone(), inside, false).unwrap();
        assert!(delete_entry(w.clone(), folder, true).is_ok());
    }

    #[test]
    fn move_between_folders() {
        let (_d, w) = well();
        let folder = create_folder(w.clone(), String::new()).unwrap();
        let id = create_note(w.clone(), String::new()).unwrap();

        let moved = move_entry(w.clone(), id.clone(), false, folder.clone()).unwrap();
        assert_eq!(moved, "new folder/untitled");
        assert!(read_note(w.clone(), id).is_err());
        assert!(read_note(w.clone(), moved.clone()).is_ok());

        let back = move_entry(w.clone(), moved, false, String::new()).unwrap();
        assert_eq!(back, "untitled");

        // a folder can't be moved into itself
        assert!(move_entry(w.clone(), folder.clone(), true, folder).is_err());
    }
}
