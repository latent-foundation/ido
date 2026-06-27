//! The app's reactive state and the actions that mutate it.
//!
//! [`State`] bundles every signal into one `Copy` handle, provided through
//! Leptos context so components read it with `expect_context::<State>()` instead
//! of prop-drilling. All backend work lives in action methods here, which call
//! [`crate::ipc`] and update the signals — keeping components purely declarative.

use std::collections::HashSet;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::blocks::{self, Block};
use crate::ipc;
use crate::model::{NoteMeta, TreeNode, WellRef};

/// Window size for the launcher (fixed, sized to its content).
pub const LAUNCH_SIZE: (f64, f64) = (400.0, 520.0);
/// Window size for the editor (a resizable work surface).
pub const EDITOR_SIZE: (f64, f64) = (1080.0, 720.0);

/// The three sections inside a well, chosen from the left rail.
#[derive(Clone, Copy, PartialEq)]
pub enum Section {
    /// Freeform foldered markdown notes (the only section built so far).
    Notes,
    /// Kanban tasks + goals (placeholder until its increment lands).
    Tasks,
    /// Linked wiki pages (placeholder until its increment lands).
    Wiki,
}

/// The three editor display modes, cycling Source → Live → Reading → Source.
#[derive(Clone, Copy, PartialEq)]
pub enum Mode {
    /// Raw markdown in a single `<textarea>` (power-user / escape hatch).
    Source,
    /// Block live-preview: rendered blocks, click one to edit its source.
    Live,
    /// Fully rendered, read-only view.
    Reading,
}

/// What an open tab points at. Only notes exist today; wiki/task variants land
/// with their sections (`load_active` then learns to materialise them).
#[derive(Clone, PartialEq)]
pub enum TabTarget {
    /// A note, by its `notes/`-relative id.
    Note(String),
}

impl TabTarget {
    /// The note id this tab shows, if it is a note. Goes through an accessor so
    /// callers never pattern-match the (currently single-variant) enum directly.
    pub fn note_id(&self) -> Option<&str> {
        match self {
            TabTarget::Note(id) => Some(id),
        }
    }

    /// Mutable note id, for rename/move re-pointing.
    fn note_id_mut(&mut self) -> Option<&mut String> {
        match self {
            TabTarget::Note(id) => Some(id),
        }
    }
}

/// One open tab: what it shows plus its own remembered editor mode.
#[derive(Clone)]
pub struct Tab {
    /// What the tab displays.
    pub target: TabTarget,
    /// This tab's editor mode (source/live/reading), restored when re-focused.
    pub mode: Mode,
}

/// New index of the tab originally at `cur` after the tab at `from` is moved to
/// `to` (remove-then-insert). Used to keep the active tab focused on reorder.
fn reindex_after_move(cur: usize, from: usize, to: usize) -> usize {
    if cur == from {
        return to;
    }
    let after_remove = if cur > from { cur - 1 } else { cur };
    if after_remove >= to {
        after_remove + 1
    } else {
        after_remove
    }
}

/// The parent portion of a `/`-separated id (`"a/b" -> "a"`, `"a" -> ""`).
pub fn parent_of(id: &str) -> String {
    match id.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
    }
}

/// Append `new_block` to `src` with a blank-line separator, creating a new
/// top-level block in the document.
fn append_to_source(src: &str, new_block: &str) -> String {
    if src.trim().is_empty() {
        new_block.to_string()
    } else if src.ends_with("\n\n") {
        format!("{src}{new_block}")
    } else if src.ends_with('\n') {
        format!("{src}\n{new_block}")
    } else {
        format!("{src}\n\n{new_block}")
    }
}

/// All of the app's reactive state. `Copy` (every field is a signal handle), so
/// action methods take `self` by value and closures can freely capture it.
#[derive(Clone, Copy)]
pub struct State {
    /// The open well, or `None` on the launch screen.
    pub well: RwSignal<Option<WellRef>>,
    /// The active section within the open well (rail selection).
    pub section: RwSignal<Section>,
    /// Recently opened wells (launch screen).
    pub recents: RwSignal<Vec<WellRef>>,
    /// The open well's tree of folders and notes.
    pub tree: RwSignal<Vec<TreeNode>>,
    /// Open tabs, in strip order (global across sections; see `docs/well-spaces-plan.md` §7).
    pub tabs: RwSignal<Vec<Tab>>,
    /// Index of the focused tab, if any.
    pub active_tab: RwSignal<Option<usize>>,
    /// Index of the tab being dragged (tab-strip reorder), if any.
    pub tab_drag: RwSignal<Option<usize>>,
    /// Id of the note materialised into the editor buffer — the active tab's
    /// note, mirrored here so the autosave/commit paths and the sidebar highlight
    /// keep working unchanged. `None` when no note tab is active.
    pub active: RwSignal<Option<String>>,
    /// Ids of expanded folders.
    pub expanded: RwSignal<HashSet<String>>,
    /// `(id, is_dir)` of the entry being renamed inline, if any.
    pub renaming: RwSignal<Option<(String, bool)>>,
    /// `(id, is_dir)` of the entry being dragged, if any.
    pub dragging: RwSignal<Option<(String, bool)>>,
    /// Id of the folder currently hovered as a drop target.
    pub drag_over: RwSignal<Option<String>>,
    /// Folder that new notes/folders are created into (`""` = root).
    pub target: RwSignal<String>,
    /// Whether the settings modal is open.
    pub settings_open: RwSignal<bool>,
    /// Whether the create-a-well form is showing (launch screen).
    pub creating: RwSignal<bool>,
    /// Draft name in the create-a-well form.
    pub new_name: RwSignal<String>,
    /// Chosen parent location in the create-a-well form.
    pub new_parent: RwSignal<Option<String>>,
    /// The active note's full markdown source (source of truth for both modes).
    pub content: RwSignal<String>,
    /// The active note's filesystem timestamps, shown in the reading view.
    pub meta: RwSignal<Option<NoteMeta>>,
    /// Top-level blocks segmented from `content`; re-derived on open and commit.
    pub blocks: RwSignal<Vec<Block>>,
    /// Index of the block currently being edited in Live mode (`None` = no block active).
    /// May equal `blocks.len()` to signal "new block being typed at the end".
    pub active_block: RwSignal<Option<usize>>,
    /// Current editor display mode.
    pub mode: RwSignal<Mode>,
    /// The Source-mode `<textarea>` — populated imperatively on open/mode-switch
    /// so the cursor never jumps from a reactive `prop:value` binding.
    pub source_editor: NodeRef<leptos::html::Textarea>,
}

impl State {
    /// Create the initial (launcher) state. Must be called within a reactive owner.
    #[allow(clippy::new_without_default)]
    pub fn new() -> Self {
        Self {
            well: RwSignal::new(None),
            section: RwSignal::new(Section::Notes),
            recents: RwSignal::new(Vec::new()),
            tree: RwSignal::new(Vec::new()),
            tabs: RwSignal::new(Vec::new()),
            active_tab: RwSignal::new(None),
            tab_drag: RwSignal::new(None),
            active: RwSignal::new(None),
            expanded: RwSignal::new(HashSet::new()),
            renaming: RwSignal::new(None),
            dragging: RwSignal::new(None),
            drag_over: RwSignal::new(None),
            target: RwSignal::new(String::new()),
            settings_open: RwSignal::new(false),
            creating: RwSignal::new(false),
            new_name: RwSignal::new(String::new()),
            new_parent: RwSignal::new(None),
            content: RwSignal::new(String::new()),
            meta: RwSignal::new(None),
            blocks: RwSignal::new(Vec::new()),
            active_block: RwSignal::new(None),
            mode: RwSignal::new(Mode::Live),
            source_editor: NodeRef::new(),
        }
    }

    // --- startup ----------------------------------------------------------

    /// Reopen the most-recent well (else stay on the launcher), then reveal the
    /// hidden window — sizing it for the right screen first so there's no flash.
    pub fn start(self) {
        spawn_local(async move {
            let list = ipc::recent_wells().await;
            self.recents.set(list.clone());
            if let Some(w) = list.into_iter().next() {
                ipc::apply_window(EDITOR_SIZE.0, EDITOR_SIZE.1, true).await;
                let path = w.path.clone();
                ipc::migrate_well(path.clone()).await;
                self.well.set(Some(w));
                self.tree.set(ipc::list_tree(path.clone()).await);
                self.restore_session(path).await;
            }
            ipc::show_window().await;
        });
    }

    // --- wells ------------------------------------------------------------

    /// Refresh the recent-wells list (launch screen).
    pub fn load_recents(self) {
        spawn_local(async move { self.recents.set(ipc::recent_wells().await) });
    }

    /// Enter a well: grow + unlock the window, switch to the editor, load notes.
    pub fn enter_well(self, w: WellRef) {
        self.creating.set(false);
        self.clear_buffer();
        self.mode.set(Mode::Live);
        self.section.set(Section::Notes);
        self.tabs.set(Vec::new());
        self.active_tab.set(None);
        self.target.set(String::new());
        self.expanded.set(HashSet::new());
        let path = w.path.clone();
        self.well.set(Some(w));
        spawn_local(async move {
            ipc::apply_window(EDITOR_SIZE.0, EDITOR_SIZE.1, true).await;
            ipc::migrate_well(path.clone()).await;
            self.tree.set(ipc::list_tree(path.clone()).await);
            self.restore_session(path).await;
        });
    }

    /// Return to the launcher: shrink + lock the window, refresh recents.
    pub fn leave_well(self) {
        self.well.set(None);
        self.clear_buffer();
        self.tabs.set(Vec::new());
        self.active_tab.set(None);
        self.tree.set(Vec::new());
        spawn_local(async move { ipc::apply_window(LAUNCH_SIZE.0, LAUNCH_SIZE.1, false).await });
        self.load_recents();
    }

    /// Pick an existing folder and open it as a well.
    pub fn open_well_picker(self) {
        spawn_local(async move {
            if let Some(path) = ipc::pick_folder().await {
                if let Some(w) = ipc::open_well(path).await {
                    self.enter_well(w);
                }
            }
        });
    }

    /// Show the create-a-well form (reset to blank).
    pub fn start_create(self) {
        self.new_name.set(String::new());
        self.new_parent.set(None);
        self.creating.set(true);
    }

    /// Hide the create-a-well form.
    pub fn cancel_create(self) {
        self.creating.set(false);
    }

    /// Pick the parent location for the new well.
    pub fn choose_location(self) {
        spawn_local(async move {
            if let Some(path) = ipc::pick_folder().await {
                self.new_parent.set(Some(path));
            }
        });
    }

    /// Create the well from the form (name + chosen location) and open it.
    pub fn confirm_create(self) {
        let name = self.new_name.get_untracked();
        let (Some(parent), false) = (self.new_parent.get_untracked(), name.trim().is_empty())
        else {
            return;
        };
        spawn_local(async move {
            if let Some(w) = ipc::create_well(parent, name).await {
                self.enter_well(w);
            }
        });
    }

    // --- notes & folders --------------------------------------------------

    /// Reload the open well's tree from disk.
    pub fn reload_tree(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move { self.tree.set(ipc::list_tree(w.path).await) });
    }

    /// Open a note in a tab (focusing it if it is already open). Called by the
    /// sidebar tree; thin wrapper over [`State::open_tab`].
    pub fn open_note(self, id: String) {
        self.open_tab(TabTarget::Note(id));
    }

    // --- tabs -------------------------------------------------------------

    /// Open `target` in a tab: focus it if already open, else append + focus.
    pub fn open_tab(self, target: TabTarget) {
        if let Some(i) = self
            .tabs
            .get_untracked()
            .iter()
            .position(|t| t.target == target)
        {
            self.activate_tab(i);
            return;
        }
        self.tabs.update(|tabs| {
            tabs.push(Tab {
                target,
                mode: Mode::Live,
            })
        });
        let last = self.tabs.get_untracked().len() - 1;
        self.active_tab.set(Some(last));
        self.load_active();
        self.persist_session();
    }

    /// Focus tab `i` and materialise its content into the editor buffer.
    pub fn activate_tab(self, i: usize) {
        if i >= self.tabs.get_untracked().len() {
            return;
        }
        self.active_tab.set(Some(i));
        self.load_active();
        self.persist_session();
    }

    /// Close tab `i`, focusing a neighbour (right, then left); empty when none left.
    pub fn close_tab(self, i: usize) {
        let len = self.tabs.get_untracked().len();
        if i >= len {
            return;
        }
        self.tabs.update(|tabs| {
            tabs.remove(i);
        });
        let remaining = len - 1;
        let new_active = match self.active_tab.get_untracked() {
            _ if remaining == 0 => None,
            Some(a) if a < i => Some(a),
            Some(a) if a > i => Some(a - 1),
            Some(_) => Some(i.min(remaining - 1)), // closed the active tab
            None => None,
        };
        self.active_tab.set(new_active);
        self.load_active();
        self.persist_session();
    }

    /// Move the tab at `from` to index `to`, keeping the active tab focused.
    pub fn reorder_tabs(self, from: usize, to: usize) {
        let len = self.tabs.get_untracked().len();
        if from >= len || to >= len || from == to {
            self.tab_drag.set(None);
            return;
        }
        self.tabs.update(|tabs| {
            let t = tabs.remove(from);
            tabs.insert(to, t);
        });
        self.active_tab
            .update(|a| *a = a.map(|cur| reindex_after_move(cur, from, to)));
        self.tab_drag.set(None);
        self.persist_session();
    }

    /// Focus the next tab (wrapping).
    pub fn next_tab(self) {
        let len = self.tabs.get_untracked().len();
        if len > 0 {
            let cur = self.active_tab.get_untracked().unwrap_or(0);
            self.activate_tab((cur + 1) % len);
        }
    }

    /// Focus the previous tab (wrapping).
    pub fn prev_tab(self) {
        let len = self.tabs.get_untracked().len();
        if len > 0 {
            let cur = self.active_tab.get_untracked().unwrap_or(0);
            self.activate_tab((cur + len - 1) % len);
        }
    }

    /// Commit an edited block back into the document: splice → re-segment → save.
    ///
    /// `idx` is the block's index at the time editing began. Three cases:
    /// - `idx < blocks.len()`: normal splice into the existing source.
    /// - `idx >= blocks.len()` with content: append after the current source.
    /// - `idx >= blocks.len()` with empty text: just clear `active_block` (no save).
    pub fn commit_block(self, idx: usize, new_text: String) {
        let text = new_text.trim_end().to_string();
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            self.active_block.set(None);
            return;
        };
        let old_src = self.content.get_untracked();
        let current_blocks = self.blocks.get_untracked();
        let new_src = if let Some(block) = current_blocks.get(idx) {
            blocks::splice(&old_src, block, &text)
        } else if text.is_empty() {
            // Nothing typed in the new-block textarea — just dismiss.
            self.active_block.set(None);
            return;
        } else {
            // Append new content after the current source with a blank-line separator.
            append_to_source(&old_src, &text)
        };
        let new_blocks = blocks::segment(&new_src);
        self.content.set(new_src.clone());
        self.blocks.set(new_blocks);
        self.active_block.set(None);
        spawn_local(async move { ipc::write_note(w.path, id, new_src).await });
    }

    /// Commit block `from_idx` and move focus to `to_idx`.
    ///
    /// Used by double-Enter (→ `idx+1`), ↑ (→ `idx-1`), ↓ (→ `idx+1`), and
    /// Backspace-merge. `to_idx` may equal `blocks.len()` (new-block-at-end sentinel).
    /// Skips the disk write when the source is unchanged (pure cursor navigation).
    pub fn commit_and_go(self, from_idx: usize, text: String, to_idx: usize) {
        let text = text.trim_end().to_string();
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            return;
        };
        let old_src = self.content.get_untracked();
        let current_blocks = self.blocks.get_untracked();
        let new_src = if let Some(block) = current_blocks.get(from_idx) {
            blocks::splice(&old_src, block, &text)
        } else if text.is_empty() {
            // New-block-at-end textarea was left empty — no change to source.
            old_src.clone()
        } else {
            append_to_source(&old_src, &text)
        };
        let new_blocks = blocks::segment(&new_src);
        // Clamp to_idx so it never exceeds blocks.len() (the new-block-at-end sentinel).
        let target = to_idx.min(new_blocks.len());
        self.content.set(new_src.clone());
        self.blocks.set(new_blocks);
        self.active_block.set(Some(target));
        if new_src != old_src {
            spawn_local(async move { ipc::write_note(w.path, id, new_src).await });
        }
    }

    /// Split block `idx` at a blank line: `text_before` becomes the block's new
    /// content and `text_after` (if non-empty) is spliced in as the next block.
    /// When `text_after` is empty this is identical to committing and opening a
    /// new empty block at `idx+1`.
    pub fn split_block(self, idx: usize, text_before: String, text_after: String) {
        let before = text_before.trim_end().to_string();
        let after = text_after.trim_start_matches('\n').trim_end().to_string();
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            return;
        };
        let old_src = self.content.get_untracked();
        let current_blocks = self.blocks.get_untracked();
        let new_src = if let Some(block) = current_blocks.get(idx) {
            let replacement = if after.is_empty() {
                before.clone()
            } else if before.is_empty() {
                after.clone()
            } else {
                format!("{before}\n\n{after}")
            };
            blocks::splice(&old_src, block, &replacement)
        } else if after.is_empty() {
            append_to_source(&old_src, &before)
        } else {
            let mid = append_to_source(&old_src, &before);
            append_to_source(&mid, &after)
        };
        let new_blocks = blocks::segment(&new_src);
        let target = (idx + 1).min(new_blocks.len());
        self.content.set(new_src.clone());
        self.blocks.set(new_blocks);
        self.active_block.set(Some(target));
        spawn_local(async move { ipc::write_note(w.path, id, new_src).await });
    }

    /// Merge block `idx` with the block above it — triggered by Backspace at
    /// cursor position 0. The two source ranges are joined with a single newline;
    /// pulldown-cmark then re-parses them as one or more blocks depending on type
    /// (two paragraphs become one; a heading stays separate from what follows).
    pub fn merge_with_prev(self, idx: usize, current_text: String) {
        if idx == 0 {
            return;
        }
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            return;
        };
        let old_src = self.content.get_untracked();
        let current_blocks = self.blocks.get_untracked();
        let (Some(prev), Some(curr)) = (current_blocks.get(idx - 1), current_blocks.get(idx))
        else {
            return;
        };
        let merged = if current_text.trim().is_empty() {
            prev.src.clone()
        } else {
            format!("{}\n{}", prev.src, current_text)
        };
        let new_src = format!(
            "{}{}{}",
            &old_src[..prev.range.start],
            merged,
            &old_src[curr.range.end..]
        );
        let new_blocks = blocks::segment(&new_src);
        self.content.set(new_src.clone());
        self.blocks.set(new_blocks);
        self.active_block.set(Some(idx - 1));
        spawn_local(async move { ipc::write_note(w.path, id, new_src).await });
    }

    /// Update the full document from Source mode's textarea (called on every input).
    pub fn update_source(self, content: String) {
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            return;
        };
        self.blocks.set(blocks::segment(&content));
        self.content.set(content.clone());
        spawn_local(async move { ipc::write_note(w.path, id, content).await });
    }

    /// Switch to `new_mode`, clearing any active block edit. The choice is
    /// remembered on the active tab so each tab keeps its own mode.
    pub fn set_mode(self, new_mode: Mode) {
        self.active_block.set(None);
        self.mode.set(new_mode);
        if let Some(i) = self.active_tab.get_untracked() {
            self.tabs.update(|tabs| {
                if let Some(t) = tabs.get_mut(i) {
                    t.mode = new_mode;
                }
            });
        }
    }

    /// Open a link from the rendered markdown in the OS browser.
    pub fn open_link(self, url: String) {
        spawn_local(async move { ipc::open_external(url).await });
    }

    /// Create a note in `parent`, open it, and start an inline rename.
    pub fn add_note(self, parent: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(id) = ipc::create_note(w.path, parent.clone()).await {
                self.expand_parent(&parent);
                self.reload_tree();
                self.open_note(id.clone());
                self.renaming.set(Some((id, false)));
            }
        });
    }

    /// Create a folder in `parent` and start an inline rename.
    pub fn add_folder(self, parent: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(id) = ipc::create_folder(w.path, parent.clone()).await {
                self.expand_parent(&parent);
                self.reload_tree();
                self.renaming.set(Some((id, true)));
            }
        });
    }

    /// Commit an inline rename (no-op if the entry is no longer the one renaming,
    /// or the name is blank).
    pub fn commit_rename(self, id: String, is_dir: bool, name: String) {
        if self.renaming.get_untracked() != Some((id.clone(), is_dir)) {
            return;
        }
        self.renaming.set(None);
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(new_id) = ipc::rename_entry(w.path, id.clone(), is_dir, name).await {
                self.sync_ids(&id, &new_id, is_dir);
                self.reload_tree();
            }
        });
    }

    /// Delete a note, or an empty folder; close any tab showing the deleted note.
    pub fn remove_entry(self, id: String, is_dir: bool) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if ipc::delete_entry(w.path, id.clone(), is_dir).await {
                self.close_note_tabs(&id);
                self.reload_tree();
            }
        });
    }

    /// Move the currently-dragged entry into `dest` (`""` = root).
    pub fn move_into(self, dest: String) {
        let Some((id, is_dir)) = self.dragging.get_untracked() else {
            return;
        };
        self.dragging.set(None);
        if parent_of(&id) == dest {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(new_id) = ipc::move_entry(w.path, id.clone(), is_dir, dest).await {
                self.sync_ids(&id, &new_id, is_dir);
                self.reload_tree();
            }
        });
    }

    /// Expand or collapse a folder.
    pub fn toggle_expand(self, path: String) {
        self.expanded.update(|set| {
            if !set.remove(&path) {
                set.insert(path);
            }
        });
    }

    // --- window -----------------------------------------------------------

    /// Fire a no-argument window command (minimize / maximize / close).
    pub fn window_cmd(self, cmd: &'static str) {
        spawn_local(async move { ipc::window_command(cmd).await });
    }

    // --- internals --------------------------------------------------------

    /// Keep a non-root `parent` folder expanded so a freshly-created child shows.
    fn expand_parent(self, parent: &str) {
        if !parent.is_empty() {
            self.expanded.update(|set| {
                set.insert(parent.to_string());
            });
        }
    }

    /// After a rename/move from `old` id to `new` id, re-point every tab showing
    /// that note (or a note inside a moved folder), then re-derive `active`.
    fn sync_ids(self, old: &str, new: &str, is_dir: bool) {
        let moved_prefix = format!("{old}/");
        self.tabs.update(|tabs| {
            for tab in tabs.iter_mut() {
                if let Some(id) = tab.target.note_id_mut() {
                    if is_dir {
                        if let Some(rest) = id.strip_prefix(&moved_prefix) {
                            *id = format!("{new}/{rest}");
                        }
                    } else if id == old {
                        *id = new.to_string();
                    }
                }
            }
        });
        self.active.set(self.active_tab_note());
        self.persist_session();
    }

    /// The note id shown by the active tab, if it is a note.
    fn active_tab_note(self) -> Option<String> {
        let i = self.active_tab.get_untracked()?;
        self.tabs
            .get_untracked()
            .get(i)
            .and_then(|t| t.target.note_id().map(String::from))
    }

    /// Materialise the active tab's content into the editor buffer signals.
    /// Clears the buffer when no note tab is active.
    fn load_active(self) {
        let tab = self
            .active_tab
            .get_untracked()
            .and_then(|i| self.tabs.get_untracked().get(i).cloned());
        let (Some(tab), Some(w)) = (tab, self.well.get_untracked()) else {
            self.clear_buffer();
            return;
        };
        let Some(id) = tab.target.note_id().map(String::from) else {
            self.clear_buffer();
            return;
        };
        self.active.set(Some(id.clone()));
        self.mode.set(tab.mode);
        self.active_block.set(None);
        self.meta.set(None);
        spawn_local(async move {
            let body = ipc::read_note(w.path.clone(), id.clone()).await;
            let segs = blocks::segment(&body);
            // Empty notes open with a cursor so no extra click is needed.
            let initial_block = if body.trim().is_empty() {
                Some(0)
            } else {
                None
            };
            self.content.set(body.clone());
            self.blocks.set(segs);
            self.active_block.set(initial_block);
            // Seed the Source textarea directly when it is already in the DOM.
            if let Some(ta) = self.source_editor.get_untracked() {
                ta.set_value(&body);
            }
            self.meta.set(ipc::note_meta(w.path, id).await);
        });
    }

    /// Empty the editor buffer (no note open).
    fn clear_buffer(self) {
        self.active.set(None);
        self.content.set(String::new());
        self.blocks.set(Vec::new());
        self.active_block.set(None);
        self.meta.set(None);
    }

    /// Close every tab showing `id` (or a note inside folder `id`), keeping the
    /// previously-active tab focused when it survives.
    fn close_note_tabs(self, id: &str) {
        let gone = |tid: &str| tid == id || tid.starts_with(&format!("{id}/"));
        let any = self
            .tabs
            .get_untracked()
            .iter()
            .any(|t| t.target.note_id().is_some_and(gone));
        if !any {
            return;
        }
        let active_target = self
            .active_tab
            .get_untracked()
            .and_then(|i| self.tabs.get_untracked().get(i).map(|t| t.target.clone()));
        self.tabs
            .update(|tabs| tabs.retain(|t| !t.target.note_id().is_some_and(gone)));
        let tabs = self.tabs.get_untracked();
        let new_active = active_target
            .and_then(|tgt| tabs.iter().position(|t| t.target == tgt))
            .or(if tabs.is_empty() { None } else { Some(0) });
        self.active_tab.set(new_active);
        self.load_active();
        self.persist_session();
    }

    /// Persist the open tabs (note ids) + active index to `.ido/session.toml`.
    fn persist_session(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        let tabs: Vec<String> = self
            .tabs
            .get_untracked()
            .iter()
            .filter_map(|t| t.target.note_id().map(String::from))
            .collect();
        let active = self.active_tab.get_untracked();
        spawn_local(async move { ipc::write_session(w.path, tabs, active).await });
    }

    /// Restore a well's saved tabs (open notes) on entry. A missing/empty
    /// session just leaves the workspace blank.
    async fn restore_session(self, well_path: String) {
        let Some(session) = ipc::read_session(well_path).await else {
            return;
        };
        if session.tabs.is_empty() {
            return;
        }
        let tabs: Vec<Tab> = session
            .tabs
            .into_iter()
            .map(|id| Tab {
                target: TabTarget::Note(id),
                mode: Mode::Live,
            })
            .collect();
        let n = tabs.len();
        self.tabs.set(tabs);
        self.active_tab
            .set(Some(session.active.filter(|&a| a < n).unwrap_or(0)));
        self.load_active();
    }
}
