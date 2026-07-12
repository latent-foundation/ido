//! Notes section: tree reload, note/folder CRUD, inline rename, drag-to-move.
//! `impl State` block — see [`super`].

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::*;

use crate::ipc;

impl State {
    // --- notes & folders --------------------------------------------------

    /// Reload the open well's tree from disk.
    pub fn reload_tree(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move { self.tree.set(ipc::list_tree(w.path).await) });
    }

    /// Open a note in a permanent tab (focusing it if already open). Used where
    /// the intent is to work on it (create, after rename); the sidebar tree's
    /// single-click uses [`State::preview_note`] instead.
    pub fn open_note(self, id: String) {
        self.open_tab(TabTarget::Note(id));
    }

    /// Open a note in the reusable **preview** tab (sidebar single-click).
    pub fn preview_note(self, id: String) {
        self.open_preview(TabTarget::Note(id));
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
            match ipc::rename_entry(w.path, id.clone(), is_dir, name).await {
                Ok(new_id) => {
                    self.sync_ids(&id, &new_id, is_dir);
                    self.reload_tree();
                }
                Err(e) => self.show_toast(e, None),
            }
        });
    }

    /// Delete a note, or an empty folder; close any tab showing the deleted note.
    pub fn remove_entry(self, id: String, is_dir: bool) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        let name = id.rsplit('/').next().unwrap_or(&id).to_string();
        spawn_local(async move {
            // Capture a note's body first so the delete can be undone (folders,
            // which must be empty to delete, get an informational toast only).
            let content = if is_dir {
                None
            } else {
                Some(ipc::read_note(w.path.clone(), id.clone()).await)
            };
            if ipc::delete_entry(w.path.clone(), id.clone(), is_dir).await {
                self.close_note_tabs(&id);
                self.tree.set(ipc::list_tree(w.path).await);
                let undo = content.map(|c| UndoEntry {
                    kind: EntryKind::Note,
                    id: id.clone(),
                    content: c,
                });
                self.show_toast(format!("deleted {name}"), undo);
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

    /// Keep a non-root `parent` folder expanded so a freshly-created child shows.
    fn expand_parent(self, parent: &str) {
        if !parent.is_empty() {
            self.expanded.update(|set| {
                set.insert(parent.to_string());
            });
        }
    }

    /// After a note rename/move from `old` id to `new` id, re-point every tab in
    /// both panes showing that note (or a note inside a moved folder).
    fn sync_ids(self, old: &str, new: &str, is_dir: bool) {
        let moved_prefix = format!("{old}/");
        for p in self.panes {
            p.tabs.update(|tabs| {
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
        }
        self.persist_session();
    }

    /// Close every tab showing note `id` (or a note inside folder `id`).
    fn close_note_tabs(self, id: &str) {
        let prefix = format!("{id}/");
        self.close_tabs(move |t| {
            t.note_id()
                .is_some_and(|tid| tid == id || tid.starts_with(&prefix))
        });
    }
}
