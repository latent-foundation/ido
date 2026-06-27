//! Data shapes shared between the store and the frontend (serialised over IPC).

use serde::{Deserialize, Serialize};

/// One of the three areas inside a well. Each maps to a reserved subfolder.
#[derive(Clone, Copy)]
pub enum Section {
    Notes,
    Tasks,
    Wiki,
}

impl Section {
    /// Every section, in rail order — used to scaffold a well's folders.
    pub const ALL: [Section; 3] = [Section::Notes, Section::Tasks, Section::Wiki];

    /// The reserved folder name backing this section.
    pub fn dir(self) -> &'static str {
        match self {
            Section::Notes => "notes",
            Section::Tasks => "tasks",
            Section::Wiki => "wiki",
        }
    }
}

/// The `.ido/well.toml` manifest: a schema version plus which sections are on.
/// Written when a well is scaffolded; its presence marks a well as migrated.
#[derive(Serialize, Deserialize)]
pub struct WellManifest {
    /// Schema version, reserved for future migrations.
    pub schema: u32,
    /// Which sections are enabled in this well.
    pub sections: SectionFlags,
}

/// Per-section enable flags (all `true` for a freshly scaffolded well).
#[derive(Serialize, Deserialize)]
pub struct SectionFlags {
    pub notes: bool,
    pub tasks: bool,
    pub wiki: bool,
}

/// The per-well editor session — which tabs are open and which is active.
/// A rebuildable cache in `.ido/session.toml`; a missing or unparseable file
/// just yields no tabs. (Increment 1: every tab is a note, stored by id.)
#[derive(Serialize, Deserialize, Default)]
pub struct Session {
    /// Open tabs as note ids, in strip order.
    #[serde(default)]
    pub tabs: Vec<String>,
    /// Index of the active tab, if any.
    #[serde(default)]
    pub active: Option<usize>,
}

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
