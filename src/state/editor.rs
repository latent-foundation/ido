//! Editor panes: tabs, split view, tab drag, mode switching, the context menu,
//! and the pane→[`Buffer`] wiring. `impl State` block — see [`super`].

use super::*;

impl State {
    // --- context menu -----------------------------------------------------

    /// Open the right-click menu at viewport `(x, y)` for `target`.
    pub fn open_menu(self, x: i32, y: i32, target: MenuTarget) {
        self.menu.set(Some(Menu { x, y, target }));
    }

    /// Close the context menu.
    pub fn close_menu(self) {
        self.menu.set(None);
    }

    /// Close every tab in pane `p_idx` except the one at `keep`.
    pub fn close_other_tabs(self, p_idx: usize, keep: usize) {
        let p = self.pane(p_idx);
        let Some(target) = p.tabs.get_untracked().get(keep).map(|t| t.target.clone()) else {
            return;
        };
        p.tabs.update(|tabs| tabs.retain(|t| t.target == target));
        p.active_tab.set(Some(0));
        self.load_pane(p);
        self.persist_session();
    }

    // --- panes & tabs -----------------------------------------------------

    /// The focused pane (untracked) — the target of sidebar opens + shortcuts.
    pub fn cur(self) -> Pane {
        self.panes[self.focused.get_untracked().min(1)]
    }

    /// Focus pane `i` (clamped); a no-op when already focused.
    pub fn focus_pane(self, i: usize) {
        let i = i.min(1);
        if self.focused.get_untracked() != i {
            self.focused.set(i);
        }
    }

    /// Open `target` in a permanent tab in the focused pane.
    pub fn open_tab(self, target: TabTarget) {
        self.open_in(self.focused.get_untracked(), target, false);
    }

    /// Open `target` in the focused pane's reusable **preview** tab.
    pub fn open_preview(self, target: TabTarget) {
        self.open_in(self.focused.get_untracked(), target, true);
    }

    /// Open `target` in pane `p_idx`: focus that tab if already open there; else
    /// reuse the pane's preview slot (when `preview`) or append a new tab.
    fn open_in(self, p_idx: usize, target: TabTarget, preview: bool) {
        let p = self.pane(p_idx);
        self.focus_pane(p_idx);
        if let Some(i) = p
            .tabs
            .get_untracked()
            .iter()
            .position(|t| t.target == target)
        {
            self.activate_tab(p_idx, i);
            return;
        }
        let reuse = preview
            .then(|| p.tabs.get_untracked().iter().position(|t| t.preview))
            .flatten();
        let i = match reuse {
            Some(i) => {
                p.tabs.update(|tabs| {
                    if let Some(t) = tabs.get_mut(i) {
                        t.target = target;
                        t.mode = Mode::Live;
                    }
                });
                i
            }
            None => {
                p.tabs.update(|tabs| {
                    tabs.push(Tab {
                        target,
                        mode: Mode::Live,
                        preview,
                    })
                });
                p.tabs.get_untracked().len() - 1
            }
        };
        p.active_tab.set(Some(i));
        self.load_pane(p);
        self.persist_session();
    }

    /// Promote tab `i` in pane `p` from preview to permanent (no-op if permanent).
    /// Triggered by a tab double-click or the first edit of its document.
    pub fn pin_in(self, p: Pane, i: usize) {
        let is_preview = p.tabs.get_untracked().get(i).map(|t| t.preview) == Some(true);
        if !is_preview {
            return;
        }
        p.tabs.update(|tabs| {
            if let Some(t) = tabs.get_mut(i) {
                t.preview = false;
            }
        });
        self.persist_session();
    }

    /// Focus tab `i` in pane `p_idx` and materialise it.
    pub fn activate_tab(self, p_idx: usize, i: usize) {
        let p = self.pane(p_idx);
        if i >= p.tabs.get_untracked().len() {
            return;
        }
        self.focus_pane(p_idx);
        p.active_tab.set(Some(i));
        self.load_pane(p);
        self.persist_session();
    }

    /// Close tab `i` in pane `p_idx`, focusing a neighbour; collapse an emptied split.
    pub fn close_tab(self, p_idx: usize, i: usize) {
        let p = self.pane(p_idx);
        if i >= p.tabs.get_untracked().len() {
            return;
        }
        self.detach(p, i);
        self.load_pane(p);
        self.collapse_if_empty();
        self.persist_session();
    }

    /// Remove tab `idx` from pane `p`, re-pointing its active tab. Does not reload.
    fn detach(self, p: Pane, idx: usize) {
        let len = p.tabs.get_untracked().len();
        if idx >= len {
            return;
        }
        p.tabs.update(|tabs| {
            tabs.remove(idx);
        });
        let remaining = len - 1;
        let new_active = match p.active_tab.get_untracked() {
            _ if remaining == 0 => None,
            Some(a) if a < idx => Some(a),
            Some(a) if a > idx => Some(a - 1),
            Some(_) => Some(idx.min(remaining - 1)), // detached the active tab
            None => None,
        };
        p.active_tab.set(new_active);
    }

    /// Focus the next tab in the focused pane (wrapping).
    pub fn next_tab(self) {
        let p = self.cur();
        let len = p.tabs.get_untracked().len();
        if len > 0 {
            let cur = p.active_tab.get_untracked().unwrap_or(0);
            self.activate_tab(self.focused.get_untracked(), (cur + 1) % len);
        }
    }

    /// Focus the previous tab in the focused pane (wrapping).
    pub fn prev_tab(self) {
        let p = self.cur();
        let len = p.tabs.get_untracked().len();
        if len > 0 {
            let cur = p.active_tab.get_untracked().unwrap_or(0);
            self.activate_tab(self.focused.get_untracked(), (cur + len - 1) % len);
        }
    }

    /// Build the editor [`Buffer`] for pane `p` (saves route to that pane's tab).
    pub fn pane_buffer(self, p: Pane) -> Buffer {
        Buffer {
            content: p.content,
            blocks: p.blocks,
            active_block: p.active_block,
            source_editor: p.source_editor,
            open: Signal::derive(move || p.open()),
            save: Callback::new(move |src| self.save_pane(p, src)),
        }
    }

    /// Switch pane `p` to `new_mode`, clearing any active block edit; the choice
    /// is remembered on the active tab so each tab keeps its own mode.
    pub fn set_mode(self, p: Pane, new_mode: Mode) {
        p.active_block.set(None);
        p.mode.set(new_mode);
        if let Some(i) = p.active_tab.get_untracked() {
            p.tabs.update(|tabs| {
                if let Some(t) = tabs.get_mut(i) {
                    t.mode = new_mode;
                }
            });
        }
    }

    // --- split & tab drag -------------------------------------------------

    /// Open the split, duplicating the focused pane's active tab into the new
    /// pane (or just focus the second pane if already split).
    pub fn open_split(self) {
        if self.split.get_untracked() {
            self.focus_pane(1);
            return;
        }
        let src = self.cur();
        let target = src
            .active_tab
            .get_untracked()
            .and_then(|i| src.tabs.get_untracked().get(i).map(|t| t.target.clone()));
        let p1 = self.panes[1];
        p1.reset();
        if let Some(target) = target {
            p1.tabs.set(vec![Tab {
                target,
                mode: Mode::Live,
                preview: false,
            }]);
            p1.active_tab.set(Some(0));
        }
        self.split.set(true);
        self.focused.set(1);
        self.load_pane(p1);
        self.persist_session();
    }

    /// Close the split (discard the second pane), focusing the primary.
    pub fn close_split(self) {
        self.end_split();
        self.persist_session();
    }

    /// Mark `(pane, idx)` as the tab being dragged.
    pub fn start_tab_drag(self, pane: usize, idx: usize) {
        self.tab_drag.set(Some((pane, idx)));
    }

    /// Clear tab-drag state (drag end / commit).
    pub fn clear_tab_drag(self) {
        self.tab_drag.set(None);
        self.tab_drop.set(None);
    }

    /// Commit the current drop target (set during dragover), if any.
    pub fn commit_tab_drop(self) {
        match self.tab_drop.get_untracked() {
            Some(drop) => self.move_tab(drop),
            None => self.clear_tab_drag(),
        }
    }

    /// Commit the dragged tab to `drop`: reorder within a strip, move between
    /// panes, or split off into a new pane. Collapses a pane left empty.
    pub fn move_tab(self, drop: TabDrop) {
        let Some((src_pane, src_idx)) = self.tab_drag.get_untracked() else {
            self.clear_tab_drag();
            return;
        };
        let src = self.pane(src_pane);
        let Some(tab) = src.tabs.get_untracked().get(src_idx).cloned() else {
            self.clear_tab_drag();
            return;
        };
        let split_now = matches!(drop, TabDrop::NewSplit) && !self.split.get_untracked();
        let (dest_idx, mut insert) = match drop {
            TabDrop::Strip { pane, index } => (pane, index),
            TabDrop::NewSplit if self.split.get_untracked() => {
                (1, self.panes[1].tabs.get_untracked().len())
            }
            TabDrop::NewSplit => (1, 0),
        };
        // Dropping onto its own position within the same strip is a no-op.
        if !matches!(drop, TabDrop::NewSplit)
            && dest_idx == src_pane
            && (insert == src_idx || insert == src_idx + 1)
        {
            self.clear_tab_drag();
            return;
        }
        if split_now {
            self.panes[1].reset();
            self.split.set(true);
        }
        self.detach(src, src_idx);
        let dest = self.pane(dest_idx);
        if dest_idx == src_pane && insert > src_idx {
            insert -= 1;
        }
        let insert = insert.min(dest.tabs.get_untracked().len());
        dest.tabs.update(|tabs| tabs.insert(insert, tab));
        dest.active_tab.set(Some(insert));
        self.focus_pane(dest_idx);
        self.load_pane(dest);
        if dest_idx != src_pane {
            self.load_pane(src);
        }
        self.collapse_if_empty();
        self.clear_tab_drag();
        self.persist_session();
    }

    /// The focused pane's active note id, if it is a note (reactive — tree highlight).
    pub fn active_note(self) -> Option<String> {
        let p = self.panes[self.focused.get().min(1)];
        let i = p.active_tab.get()?;
        p.tabs
            .get()
            .get(i)
            .and_then(|t| t.target.note_id().map(String::from))
    }

    /// The focused pane's active wiki slug, if it is a page (reactive — list highlight).
    pub fn active_page(self) -> Option<String> {
        let p = self.panes[self.focused.get().min(1)];
        let i = p.active_tab.get()?;
        p.tabs
            .get()
            .get(i)
            .and_then(|t| t.target.page_slug().map(String::from))
    }
}
