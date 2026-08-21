//! The app's reactive state and the actions that mutate it.
//!
//! [`State`] bundles every signal into one `Copy` handle, provided through
//! Leptos context so components read it with `expect_context::<State>()` instead
//! of prop-drilling. All backend work lives in action methods, which call
//! [`crate::ipc`] and update the signals — keeping components purely declarative.
//!
//! The value types live in [`types`]; the action methods are split across
//! sibling modules by area ([`wells`], [`search`], [`notes`], [`wiki`], [`tasks`],
//! [`editor`], [`assets`]), each an `impl State` block. Cross-section helpers
//! (pane loading, session persistence, toasts) stay here so every submodule can
//! reach them.

use std::collections::{HashMap, HashSet};
use std::time::Duration;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::blocks;
use crate::dates;
use crate::ipc;
use crate::model::{Goal, SavedView, SearchHit, Task, TreeNode, WellRef};

mod assets;
mod editor;
mod notes;
mod search;
mod tasks;
mod types;
mod wells;
mod wiki;

pub use types::*;

/// The fixed command-palette commands: `(id, label, lucide-icon)`. They ride the
/// search results (kind `"cmd"`); [`State::run_command`] dispatches on the id.
pub const COMMANDS: [(&str, &str, &str); 5] = [
    ("cmd:new-note", "new note", "file-plus"),
    ("cmd:new-page", "new wiki page", "book"),
    ("cmd:new-task", "new task", "square-kanban"),
    ("cmd:theme", "toggle theme", "sun-moon"),
    ("cmd:settings", "settings", "settings"),
];

/// Collect every page slug (a non-folder node's `name`) in a wiki tree, depth
/// first — the flat list behind `State::wiki`.
fn collect_slugs(nodes: &[TreeNode], out: &mut Vec<String>) {
    for n in nodes {
        if n.is_dir {
            collect_slugs(&n.children, out);
        } else {
            out.push(n.name.clone());
        }
    }
}

/// Command entries whose label matches `query` (all when empty), as synthetic
/// `SearchHit`s so they appear in the palette alongside content hits.
fn command_hits(query: &str) -> Vec<SearchHit> {
    let q = query.trim().to_lowercase();
    COMMANDS
        .iter()
        .filter(|(_, label, _)| q.is_empty() || label.contains(q.as_str()))
        .map(|(id, label, _)| SearchHit {
            kind: "cmd".into(),
            id: (*id).to_string(),
            title: (*label).to_string(),
            snippet: String::new(),
        })
        .collect()
}

/// All of the app's reactive state. `Copy` (every field is a signal handle), so
/// action methods take `self` by value and closures can freely capture it.
#[derive(Clone, Copy)]
pub struct State {
    // --- well & navigation ------------------------------------------------
    /// The open well, or `None` on the launch screen.
    pub well: RwSignal<Option<WellRef>>,
    /// The active section within the open well (rail selection).
    pub section: RwSignal<Section>,
    /// Recently opened wells (launch screen).
    pub recents: RwSignal<Vec<WellRef>>,

    // --- section content --------------------------------------------------
    /// The open well's tree of folders and notes (the notes section).
    pub tree: RwSignal<Vec<TreeNode>>,
    /// Every wiki page's slug, flattened across all folders — backs existence
    /// checks (open-on-click, undo). Kept in sync with `wiki_tree` by `set_wiki`.
    pub wiki: RwSignal<Vec<String>>,
    /// The wiki section's tree of organisational folders and pages (page node
    /// `name` = slug; folders are display-only). Drives the wiki sidebar.
    pub wiki_tree: RwSignal<Vec<TreeNode>>,

    // --- image attachments --------------------------------------------------
    /// Cache of resolved asset `data:` URLs for the open well, keyed by
    /// well-relative id (`assets/foo.png`) — populated lazily as images are
    /// first rendered (see `State::resolve_asset`); cleared on well switch.
    pub assets: RwSignal<HashMap<String, String>>,
    /// Ids currently being fetched, so a burst of re-renders (every keystroke)
    /// doesn't fire duplicate concurrent reads for the same id.
    pub assets_pending: RwSignal<HashSet<String>>,

    // --- tasks & goals (the board) ----------------------------------------
    /// All tasks in the well — the kanban board's data.
    pub tasks: RwSignal<Vec<Task>>,
    /// The board's columns (task `status` values), in order.
    pub columns: RwSignal<Vec<String>>,
    /// The auto-archive-done-after-N-days setting (`.ido/well.toml`); `None` =
    /// off. Mirrors `columns` — fetched on well open, written via
    /// `State::set_archive_days`.
    pub archive_days: RwSignal<Option<u32>>,
    /// The well's saved tasks-toolbar views (`.ido/well.toml`), in stored order.
    /// The board's "views" dropdown lists these; `apply_view` sets the six
    /// toolbar signals from one, `save_view` / `delete_view` persist the list.
    /// Fetched on well open, like `columns`.
    pub saved_views: RwSignal<Vec<SavedView>>,
    /// Id of the task open in the detail drawer, if any.
    pub active_task: RwSignal<Option<String>>,
    /// All goals (milestones) in the well.
    pub goals: RwSignal<Vec<Goal>>,
    /// The goal the board is scoped to, if any (`None` = all tasks).
    pub goal_filter: RwSignal<Option<String>>,
    /// The tag the board/backlog/calendar are scoped to, if any (`None` = all
    /// tags). Toggled by clicking a tag chip on a card/backlog row; exact,
    /// case-sensitive match (tags render as typed).
    pub tag_filter: RwSignal<Option<String>>,
    /// Id of the goal open in the detail drawer, if any.
    pub active_goal: RwSignal<Option<String>>,
    /// Id of the goal chip being dragged in the goals bar, if any.
    pub goal_drag: RwSignal<Option<String>>,
    /// Goal id a dragged chip would be inserted *before*; `None` = append.
    pub goal_drop_before: RwSignal<Option<String>>,
    /// Id of the task card being dragged, if any.
    pub task_drag: RwSignal<Option<String>>,
    /// Drop target a dragged card is hovering: a column name, or `""` = backlog.
    pub task_drag_over: RwSignal<Option<String>>,
    /// Card id a dragged card would be inserted *before* (manual sort); `None`
    /// = append to the hovered column's end. Drives the insertion line.
    pub task_drop_before: RwSignal<Option<String>>,
    /// Board search filter (matches task id + tags); empty = no filter.
    pub task_filter: RwSignal<String>,
    /// Within-column sort order for the board.
    pub task_sort: RwSignal<TaskSort>,
    /// Whether to hide cards in the done (last) column.
    pub hide_done: RwSignal<bool>,
    /// Whether the tasks section shows the archive view instead of the board.
    pub show_archive: RwSignal<bool>,
    /// Which tasks surface is showing: the board or the month calendar.
    /// Session-local (always opens on the board).
    pub task_view: RwSignal<TaskView>,
    /// The calendar's viewed month as `(year, month0)`. Starts on the current
    /// month; session-local, not persisted.
    pub cal_month: RwSignal<(i32, u32)>,
    /// The calendar day (`YYYY-MM-DD`) a dragged task chip is hovering — drives
    /// the drop-highlight class on that day's cell. Cleared with the card drag.
    pub cal_drag_day: RwSignal<Option<String>>,

    // --- editor panes & tabs ----------------------------------------------
    /// The editor panes (`[0]` primary, `[1]` the optional split). Each holds its
    /// own tabs + active document; tabs are global across sections within a pane.
    pub panes: [Pane; 2],
    /// Whether the second pane is shown (split view active).
    pub split: RwSignal<bool>,
    /// Index of the focused pane (`0`/`1`) — the target of sidebar opens, keyboard
    /// shortcuts, and the sidebar highlight.
    pub focused: RwSignal<usize>,
    /// The tab being dragged, as `(pane, tab index)`, if any.
    pub tab_drag: RwSignal<Option<(usize, usize)>>,
    /// Where the dragged tab would land — drives the drop indicator.
    pub tab_drop: RwSignal<Option<TabDrop>>,

    // --- notes-tree sidebar editing ---------------------------------------
    /// Ids of expanded notes folders.
    pub expanded: RwSignal<HashSet<String>>,
    /// `(id, is_dir)` of the notes-tree entry being renamed inline, if any.
    pub renaming: RwSignal<Option<(String, bool)>>,
    /// `(id, is_dir)` of the notes entry being dragged, if any.
    pub dragging: RwSignal<Option<(String, bool)>>,
    /// Id of the notes folder currently hovered as a drop target.
    pub drag_over: RwSignal<Option<String>>,
    /// Notes folder that new notes/folders are created into (`""` = root).
    pub target: RwSignal<String>,

    // --- wiki-tree sidebar editing ----------------------------------------
    /// Wiki-relative ids of expanded wiki folders.
    pub wiki_expanded: RwSignal<HashSet<String>>,
    /// `(path, is_dir)` of the wiki-tree entry being renamed inline, if any.
    /// A page (`is_dir == false`) renames by its slug (the path's last segment,
    /// rewriting links); a folder renames purely on disk.
    pub wiki_renaming: RwSignal<Option<(String, bool)>>,
    /// `(path, is_dir)` of the wiki entry being dragged, if any.
    pub wiki_dragging: RwSignal<Option<(String, bool)>>,
    /// Wiki-relative id of the wiki folder currently hovered as a drop target.
    pub wiki_drag_over: RwSignal<Option<String>>,
    /// Wiki folder that new pages/folders are created into (`""` = wiki root).
    pub wiki_target: RwSignal<String>,

    // --- create-a-well form (launch screen) -------------------------------
    /// Whether the create-a-well form is showing (launch screen).
    pub creating: RwSignal<bool>,
    /// Draft name in the create-a-well form.
    pub new_name: RwSignal<String>,
    /// Chosen parent location in the create-a-well form.
    pub new_parent: RwSignal<Option<String>>,

    // --- search palette ---------------------------------------------------
    /// Whether the cross-section search palette is open.
    pub search_open: RwSignal<bool>,
    /// The current search query text.
    pub search_query: RwSignal<String>,
    /// Hits for the current query (notes + wiki + tasks).
    pub search_results: RwSignal<Vec<SearchHit>>,
    /// Index of the keyboard-highlighted result.
    pub search_sel: RwSignal<usize>,
    /// Generation counter debouncing the disk scan across rapid keystrokes.
    pub search_gen: RwSignal<u32>,

    // --- transient UI (settings / toast / menu) ---------------------------
    /// Whether the settings modal is open.
    pub settings_open: RwSignal<bool>,
    /// The current transient toast (delete-undo / errors), if any.
    pub toast: RwSignal<Option<Toast>>,
    /// Generation counter so a stale auto-dismiss timer can't clear a newer toast.
    pub toast_gen: RwSignal<u32>,
    /// The open right-click context menu, if any.
    pub menu: RwSignal<Option<Menu>>,
}

impl State {
    /// Create the initial (launcher) state. Must be called within a reactive owner.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            // well & navigation
            well: RwSignal::new(None),
            section: RwSignal::new(Section::Notes),
            recents: RwSignal::new(Vec::new()),

            // section content
            tree: RwSignal::new(Vec::new()),
            wiki: RwSignal::new(Vec::new()),
            wiki_tree: RwSignal::new(Vec::new()),

            // image attachments
            assets: RwSignal::new(HashMap::new()),
            assets_pending: RwSignal::new(HashSet::new()),

            // tasks & goals
            tasks: RwSignal::new(Vec::new()),
            columns: RwSignal::new(Vec::new()),
            archive_days: RwSignal::new(None),
            saved_views: RwSignal::new(Vec::new()),
            active_task: RwSignal::new(None),
            goals: RwSignal::new(Vec::new()),
            goal_filter: RwSignal::new(None),
            tag_filter: RwSignal::new(None),
            active_goal: RwSignal::new(None),
            goal_drag: RwSignal::new(None),
            goal_drop_before: RwSignal::new(None),
            task_drag: RwSignal::new(None),
            task_drag_over: RwSignal::new(None),
            task_drop_before: RwSignal::new(None),
            task_filter: RwSignal::new(String::new()),
            task_sort: RwSignal::new(TaskSort::Manual),
            hide_done: RwSignal::new(false),
            show_archive: RwSignal::new(false),
            task_view: RwSignal::new(TaskView::Board),
            cal_month: {
                let (y, m0, _) = dates::today();
                RwSignal::new((y, m0))
            },
            cal_drag_day: RwSignal::new(None),

            // editor panes & tabs
            panes: [Pane::new(), Pane::new()],
            split: RwSignal::new(false),
            focused: RwSignal::new(0),
            tab_drag: RwSignal::new(None),
            tab_drop: RwSignal::new(None),

            // notes-tree sidebar editing
            expanded: RwSignal::new(HashSet::new()),
            renaming: RwSignal::new(None),
            dragging: RwSignal::new(None),
            drag_over: RwSignal::new(None),
            target: RwSignal::new(String::new()),

            // wiki-tree sidebar editing
            wiki_expanded: RwSignal::new(HashSet::new()),
            wiki_renaming: RwSignal::new(None),
            wiki_dragging: RwSignal::new(None),
            wiki_drag_over: RwSignal::new(None),
            wiki_target: RwSignal::new(String::new()),

            // create-a-well form
            creating: RwSignal::new(false),
            new_name: RwSignal::new(String::new()),
            new_parent: RwSignal::new(None),

            // search palette
            search_open: RwSignal::new(false),
            search_query: RwSignal::new(String::new()),
            search_results: RwSignal::new(Vec::new()),
            search_sel: RwSignal::new(0),
            search_gen: RwSignal::new(0),

            // transient UI
            settings_open: RwSignal::new(false),
            toast: RwSignal::new(None),
            toast_gen: RwSignal::new(0),
            menu: RwSignal::new(None),
        }
    }

    // --- toasts -----------------------------------------------------------

    /// Show a transient toast (auto-dismisses after a few seconds). `undo`, when
    /// present, restores a soft-deleted entry.
    fn show_toast(self, message: String, undo: Option<UndoEntry>) {
        let g = self.toast_gen.get_untracked().wrapping_add(1);
        self.toast_gen.set(g);
        self.toast.set(Some(Toast { message, undo }));
        set_timeout(
            move || {
                if self.toast_gen.get_untracked() == g {
                    self.toast.set(None);
                }
            },
            Duration::from_secs(6),
        );
    }

    /// Dismiss the current toast (and void its pending auto-dismiss).
    pub fn dismiss_toast(self) {
        self.toast_gen.update(|g| *g = g.wrapping_add(1));
        self.toast.set(None);
    }

    /// Undo the current toast's soft-delete: rewrite the file and reopen it.
    pub fn undo_toast(self) {
        let Some(Toast { undo: Some(u), .. }) = self.toast.get_untracked() else {
            self.dismiss_toast();
            return;
        };
        self.dismiss_toast();
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match u.kind {
                EntryKind::Note => {
                    ipc::write_note(w.path.clone(), u.id.clone(), u.content).await;
                    self.tree.set(ipc::list_tree(w.path).await);
                    self.open_note(u.id);
                }
                EntryKind::WikiPage => {
                    ipc::write_page(w.path.clone(), u.id.clone(), u.content).await;
                    self.set_wiki(ipc::list_wiki(w.path).await);
                    self.open_wiki(u.id);
                }
                EntryKind::Task => {
                    ipc::restore_task(w.path.clone(), u.id.clone(), u.content).await;
                    self.tasks.set(ipc::list_tasks(w.path).await);
                    self.section.set(Section::Tasks);
                    self.open_task(u.id);
                }
                EntryKind::Goal => {
                    ipc::restore_goal(w.path.clone(), u.id.clone(), u.content).await;
                    self.goals.set(ipc::list_goals(w.path).await);
                    self.section.set(Section::Tasks);
                    self.open_goal(u.id);
                }
            }
        });
    }

    /// Pane by index (clamped to a valid pane).
    fn pane(self, i: usize) -> Pane {
        self.panes[i.min(1)]
    }

    /// Tear down the second pane and focus the primary (no persist).
    fn end_split(self) {
        self.panes[1].reset();
        self.split.set(false);
        self.focused.set(0);
    }

    /// Collapse the split when a pane has emptied: drop an empty second pane, or
    /// promote the second pane into the first when the first empties.
    fn collapse_if_empty(self) {
        if !self.split.get_untracked() {
            return;
        }
        let p0_empty = self.panes[0].tabs.get_untracked().is_empty();
        let p1_empty = self.panes[1].tabs.get_untracked().is_empty();
        if p1_empty {
            self.end_split();
        } else if p0_empty {
            let tabs = self.panes[1].tabs.get_untracked();
            let active = self.panes[1].active_tab.get_untracked();
            self.panes[0].tabs.set(tabs);
            self.panes[0].active_tab.set(active);
            self.panes[1].reset();
            self.split.set(false);
            self.focused.set(0);
            self.load_pane(self.panes[0]);
        }
    }

    // --- window -----------------------------------------------------------

    /// Fire a no-argument window command (minimize / maximize / close).
    pub fn window_cmd(self, cmd: &'static str) {
        spawn_local(async move { ipc::window_command(cmd).await });
    }

    /// Persist the editor window's current size + maximized state (debounced by
    /// the resize handler in the workspace).
    pub fn remember_window(self) {
        spawn_local(async move { ipc::remember_window().await });
    }

    /// Materialise pane `p`'s active tab into its buffer — reading a note or a
    /// wiki page, plus the note's timestamps or the page's backlinks. Clears the
    /// pane's buffer when no tab is active.
    fn load_pane(self, p: Pane) {
        let tab = p
            .active_tab
            .get_untracked()
            .and_then(|i| p.tabs.get_untracked().get(i).cloned());
        let (Some(tab), Some(w)) = (tab, self.well.get_untracked()) else {
            p.clear_buffer();
            return;
        };
        p.mode.set(tab.mode);
        p.active_block.set(None);
        p.meta.set(None);
        p.backlinks.set(Vec::new());
        let target = tab.target;
        // Tag this load; a later switch bumps the gen, so a slower earlier read
        // that resolves afterwards is dropped instead of clobbering the new tab.
        let this_gen = p.load_gen.get_untracked().wrapping_add(1);
        p.load_gen.set(this_gen);
        spawn_local(async move {
            let body = match &target {
                TabTarget::Note(id) => ipc::read_note(w.path.clone(), id.clone()).await,
                TabTarget::WikiPage(slug) => ipc::read_page(w.path.clone(), slug.clone()).await,
            };
            if p.load_gen.get_untracked() != this_gen {
                return;
            }
            let segs = blocks::segment(&body);
            // Empty docs open with a cursor so no extra click is needed.
            let initial_block = if body.trim().is_empty() {
                Some(0)
            } else {
                None
            };
            p.content.set(body.clone());
            p.blocks.set(segs);
            p.active_block.set(initial_block);
            // Seed the Source textarea directly when it is already in the DOM.
            if let Some(ta) = p.source_editor.get_untracked() {
                ta.set_value(&body);
            }
            let (meta, backlinks) = match target {
                TabTarget::Note(id) => (ipc::note_meta(w.path, id).await, None),
                TabTarget::WikiPage(slug) => (None, Some(ipc::backlinks(w.path, slug).await)),
            };
            if p.load_gen.get_untracked() != this_gen {
                return;
            }
            p.meta.set(meta);
            if let Some(backlinks) = backlinks {
                p.backlinks.set(backlinks);
            }
        });
    }

    /// Reload both panes' open buffers from disk.
    fn reload_panes(self) {
        for p in self.panes {
            self.load_pane(p);
        }
    }

    /// Update both wiki signals from a freshly-listed tree: `wiki_tree` drives
    /// the sidebar, and the flattened `wiki` slug list backs existence checks
    /// (open-on-click, undo). Kept together so they can never drift.
    fn set_wiki(self, tree: Vec<TreeNode>) {
        let mut slugs = Vec::new();
        collect_slugs(&tree, &mut slugs);
        self.wiki.set(slugs);
        self.wiki_tree.set(tree);
    }

    /// Persist `src` to pane `p`'s active tab backing file (note or wiki page).
    fn save_pane(self, p: Pane, src: String) {
        let (Some(w), Some(i)) = (self.well.get_untracked(), p.active_tab.get_untracked()) else {
            return;
        };
        // Editing a preview tab makes it permanent.
        self.pin_in(p, i);
        let Some(target) = p.tabs.get_untracked().get(i).map(|t| t.target.clone()) else {
            return;
        };
        spawn_local(async move {
            match target {
                TabTarget::Note(id) => ipc::write_note(w.path, id, src).await,
                TabTarget::WikiPage(slug) => ipc::write_page(w.path, slug, src).await,
            }
        });
    }

    /// Close every tab (in both panes) whose target matches `gone`, keeping each
    /// pane's previously-active tab focused where it survives; collapse a split
    /// left empty.
    fn close_tabs(self, gone: impl Fn(&TabTarget) -> bool) {
        let mut touched = false;
        for p in self.panes {
            if !p.tabs.get_untracked().iter().any(|t| gone(&t.target)) {
                continue;
            }
            touched = true;
            let active_target = p
                .active_tab
                .get_untracked()
                .and_then(|i| p.tabs.get_untracked().get(i).map(|t| t.target.clone()));
            p.tabs.update(|tabs| tabs.retain(|t| !gone(&t.target)));
            let tabs = p.tabs.get_untracked();
            let new_active = active_target
                .and_then(|tgt| tabs.iter().position(|t| t.target == tgt))
                .or(if tabs.is_empty() { None } else { Some(0) });
            p.active_tab.set(new_active);
            self.load_pane(p);
        }
        if touched {
            self.collapse_if_empty();
            self.persist_session();
        }
    }

    /// Persist the primary pane's tabs + active index to `.ido/session.toml`,
    /// each encoded `"note:<id>"` / `"wiki:<slug>"`. The split is a within-session
    /// view — only the primary pane is restored on relaunch.
    fn persist_session(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        let p = self.panes[0];
        let tabs: Vec<String> = p
            .tabs
            .get_untracked()
            .iter()
            .map(|t| match &t.target {
                TabTarget::Note(id) => format!("note:{id}"),
                TabTarget::WikiPage(slug) => format!("wiki:{slug}"),
            })
            .collect();
        let active = p.active_tab.get_untracked();
        spawn_local(async move { ipc::write_session(w.path, tabs, active).await });
    }
}
