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

/// Filesystem timestamps for a note, as Unix-epoch milliseconds.
#[derive(Clone, Serialize, Deserialize)]
pub struct NoteMeta {
    /// Creation (birth) time — `None` when the platform doesn't record it.
    pub created: Option<u64>,
    /// Last-modified time.
    pub modified: Option<u64>,
}

/// The per-well editor session — open tabs (by note id) + the active index.
/// Mirrors the backend `Session`; a rebuildable cache in `.ido/session.toml`.
#[derive(Clone, Serialize, Deserialize)]
pub struct Session {
    /// Open tabs as note ids, in strip order.
    pub tabs: Vec<String>,
    /// Index of the active tab, if any.
    pub active: Option<usize>,
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
