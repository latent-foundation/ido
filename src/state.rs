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
    /// Recently opened wells (launch screen).
    pub recents: RwSignal<Vec<WellRef>>,
    /// The open well's tree of folders and notes.
    pub tree: RwSignal<Vec<TreeNode>>,
    /// Id of the note currently in the editor, if any.
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
            recents: RwSignal::new(Vec::new()),
            tree: RwSignal::new(Vec::new()),
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
                self.well.set(Some(w));
                self.reload_tree();
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
        self.active.set(None);
        self.content.set(String::new());
        self.meta.set(None);
        self.blocks.set(Vec::new());
        self.active_block.set(None);
        self.mode.set(Mode::Live);
        self.target.set(String::new());
        self.expanded.set(HashSet::new());
        self.well.set(Some(w));
        spawn_local(async move { ipc::apply_window(EDITOR_SIZE.0, EDITOR_SIZE.1, true).await });
        self.reload_tree();
    }

    /// Return to the launcher: shrink + lock the window, refresh recents.
    pub fn leave_well(self) {
        self.well.set(None);
        self.active.set(None);
        self.content.set(String::new());
        self.meta.set(None);
        self.blocks.set(Vec::new());
        self.active_block.set(None);
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

    /// Open a note: fetch its body, segment it into blocks, and clear any active block.
    ///
    /// Empty notes start with `active_block = Some(0)` so the cursor appears
    /// immediately without an extra click.
    pub fn open_note(self, id: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        self.active.set(Some(id.clone()));
        self.active_block.set(None);
        self.meta.set(None);
        spawn_local(async move {
            let body = ipc::read_note(w.path.clone(), id.clone()).await;
            let segs = blocks::segment(&body);
            let initial_block = if body.trim().is_empty() {
                Some(0)
            } else {
                None
            };
            self.content.set(body.clone());
            self.blocks.set(segs);
            self.active_block.set(initial_block);
            // Seed the Source textarea directly when it is already in the DOM
            // (i.e. the user opened a note while already in Source mode).
            if let Some(ta) = self.source_editor.get_untracked() {
                ta.set_value(&body);
            }
            self.meta.set(ipc::note_meta(w.path, id).await);
        });
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

    /// Switch to `new_mode`, clearing any active block edit.
    pub fn set_mode(self, new_mode: Mode) {
        self.active_block.set(None);
        self.mode.set(new_mode);
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
                self.sync_active_id(&id, &new_id, is_dir);
                self.reload_tree();
            }
        });
    }

    /// Delete a note, or an empty folder; clear the editor if the open note went.
    pub fn remove_entry(self, id: String, is_dir: bool) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if ipc::delete_entry(w.path, id.clone(), is_dir).await {
                let gone = self
                    .active
                    .get_untracked()
                    .is_some_and(|a| a == id || a.starts_with(&format!("{id}/")));
                if gone {
                    self.active.set(None);
                    self.content.set(String::new());
                    self.meta.set(None);
                    self.blocks.set(Vec::new());
                    self.active_block.set(None);
                }
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
                self.sync_active_id(&id, &new_id, is_dir);
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

    /// After a rename/move from `old` id to `new` id, keep `active` pointing at
    /// the same note (whether it moved directly or sat inside a moved folder).
    fn sync_active_id(self, old: &str, new: &str, is_dir: bool) {
        self.active.update(|active| {
            if let Some(cur) = active {
                if is_dir {
                    if let Some(rest) = cur.strip_prefix(&format!("{old}/")) {
                        *cur = format!("{new}/{rest}");
                    }
                } else if cur == old {
                    *cur = new.to_string();
                }
            }
        });
    }
}
