//! Value types shared by [`State`](super::State) and its action methods: editor
//! [`Buffer`]/[`Pane`], the tab/section/mode enums, and the small UI structs
//! (toasts, context menu). Pure data + a little block-editing logic on `Buffer`.

use leptos::prelude::*;

use crate::blocks::{self, Block};
use crate::model::{LinkRef, NoteMeta};

/// Window size for the launcher (fixed, sized to its content). The editor's size
/// is remembered per user (see `window::restore_window` / `remember_window`).
pub const LAUNCH_SIZE: (f64, f64) = (400.0, 520.0);

/// The three sections inside a well, chosen from the left rail.
#[derive(Clone, Copy, PartialEq)]
pub enum Section {
    /// Freeform foldered markdown notes.
    Notes,
    /// Kanban tasks + goals (a full-width board, not the editor pane).
    Tasks,
    /// A `[[`-linked namespace of wiki pages (globally-unique slugs), organisable
    /// into purely-cosmetic folders.
    Wiki,
}

/// Which surface the tasks section shows: the kanban board (status as the
/// axis), the month calendar (time as the axis), or the flat sortable table
/// (everything at once, for review). Session-local — the tasks section always
/// opens on the board.
#[derive(Clone, Copy, PartialEq)]
pub enum TaskView {
    /// The kanban board + backlog.
    Board,
    /// The month-grid calendar (task due dates + goal targets).
    Calendar,
    /// The flat sortable table of every task (backlog included).
    Table,
}

/// Within-column ordering for the task board.
#[derive(Clone, Copy, PartialEq)]
pub enum TaskSort {
    /// Manual order (the `order` field / drag position).
    Manual,
    /// Highest priority first.
    Priority,
    /// Earliest due date first.
    Due,
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

/// What an open tab points at. A note (foldered id) or a wiki page (flat slug);
/// task variants land with that section.
#[derive(Clone, PartialEq)]
pub enum TabTarget {
    /// A note, by its `notes/`-relative id.
    Note(String),
    /// A wiki page, by its slug.
    WikiPage(String),
}

impl TabTarget {
    /// The inner id/slug, regardless of kind — for the buffer pointer + breadcrumbs.
    pub fn entry_id(&self) -> &str {
        match self {
            TabTarget::Note(id) | TabTarget::WikiPage(id) => id,
        }
    }

    /// The note id, if this tab is a note.
    pub fn note_id(&self) -> Option<&str> {
        match self {
            TabTarget::Note(id) => Some(id),
            TabTarget::WikiPage(_) => None,
        }
    }

    /// Mutable note id, for rename/move re-pointing.
    pub(super) fn note_id_mut(&mut self) -> Option<&mut String> {
        match self {
            TabTarget::Note(id) => Some(id),
            TabTarget::WikiPage(_) => None,
        }
    }

    /// The wiki slug, if this tab is a page.
    pub fn page_slug(&self) -> Option<&str> {
        match self {
            TabTarget::WikiPage(slug) => Some(slug),
            TabTarget::Note(_) => None,
        }
    }

    /// Mutable wiki slug, for rename re-pointing.
    pub(super) fn page_slug_mut(&mut self) -> Option<&mut String> {
        match self {
            TabTarget::WikiPage(slug) => Some(slug),
            TabTarget::Note(_) => None,
        }
    }

    /// The Lucide icon name for this tab's kind.
    pub fn icon(&self) -> &'static str {
        match self {
            TabTarget::Note(_) => "file-text",
            TabTarget::WikiPage(_) => "book",
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
    /// Whether this is the reusable **preview** tab (italic, replaced by the next
    /// preview-open). Pinned to a permanent tab on first edit or a tab double-click.
    pub preview: bool,
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

/// An editable markdown document: the editing signals + a save callback. The
/// block/source editor operates on a `Buffer`, so the same editor serves the
/// main pane's active tab (built from [`State::pane_buffer`]) and the task
/// drawer's body (built locally). `Copy` — every field is a handle.
#[derive(Clone, Copy)]
pub struct Buffer {
    /// Full markdown source — the source of truth for every mode.
    pub content: RwSignal<String>,
    /// Top-level blocks segmented from `content`; re-derived on edit.
    pub blocks: RwSignal<Vec<Block>>,
    /// Index of the block being edited (`None` = none; `blocks.len()` = new-at-end).
    pub active_block: RwSignal<Option<usize>>,
    /// The Source-mode `<textarea>`, seeded imperatively to avoid cursor jumps.
    pub source_editor: NodeRef<leptos::html::Textarea>,
    /// Whether a document is open (drives the editor's empty state).
    pub open: Signal<bool>,
    /// Persist the full new source after every edit.
    pub save: Callback<String>,
}

impl Buffer {
    /// Commit an edited block back into the document: splice → re-segment → save.
    ///
    /// `idx` is the block's index at the time editing began. Three cases:
    /// - `idx < blocks.len()`: normal splice into the existing source.
    /// - `idx >= blocks.len()` with content: append after the current source.
    /// - `idx >= blocks.len()` with empty text: just clear `active_block` (no save).
    pub fn commit_block(self, idx: usize, new_text: String) {
        if !self.open.get_untracked() {
            self.active_block.set(None);
            return;
        }
        let text = new_text.trim_end().to_string();
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
        self.save.run(new_src);
    }

    /// Commit block `from_idx` and move focus to `to_idx`.
    ///
    /// Used by double-Enter (→ `idx+1`), ↑ (→ `idx-1`), ↓ (→ `idx+1`), and
    /// Backspace-merge. `to_idx` may equal `blocks.len()` (new-block-at-end sentinel).
    /// Skips the disk write when the source is unchanged (pure cursor navigation).
    pub fn commit_and_go(self, from_idx: usize, text: String, to_idx: usize) {
        if !self.open.get_untracked() {
            return;
        }
        let text = text.trim_end().to_string();
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
            self.save.run(new_src);
        }
    }

    /// Split block `idx` at a blank line: `text_before` becomes the block's new
    /// content and `text_after` (if non-empty) is spliced in as the next block.
    /// When `text_after` is empty this is identical to committing and opening a
    /// new empty block at `idx+1`.
    pub fn split_block(self, idx: usize, text_before: String, text_after: String) {
        if !self.open.get_untracked() {
            return;
        }
        let before = text_before.trim_end().to_string();
        let after = text_after.trim_start_matches('\n').trim_end().to_string();
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
        self.save.run(new_src);
    }

    /// Merge block `idx` with the block above it — triggered by Backspace at
    /// cursor position 0. The two source ranges are joined with a single newline;
    /// pulldown-cmark then re-parses them as one or more blocks depending on type
    /// (two paragraphs become one; a heading stays separate from what follows).
    pub fn merge_with_prev(self, idx: usize, current_text: String) {
        if idx == 0 || !self.open.get_untracked() {
            return;
        }
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
        self.save.run(new_src);
    }

    /// Update the full document from Source mode's textarea (called on every input).
    pub fn update_source(self, content: String) {
        if !self.open.get_untracked() {
            return;
        }
        self.blocks.set(blocks::segment(&content));
        self.content.set(content.clone());
        self.save.run(content);
    }
}

/// One editor pane: its own tab strip, active tab, and materialised document.
/// Split view shows two — each an independent editing surface. `Copy` (every
/// field is a signal handle). The block/source editor runs on a [`Buffer`] built
/// from a pane via [`State::pane_buffer`].
#[derive(Clone, Copy)]
pub struct Pane {
    /// Open tabs in this pane, in strip order.
    pub tabs: RwSignal<Vec<Tab>>,
    /// Index of the focused tab in this pane, if any.
    pub active_tab: RwSignal<Option<usize>>,
    /// The active tab's full markdown source.
    pub content: RwSignal<String>,
    /// Top-level blocks segmented from `content`.
    pub blocks: RwSignal<Vec<Block>>,
    /// Index of the block being edited in Live mode (`None` = none; `len()` = new-at-end).
    pub active_block: RwSignal<Option<usize>>,
    /// This pane's editor display mode.
    pub mode: RwSignal<Mode>,
    /// The Source-mode `<textarea>`, seeded imperatively to avoid cursor jumps.
    pub source_editor: NodeRef<leptos::html::Textarea>,
    /// The active note's filesystem timestamps (reading view).
    pub meta: RwSignal<Option<NoteMeta>>,
    /// Backlinks for the active wiki page (notes + wiki pages that link to it).
    pub backlinks: RwSignal<Vec<LinkRef>>,
    /// Bumped on every load so a slower earlier read can't clobber a later tab
    /// switch (async reads may resolve out of order). See `State::load_pane`.
    pub(super) load_gen: RwSignal<u32>,
}

impl Pane {
    /// A fresh empty pane (signals created in the current reactive owner).
    pub(super) fn new() -> Self {
        Self {
            tabs: RwSignal::new(Vec::new()),
            active_tab: RwSignal::new(None),
            content: RwSignal::new(String::new()),
            blocks: RwSignal::new(Vec::new()),
            active_block: RwSignal::new(None),
            mode: RwSignal::new(Mode::Live),
            source_editor: NodeRef::new(),
            meta: RwSignal::new(None),
            backlinks: RwSignal::new(Vec::new()),
            load_gen: RwSignal::new(0),
        }
    }

    /// Whether a document is open in this pane (a tab is active).
    pub(super) fn open(self) -> bool {
        self.active_tab.get().is_some()
    }

    /// Empty the pane's buffer (nothing open).
    pub(super) fn clear_buffer(self) {
        self.content.set(String::new());
        self.blocks.set(Vec::new());
        self.active_block.set(None);
        self.meta.set(None);
        self.backlinks.set(Vec::new());
    }

    /// Reset the pane to empty (no tabs, nothing open).
    pub(super) fn reset(self) {
        self.tabs.set(Vec::new());
        self.active_tab.set(None);
        self.mode.set(Mode::Live);
        self.clear_buffer();
    }
}

/// Where a dragged tab would land — drives the drop indicator and the move.
#[derive(Clone, Copy, PartialEq)]
pub enum TabDrop {
    /// Insert before `index` in pane `pane`'s strip (`index == len` = append).
    Strip { pane: usize, index: usize },
    /// Open a split and move the tab into the new second pane.
    NewSplit,
}

/// A soft-deleted entry held for undo — a note, wiki page, task, or goal plus
/// its content, so [`State::undo_toast`] can rewrite the file and reopen it.
#[derive(Clone)]
pub struct UndoEntry {
    /// Which kind of entry was deleted.
    pub kind: EntryKind,
    /// The note id / wiki slug / task id / goal id it was deleted under.
    pub id: String,
    /// The file's content at delete time (raw markdown, frontmatter included).
    pub content: String,
}

/// The undoable entry kinds.
#[derive(Clone, Copy, PartialEq)]
pub enum EntryKind {
    Note,
    WikiPage,
    Task,
    Goal,
}

/// A transient toast: a short message and, optionally, an undo action. Auto-
/// dismisses; an undo restores a soft-deleted entry.
#[derive(Clone)]
pub struct Toast {
    /// The message shown.
    pub message: String,
    /// The soft-deleted entry to restore on undo (`None` = informational toast).
    pub undo: Option<UndoEntry>,
}

/// What was right-clicked — selects the context menu's items.
#[derive(Clone)]
pub enum MenuTarget {
    /// A note-tree entry (`is_dir` = folder).
    Note { id: String, is_dir: bool },
    /// A wiki-tree entry, by wiki-relative path (`is_dir` = organisational
    /// folder; otherwise a page whose slug is the path's last segment).
    Wiki { path: String, is_dir: bool },
    /// A task card, by id.
    Task { id: String },
    /// A goal chip, by id.
    Goal { id: String },
    /// A tab at index `idx` in pane `pane`.
    Tab { pane: usize, idx: usize },
    /// The editor body of pane `pane` (clipboard + mode + split).
    Editor { pane: usize },
}

/// An open context menu: where it sits and what it acts on.
#[derive(Clone)]
pub struct Menu {
    /// Viewport x of the click.
    pub x: i32,
    /// Viewport y of the click.
    pub y: i32,
    /// What was right-clicked.
    pub target: MenuTarget,
}
