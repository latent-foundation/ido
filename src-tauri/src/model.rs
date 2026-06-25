//! Data shapes shared between the store and the frontend (serialised over IPC).

use serde::{Deserialize, Serialize};

/// A reference to a *well* — a folder the user has opened as a notes root.
#[derive(Serialize, Deserialize, Clone)]
pub struct WellRef {
    /// Absolute path to the well's root folder; the id the frontend passes back.
    pub path: String,
    /// Display name (the folder's own name).
    pub name: String,
}

/// Filesystem timestamps for a note, as Unix-epoch milliseconds.
#[derive(Serialize)]
pub struct NoteMeta {
    /// Creation (birth) time — `None` when the platform/filesystem doesn't record it.
    pub created: Option<u64>,
    /// Last-modified time.
    pub modified: Option<u64>,
}

/// One entry in a well's tree: a folder (with `children`) or a note.
#[derive(Serialize)]
pub struct TreeNode {
    /// Display name — a folder name, or a note's file stem (no `.md`).
    pub name: String,
    /// Path relative to the well root, `/`-separated; for notes, without `.md`.
    /// This is the id used by `read_note` / `rename_entry` / `move_entry` / ….
    pub path: String,
    /// Whether this entry is a folder.
    pub is_dir: bool,
    /// Folder contents (folders first, then notes; both alphabetical). Empty for notes.
    pub children: Vec<TreeNode>,
}
