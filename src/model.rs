//! Data shapes that cross the IPC boundary and flow through the UI. Mirrors the
//! backend's `model` (see `src-tauri/src/model.rs`).

use serde::{Deserialize, Serialize};

/// A reference to a *well* — a folder opened as a notes root.
#[derive(Clone, Serialize, Deserialize)]
pub struct WellRef {
    /// Absolute path to the well's root folder; the id passed back to the backend.
    pub path: String,
    /// Display name (the folder's own name).
    pub name: String,
}

/// One entry in a well's tree: a folder (with `children`) or a note.
#[derive(Clone, Serialize, Deserialize)]
pub struct TreeNode {
    /// Display name — a folder name, or a note's file stem (no `.md`).
    pub name: String,
    /// Well-relative, `/`-separated id; for notes, without `.md`.
    pub path: String,
    /// Whether this entry is a folder.
    pub is_dir: bool,
    /// Folder contents (folders first, then notes). Empty for notes.
    pub children: Vec<TreeNode>,
}
