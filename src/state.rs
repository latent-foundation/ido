//! The app's reactive state and the actions that mutate it.
//!
//! [`State`] bundles every signal into one `Copy` handle, provided through
//! Leptos context so components read it with `expect_context::<State>()` instead
//! of prop-drilling. All backend work lives in action methods here, which call
//! [`crate::ipc`] and update the signals — keeping components purely declarative.

use std::collections::HashSet;

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::ipc;
use crate::model::{TreeNode, WellRef};

/// Window size for the launcher (fixed, sized to its content).
pub const LAUNCH_SIZE: (f64, f64) = (400.0, 520.0);
/// Window size for the editor (a resizable work surface).
pub const EDITOR_SIZE: (f64, f64) = (1080.0, 720.0);

/// The parent portion of a `/`-separated id (`"a/b" -> "a"`, `"a" -> ""`).
pub fn parent_of(id: &str) -> String {
    match id.rsplit_once('/') {
        Some((parent, _)) => parent.to_string(),
        None => String::new(),
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
    /// The editor `<textarea>` (written imperatively to avoid cursor jumps).
    pub editor: NodeRef<leptos::html::Textarea>,
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
            editor: NodeRef::new(),
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

    /// Open a note: fetch its body and push it into the (uncontrolled) editor.
    pub fn open_note(self, id: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        self.active.set(Some(id.clone()));
        spawn_local(async move {
            let content = ipc::read_note(w.path, id).await;
            if let Some(ta) = self.editor.get_untracked() {
                ta.set_value(&content);
            }
        });
    }

    /// Persist the active note's body (called on every keystroke).
    pub fn save_note(self, content: String) {
        let (Some(w), Some(id)) = (self.well.get_untracked(), self.active.get_untracked()) else {
            return;
        };
        spawn_local(async move { ipc::write_note(w.path, id, content).await });
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
                    if let Some(ta) = self.editor.get_untracked() {
                        ta.set_value("");
                    }
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
