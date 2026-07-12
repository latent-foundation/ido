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

/// A kanban task. Mirrors the backend `Task`: a markdown file under `tasks/`
/// whose frontmatter holds the board metadata. `id` is the file stem (slug).
#[derive(Clone, Serialize, Deserialize)]
pub struct Task {
    /// File stem (slug) — the task's id.
    pub id: String,
    /// Display title (free text; the backend falls back to the id).
    pub title: String,
    /// Column id (a `status` value); `""` if unset.
    pub status: String,
    /// Priority: `""` / `low` / `normal` / `high`.
    pub priority: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Sort key within a column.
    pub order: i64,
    /// Due date `YYYY-MM-DD`, or `""`.
    pub due: String,
    /// The goal this task belongs to, by slug; `""` if none.
    pub goal: String,
    /// Recurrence rule (`daily` / `weekly` / `every N days` / …); `""` if the
    /// task is non-recurring.
    #[serde(default)]
    pub repeat: String,
    /// Whether the task is archived (hidden from the board).
    pub archived: bool,
    /// The date the task entered the done column (`YYYY-MM-DD`), or `""`. Stamped
    /// backend-side on completion; surfaced in the table view's completed column.
    /// `#[serde(default)]` so it can land either side first.
    #[serde(default)]
    pub completed: String,
    /// Number of checked `- [x]` task-list items in the body — the card's
    /// checklist rollup chip. `#[serde(default)]` so it can land either side first.
    #[serde(default)]
    pub checks_done: u32,
    /// Total number of task-list items (`- [ ]` + `- [x]`) in the body.
    #[serde(default)]
    pub checks_total: u32,
    /// Markdown body.
    pub body: String,
}

/// A goal (milestone) — mirrors the backend `Goal`. A markdown file under
/// `tasks/goals/`; progress is derived from the tasks that target it.
#[derive(Clone, Serialize, Deserialize)]
pub struct Goal {
    /// File stem (slug) — the goal's id (what tasks' `goal:` fields reference).
    pub id: String,
    /// Display title (free text; the backend falls back to the id).
    pub title: String,
    /// Target date `YYYY-MM-DD`, or `""`.
    pub target: String,
    /// Sort key in the goals bar (drag-to-reorder).
    pub order: i64,
    /// Whether the goal is archived (hidden from the goals bar).
    pub archived: bool,
    /// Markdown body.
    pub body: String,
}

/// A saved tasks-toolbar preset — mirrors the backend `SavedView`. Snapshots
/// the six toolbar controls; the board's "views" dropdown applies one all at
/// once (`State::apply_view`). `view` / `sort` are plain strings mapped to the
/// `TaskView` / `TaskSort` enums (unknown → board / manual); `tag` / `goal` are
/// the scope filters (`None` = unscoped). Field names match the backend exactly
/// (plain snake_case — no `rename_all`, since this rides inside a command arg
/// rather than being a top-level param).
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedView {
    /// Display name — the dropdown row label + the save key (an exact,
    /// case-sensitive name match replaces on save).
    pub name: String,
    /// The `TaskView` variant, as a string (`board` / `calendar` / `table`).
    pub view: String,
    /// The free-text filter box contents (`""` = no text filter).
    pub filter: String,
    /// The tag scope, if any (`None` = all tags).
    pub tag: Option<String>,
    /// The goal scope, by goal id, if any (`None` = all goals).
    pub goal: Option<String>,
    /// Whether the done (last) column is hidden.
    pub hide_done: bool,
    /// The `TaskSort` variant, as a string (`manual` / `priority` / `due`).
    pub sort: String,
}

/// A backlink source — whatever links to the page in view. Mirrors the backend
/// `LinkRef`; `kind` (`note` / `wiki` / `task` / `goal`) selects the open action.
#[derive(Clone, Serialize, Deserialize)]
pub struct LinkRef {
    /// Which section: `"note"`, `"wiki"`, `"task"`, or `"goal"`.
    pub kind: String,
    /// The id to open: note id (path), wiki slug, or task/goal id.
    pub id: String,
    /// Display title — file stem, slug, or a task/goal's `title:`.
    pub title: String,
}

/// A cross-section search result — mirrors the backend `SearchHit`. `kind`
/// (`note` / `wiki` / `task`) selects the open action; `id` is what to open.
#[derive(Clone, Serialize, Deserialize)]
pub struct SearchHit {
    /// Which section: `"note"`, `"wiki"`, or `"task"`.
    pub kind: String,
    /// The id to open: note id (path), wiki slug, or task id.
    pub id: String,
    /// Display title — file stem or slug.
    pub title: String,
    /// A short context excerpt around the match (may be empty).
    pub snippet: String,
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
