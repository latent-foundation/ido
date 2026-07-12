//! Well lifecycle: startup, opening/creating/leaving a well, and restoring its
//! saved session. `impl State` block — see [`super`].

use std::collections::HashSet;

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::*;

use crate::ipc;
use crate::model::WellRef;

impl State {
    // --- startup ----------------------------------------------------------

    /// Reopen the most-recent well (else stay on the launcher), then reveal the
    /// hidden window — sizing it for the right screen first so there's no flash.
    pub fn start(self) {
        spawn_local(async move {
            let list = ipc::recent_wells().await;
            self.recents.set(list.clone());
            if let Some(w) = list.into_iter().next() {
                ipc::restore_window().await;
                let path = w.path.clone();
                ipc::migrate_well(path.clone()).await;
                self.well.set(Some(w));
                self.tree.set(ipc::list_tree(path.clone()).await);
                self.wiki.set(ipc::list_wiki(path.clone()).await);
                self.columns.set(ipc::task_columns(path.clone()).await);
                self.archive_days.set(ipc::archive_days(path.clone()).await);
                self.saved_views.set(ipc::saved_views(path.clone()).await);
                self.tasks.set(ipc::list_tasks(path.clone()).await);
                self.goals.set(ipc::list_goals(path.clone()).await);
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
        self.reset_panes();
        self.clear_assets();
        self.section.set(Section::Notes);
        self.wiki.set(Vec::new());
        self.tasks.set(Vec::new());
        self.active_task.set(None);
        self.goals.set(Vec::new());
        self.goal_filter.set(None);
        self.tag_filter.set(None);
        self.active_goal.set(None);
        self.task_filter.set(String::new());
        self.show_archive.set(false);
        self.target.set(String::new());
        self.expanded.set(HashSet::new());
        let path = w.path.clone();
        self.well.set(Some(w));
        spawn_local(async move {
            ipc::restore_window().await;
            ipc::migrate_well(path.clone()).await;
            self.tree.set(ipc::list_tree(path.clone()).await);
            self.wiki.set(ipc::list_wiki(path.clone()).await);
            self.columns.set(ipc::task_columns(path.clone()).await);
            self.archive_days.set(ipc::archive_days(path.clone()).await);
            self.saved_views.set(ipc::saved_views(path.clone()).await);
            self.tasks.set(ipc::list_tasks(path.clone()).await);
            self.goals.set(ipc::list_goals(path.clone()).await);
            self.restore_session(path).await;
        });
    }

    /// Return to the launcher: shrink + lock the window, refresh recents.
    pub fn leave_well(self) {
        self.well.set(None);
        self.reset_panes();
        self.tree.set(Vec::new());
        self.wiki.set(Vec::new());
        self.tasks.set(Vec::new());
        self.active_task.set(None);
        self.goals.set(Vec::new());
        self.goal_filter.set(None);
        self.tag_filter.set(None);
        self.active_goal.set(None);
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

    /// Reset both panes to empty and collapse any split.
    fn reset_panes(self) {
        for p in self.panes {
            p.reset();
        }
        self.split.set(false);
        self.focused.set(0);
        self.clear_tab_drag();
    }

    /// Restore a well's saved tabs into the primary pane on entry. A missing /
    /// empty / unparseable session just leaves the workspace blank.
    async fn restore_session(self, well_path: String) {
        let Some(session) = ipc::read_session(well_path).await else {
            return;
        };
        let tabs: Vec<Tab> = session
            .tabs
            .into_iter()
            .filter_map(|encoded| {
                let (kind, id) = encoded.split_once(':')?;
                let target = match kind {
                    "note" => TabTarget::Note(id.to_string()),
                    "wiki" => TabTarget::WikiPage(id.to_string()),
                    _ => return None,
                };
                Some(Tab {
                    target,
                    mode: Mode::Live,
                    preview: false,
                })
            })
            .collect();
        if tabs.is_empty() {
            return;
        }
        let n = tabs.len();
        let p = self.panes[0];
        p.tabs.set(tabs);
        p.active_tab
            .set(Some(session.active.filter(|&a| a < n).unwrap_or(0)));
        self.load_pane(p);
    }
}
