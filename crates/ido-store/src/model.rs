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

/// The `.ido/well.toml` manifest: a schema version, which sections are on, and
/// the task board's columns. Written when a well is scaffolded; its presence
/// marks a well as migrated.
#[derive(Serialize, Deserialize)]
pub struct WellManifest {
    /// Schema version, reserved for future migrations.
    pub schema: u32,
    /// Which sections are enabled in this well.
    pub sections: SectionFlags,
    /// The kanban columns (task `status` values), in board order. Defaulted so
    /// wells scaffolded before this field round-trip cleanly.
    #[serde(default = "default_columns")]
    pub columns: Vec<String>,
    /// Auto-archive done tasks older than this many days; `None` (the default,
    /// for wells written before this field existed) means off. See
    /// `tasks::sweep_archive`.
    #[serde(default)]
    pub archive_done_after_days: Option<u32>,
    /// The tasks toolbar's saved views (the board's "views" dropdown), in stored
    /// order. Defaulted so wells written before this field round-trip cleanly.
    #[serde(default)]
    pub views: Vec<SavedView>,
}

/// A saved snapshot of the tasks toolbar — applied all at once from the board's
/// "views" dropdown. Durable per-well config in `.ido/well.toml` (like
/// `columns`). The store keeps it verbatim and never interprets it: `view` /
/// `sort` are plain strings the frontend maps back to its `TaskView` /
/// `TaskSort` enums (falling back to board / manual on anything unknown), and a
/// `tag` / `goal` scope that no longer exists is applied as a filter that
/// simply matches nothing (deliberately not validated). Field names match the
/// frontend mirror exactly — plain snake_case, so no `rename_all` is needed
/// (Tauri's camelCase mapping only touches top-level command params, not this
/// nested struct).
#[derive(Serialize, Deserialize, Clone)]
pub struct SavedView {
    /// Display name — the dropdown row label and the save key (saving under an
    /// existing name replaces it).
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

/// The default task board columns for a fresh well. Column ids are slugs;
/// `in-progress` renders as "in progress".
pub fn default_columns() -> Vec<String> {
    vec![
        "todo".to_string(),
        "planning".into(),
        "in-progress".into(),
        "done".into(),
    ]
}

/// Per-section enable flags (all `true` for a freshly scaffolded well).
#[derive(Serialize, Deserialize)]
pub struct SectionFlags {
    pub notes: bool,
    pub tasks: bool,
    pub wiki: bool,
}

/// A task: a markdown file under `tasks/`, its frontmatter parsed into fields.
/// Its `id` is the file stem (slug); `status` names the column it sits in.
#[derive(Serialize, Deserialize, Clone)]
pub struct Task {
    /// File stem (slug) — the task's id.
    pub id: String,
    /// Display title (free text, `title:` frontmatter). Falls back to the id
    /// for files without one, so pre-title tasks still show a name.
    pub title: String,
    /// Column id (a `status` value from the manifest); `""` if unset.
    pub status: String,
    /// Priority: `""` / `low` / `normal` / `high`.
    pub priority: String,
    /// Free-form labels.
    pub tags: Vec<String>,
    /// Sort key within a column (ascending).
    pub order: i64,
    /// Due date as `YYYY-MM-DD`, or `""`.
    pub due: String,
    /// The goal (milestone) this task belongs to, by slug; `""` if none.
    pub goal: String,
    /// Recurrence rule (`daily` / `weekly` / `every N days` / …); `""` if the
    /// task is non-recurring. On completion a recurring task spawns its next
    /// occurrence (see `tasks::spawn_next`).
    pub repeat: String,
    /// Whether the task is archived (hidden from the board, kept on disk).
    pub archived: bool,
    /// The date the task entered the done column (`YYYY-MM-DD`), or `""` if it
    /// isn't complete. Stamped / cleared by `tasks::apply_status`; surfaced in
    /// the table view.
    pub completed: String,
    /// Number of checked `- [x]` task-list items in the body (see
    /// `tasks::count_checks`) — the cards' checklist rollup chip.
    pub checks_done: u32,
    /// Total number of task-list items (`- [ ]` + `- [x]`) in the body.
    pub checks_total: u32,
    /// Markdown body (everything after the frontmatter).
    pub body: String,
}

/// A goal (a.k.a. milestone): a markdown file under `tasks/goals/`. Tasks join
/// it via their `goal:` field; progress (done / total) is derived from those
/// tasks, never stored. Its `id` is the file stem (slug).
#[derive(Serialize, Deserialize, Clone)]
pub struct Goal {
    /// File stem (slug) — the goal's id (what tasks' `goal:` fields reference).
    pub id: String,
    /// Display title (free text, `title:` frontmatter); falls back to the id.
    pub title: String,
    /// Target date `YYYY-MM-DD`, or `""` (a dated goal is a milestone).
    pub target: String,
    /// Sort key in the goals bar (ascending; drag-to-reorder renumbers it).
    pub order: i64,
    /// Whether the goal is archived (hidden from the goals bar, kept on disk).
    pub archived: bool,
    /// Markdown body (description / definition of done).
    pub body: String,
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

/// A reference to a document — a backlink source (whatever links to the page
/// in view). `kind` selects the open action.
#[derive(Serialize)]
pub struct LinkRef {
    /// Which section the source lives in: `"note"`, `"wiki"`, `"task"`, or `"goal"`.
    pub kind: String,
    /// The id to open: note id (path), wiki slug, or task/goal id.
    pub id: String,
    /// Display title — file stem, slug, or a task/goal's `title:`.
    pub title: String,
}

/// One cross-section search result. `kind` (`note` / `wiki` / `task`) selects
/// the open action; `id` is the note id / wiki slug / task id to open; `snippet`
/// is a short context excerpt (the first matching line of the body).
#[derive(Serialize)]
pub struct SearchHit {
    /// Which section the hit lives in: `"note"`, `"wiki"`, or `"task"`.
    pub kind: String,
    /// The id to open: note id (path), wiki slug, or task id.
    pub id: String,
    /// Display title — the note/task file stem or wiki slug.
    pub title: String,
    /// A short context excerpt around the match (may be empty for title-only hits).
    pub snippet: String,
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
