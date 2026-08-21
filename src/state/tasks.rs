//! Tasks section: the kanban board (cards, drag/drop, sort) and goals.
//! `impl State` block — see [`super`].

use leptos::task::spawn_local;

use super::*;

use crate::ipc;
use crate::model::{SavedView, Task};

impl State {
    // --- tasks ------------------------------------------------------------

    /// Open the detail drawer for task `id` (closing the goal drawer).
    pub fn open_task(self, id: String) {
        self.active_goal.set(None);
        self.active_task.set(Some(id));
    }

    /// Close the task detail drawer.
    pub fn close_task(self) {
        self.active_task.set(None);
    }

    /// Create an untitled task in column `status`, refresh the board, and open
    /// its drawer (the fill-in-the-details flow; the palette's "new task").
    pub fn add_task(self, status: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(id) = ipc::create_task(w.path.clone(), status, String::new(), None).await {
                self.tasks.set(ipc::list_tasks(w.path).await);
                self.active_task.set(Some(id));
            }
        });
    }

    /// Quick capture: create a task titled `title` in column `status`, due on
    /// `due` (`None` = undated), and refresh the board — **without** opening the
    /// drawer, so typing can continue. Backs the board's column quick-add and,
    /// with a date, the calendar's day-popover quick-add.
    pub fn add_task_quick(self, status: String, title: String, due: Option<String>) {
        if title.trim().is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if ipc::create_task(w.path.clone(), status, title, due)
                .await
                .is_some()
            {
                self.tasks.set(ipc::list_tasks(w.path).await);
            }
        });
    }

    /// Quick capture from the calendar's day popover: a task titled `title`,
    /// due on `due`, in the **first** column (it's triaged work, not backlog).
    pub fn add_task_quick_due(self, title: String, due: String) {
        let status = self
            .columns
            .get_untracked()
            .first()
            .cloned()
            .unwrap_or_default();
        self.add_task_quick(status, title, Some(due));
    }

    /// Move the dragged task into `status` (a column name, or `""` for the
    /// backlog), appended at its end.
    pub fn move_task_to(self, id: String, status: String) {
        self.clear_task_drag();
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::move_task(w.path.clone(), id, status).await;
            self.tasks.set(ipc::list_tasks(w.path).await);
        });
    }

    /// Commit a card drag onto the board: drop it into the hovered column at the
    /// hovered position (manual sort) or appended (other sorts / empty space),
    /// renumbering that column. Reads the live drag signals, then clears them.
    pub fn commit_card_drop(self) {
        let (Some(id), Some(col)) = (
            self.task_drag.get_untracked(),
            self.task_drag_over.get_untracked(),
        ) else {
            self.clear_task_drag();
            return;
        };
        let before = self.task_drop_before.get_untracked();
        // The target column's cards in manual order, minus the dragged one.
        let mut ids: Vec<String> = {
            let mut cards: Vec<Task> = self
                .tasks
                .get_untracked()
                .into_iter()
                .filter(|t| t.status == col && t.id != id)
                .collect();
            cards.sort_by_key(|t| t.order);
            cards.into_iter().map(|t| t.id).collect()
        };
        let pos = match &before {
            Some(b) => ids.iter().position(|x| x == b).unwrap_or(ids.len()),
            None => ids.len(),
        };
        ids.insert(pos, id);
        self.clear_task_drag();
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::reorder_column(w.path.clone(), col, ids).await;
            self.tasks.set(ipc::list_tasks(w.path).await);
        });
    }

    /// Clear the card-drag signals (dragged id, hovered column, drop position,
    /// hovered calendar day).
    pub fn clear_task_drag(self) {
        self.task_drag.set(None);
        self.task_drag_over.set(None);
        self.task_drop_before.set(None);
        self.cal_drag_day.set(None);
    }

    /// Set (or clear, when blank) one of a task's metadata fields, then refresh.
    pub fn set_task_field(self, id: String, key: String, value: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::set_task_field(w.path.clone(), id, key, value).await;
            self.tasks.set(ipc::list_tasks(w.path).await);
        });
    }

    /// Persist a task's body, then refresh the board: the checklist rollup chip
    /// (`checks_done`/`checks_total`, parsed from the body) is shown on cards, so
    /// a body edit must re-run `list_tasks` for the card to pick it up.
    ///
    /// The refresh is kept per-commit (rather than debounced) because it is no
    /// longer disruptive: the task drawer is keyed on the id (see `drawers.rs`),
    /// so a `state.tasks` refresh no longer recreates `DrawerInner` — it only
    /// updates the reactive memos (existence / archived) and the card chip. The
    /// body lives on a local `Buffer` created once per mount, entirely
    /// independent of `state.tasks`, so a background reload can never clobber
    /// in-progress editing — even Source mode's per-keystroke commits are safe.
    /// (Live, the drawer's default, commits coarsely at block boundaries anyway.)
    pub fn update_task_body(self, id: String, body: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::update_task_body(w.path.clone(), id, body).await;
            self.tasks.set(ipc::list_tasks(w.path).await);
        });
    }

    /// Rename task `id`; keep the drawer pointed at it, then refresh the board.
    pub fn rename_task(self, id: String, name: String) {
        if name.trim().is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match ipc::rename_task(w.path.clone(), id.clone(), name).await {
                Ok(new_id) => {
                    // Fetch the refreshed board *before* touching any signal, then
                    // set `tasks` and re-point `active_task` synchronously (one
                    // batch): the drawer is keyed on the id, so it remounts on the
                    // new id — and must find it already present in `state.tasks`,
                    // or its uncontrolled inputs would seed from the stale list.
                    let tasks = ipc::list_tasks(w.path).await;
                    let is_active =
                        self.active_task.get_untracked().as_deref() == Some(id.as_str());
                    self.tasks.set(tasks);
                    if is_active {
                        self.active_task.set(Some(new_id));
                    }
                }
                // The board is unchanged on-disk; refresh anyway to keep the memos
                // consistent. The drawer's (uncontrolled) title input keeps the
                // rejected text until reopen — the toast signals the rejection.
                Err(e) => {
                    self.tasks.set(ipc::list_tasks(w.path).await);
                    self.show_toast(e, None);
                }
            }
        });
    }

    /// Delete task `id` (soft — an undo toast can restore it); close the drawer
    /// if it was open, then refresh.
    pub fn remove_task(self, id: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(content) = ipc::delete_task(w.path.clone(), id.clone()).await {
                if self.active_task.get_untracked().as_deref() == Some(id.as_str()) {
                    self.active_task.set(None);
                }
                self.tasks.set(ipc::list_tasks(w.path).await);
                self.show_toast(
                    format!("deleted {id}"),
                    Some(UndoEntry {
                        kind: EntryKind::Task,
                        id,
                        content,
                    }),
                );
            }
        });
    }

    /// Toggle the board's tag filter: select `tag`, or clear it if already
    /// active. Backs a card/backlog tag chip click (board.rs stops the click's
    /// propagation so it doesn't also open the drawer) and the toolbar's
    /// dismissible `tag: … ×` chip.
    pub fn toggle_tag_filter(self, tag: String) {
        self.tag_filter.update(|f| {
            *f = if f.as_deref() == Some(tag.as_str()) {
                None
            } else {
                Some(tag)
            };
        });
    }

    // --- saved views ------------------------------------------------------

    /// Apply a saved view: set all six toolbar signals from the snapshot in one
    /// go. Unknown `view` / `sort` strings fall back to Board / Manual (a
    /// since-removed enum variant can never wedge the toolbar); a `tag` / `goal`
    /// scope that no longer exists applies as a filter that simply matches
    /// nothing — deliberately not validated (per the roadmap: keep it dumb).
    pub fn apply_view(self, v: SavedView) {
        self.task_view.set(match v.view.as_str() {
            "calendar" => TaskView::Calendar,
            "table" => TaskView::Table,
            _ => TaskView::Board,
        });
        self.task_sort.set(match v.sort.as_str() {
            "priority" => TaskSort::Priority,
            "due" => TaskSort::Due,
            _ => TaskSort::Manual,
        });
        self.task_filter.set(v.filter);
        self.tag_filter.set(v.tag);
        self.goal_filter.set(v.goal);
        self.hide_done.set(v.hide_done);
    }

    /// Snapshot the current toolbar state under `name` and persist the list. A
    /// name matching an existing view **replaces** it (exact, case-sensitive);
    /// otherwise the snapshot is appended. A blank (trimmed) name does nothing.
    pub fn save_view(self, name: String) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let view = match self.task_view.get_untracked() {
            TaskView::Board => "board",
            TaskView::Calendar => "calendar",
            TaskView::Table => "table",
        }
        .to_string();
        let sort = match self.task_sort.get_untracked() {
            TaskSort::Manual => "manual",
            TaskSort::Priority => "priority",
            TaskSort::Due => "due",
        }
        .to_string();
        let snapshot = SavedView {
            name: name.clone(),
            view,
            filter: self.task_filter.get_untracked(),
            tag: self.tag_filter.get_untracked(),
            goal: self.goal_filter.get_untracked(),
            hide_done: self.hide_done.get_untracked(),
            sort,
        };
        let mut views = self.saved_views.get_untracked();
        match views.iter_mut().find(|v| v.name == name) {
            Some(slot) => *slot = snapshot,
            None => views.push(snapshot),
        }
        self.persist_views(views);
    }

    /// Delete the saved view named `name` (exact match) and persist the list.
    pub fn delete_view(self, name: String) {
        let views: Vec<SavedView> = self
            .saved_views
            .get_untracked()
            .into_iter()
            .filter(|v| v.name != name)
            .collect();
        self.persist_views(views);
    }

    /// Persist the whole saved-views list; update the signal on success, show an
    /// error toast on failure (the on-disk list is unchanged then).
    fn persist_views(self, views: Vec<SavedView>) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match ipc::set_saved_views(w.path, views).await {
                Ok(stored) => self.saved_views.set(stored),
                Err(e) => self.show_toast(e, None),
            }
        });
    }

    // --- goals ------------------------------------------------------------

    /// Open the detail drawer for goal `id` (closing the task drawer).
    pub fn open_goal(self, id: String) {
        self.active_task.set(None);
        self.active_goal.set(Some(id));
    }

    /// Close the goal detail drawer.
    pub fn close_goal(self) {
        self.active_goal.set(None);
    }

    /// Toggle the board's goal filter: select `id`, or clear it if already active.
    pub fn toggle_goal_filter(self, id: String) {
        self.goal_filter.update(|f| {
            *f = if f.as_deref() == Some(id.as_str()) {
                None
            } else {
                Some(id)
            };
        });
    }

    /// Create a goal, refresh, and open its drawer.
    pub fn add_goal(self) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(id) = ipc::create_goal(w.path.clone()).await {
                self.goals.set(ipc::list_goals(w.path).await);
                self.open_goal(id);
            }
        });
    }

    /// Mark goal `id` as the chip being dragged in the goals bar.
    pub fn start_goal_drag(self, id: String) {
        self.goal_drag.set(Some(id));
    }

    /// Clear goal-drag state.
    pub fn clear_goal_drag(self) {
        self.goal_drag.set(None);
        self.goal_drop_before.set(None);
    }

    /// Commit a goals-bar drag: insert the dragged goal at the drop position
    /// (before `goal_drop_before`, or at the end), renumber, then refresh.
    pub fn commit_goal_drop(self) {
        let Some(id) = self.goal_drag.get_untracked() else {
            self.clear_goal_drag();
            return;
        };
        let before = self.goal_drop_before.get_untracked();
        // The goals signal is already in `order` — drop the dragged id, then
        // re-insert it at the target slot.
        let mut ids: Vec<String> = self
            .goals
            .get_untracked()
            .into_iter()
            .map(|g| g.id)
            .filter(|x| x != &id)
            .collect();
        let pos = match &before {
            Some(b) => ids.iter().position(|x| x == b).unwrap_or(ids.len()),
            None => ids.len(),
        };
        ids.insert(pos, id);
        self.clear_goal_drag();
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::reorder_goals(w.path.clone(), ids).await;
            self.goals.set(ipc::list_goals(w.path).await);
        });
    }

    /// Toggle the archive view (archived tasks + goals) vs the board.
    pub fn toggle_archive(self) {
        self.show_archive.update(|v| *v = !*v);
    }

    // --- calendar -----------------------------------------------------------

    /// Shift the calendar's viewed month by `delta` months (± the ‹ › buttons),
    /// rolling over years.
    pub fn cal_shift_month(self, delta: i32) {
        self.cal_month.update(|(y, m0)| {
            let (ny, nm0, _) = dates::add_months(*y, *m0, 1, delta);
            *y = ny;
            *m0 = nm0;
        });
    }

    /// Jump the calendar back to the current month.
    pub fn cal_go_today(self) {
        let (y, m0, _) = dates::today();
        self.cal_month.set((y, m0));
    }

    /// Replace the board's columns from a comma-separated list (the settings
    /// editor). The backend slugifies, dedupes, and persists to `well.toml`;
    /// an error (no valid names) shows a toast and leaves the board unchanged.
    pub fn set_columns(self, input: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        let names: Vec<String> = input.split(',').map(str::trim).map(String::from).collect();
        spawn_local(async move {
            match ipc::set_task_columns(w.path, names).await {
                Ok(cols) => self.columns.set(cols),
                Err(e) => self.show_toast(e, None),
            }
        });
    }

    /// Set (or clear, with `None`) the auto-archive-done-after-N-days setting
    /// (the settings editor). The backend runs the sweep as part of the write,
    /// so a lowered threshold can archive tasks immediately — refresh the
    /// board so that shows up without waiting for a reopen.
    pub fn set_archive_days(self, days: Option<u32>) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match ipc::set_archive_days(w.path.clone(), days).await {
                Ok(days) => {
                    self.archive_days.set(days);
                    self.tasks.set(ipc::list_tasks(w.path).await);
                }
                Err(e) => self.show_toast(e, None),
            }
        });
    }

    /// Archive (or restore) task `id` — sets/clears its `archived` frontmatter
    /// and refreshes the board (it drops out of / back into the columns).
    pub fn archive_task(self, id: String, archived: bool) {
        let v = if archived { "true" } else { "" };
        self.set_task_field(id, "archived".into(), v.into());
    }

    /// Archive (or restore) goal `id`; clears the goal filter if it pointed here.
    pub fn archive_goal(self, id: String, archived: bool) {
        if archived && self.goal_filter.get_untracked().as_deref() == Some(id.as_str()) {
            self.goal_filter.set(None);
        }
        let v = if archived { "true" } else { "" };
        self.set_goal_field(id, "archived".into(), v.into());
    }

    /// Set (or clear, when blank) a goal field, then refresh.
    pub fn set_goal_field(self, id: String, key: String, value: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            ipc::set_goal_field(w.path.clone(), id, key, value).await;
            self.goals.set(ipc::list_goals(w.path).await);
        });
    }

    /// Persist a goal's body.
    pub fn update_goal_body(self, id: String, body: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move { ipc::update_goal_body(w.path, id, body).await });
    }

    /// Rename goal `id`; keep the drawer + filter pointed at it; refresh goals + tasks
    /// (task `goal:` refs were re-pointed on disk).
    pub fn rename_goal(self, id: String, name: String) {
        if name.trim().is_empty() {
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            match ipc::rename_goal(w.path.clone(), id.clone(), name).await {
                Ok(new_id) => {
                    // Fetch both lists first, then set everything synchronously (one
                    // batch) so the id-keyed goal drawer remounts on the new id with
                    // `state.goals` already refreshed (else it seeds from a stale list).
                    let goals = ipc::list_goals(w.path.clone()).await;
                    let tasks = ipc::list_tasks(w.path).await;
                    let is_active =
                        self.active_goal.get_untracked().as_deref() == Some(id.as_str());
                    let is_filter =
                        self.goal_filter.get_untracked().as_deref() == Some(id.as_str());
                    self.goals.set(goals);
                    self.tasks.set(tasks);
                    if is_active {
                        self.active_goal.set(Some(new_id.clone()));
                    }
                    if is_filter {
                        self.goal_filter.set(Some(new_id));
                    }
                }
                // The goal is unchanged on-disk; refresh anyway to keep memos
                // consistent. The (uncontrolled) title input keeps the rejected
                // text until reopen — the toast signals the rejection.
                Err(e) => {
                    self.goals.set(ipc::list_goals(w.path).await);
                    self.show_toast(e, None);
                }
            }
        });
    }

    /// Delete goal `id` (soft — an undo toast restores the goal file); close its
    /// drawer / clear the filter; refresh goals + tasks.
    pub fn remove_goal(self, id: String) {
        let Some(w) = self.well.get_untracked() else {
            return;
        };
        spawn_local(async move {
            if let Some(content) = ipc::delete_goal(w.path.clone(), id.clone()).await {
                if self.active_goal.get_untracked().as_deref() == Some(id.as_str()) {
                    self.active_goal.set(None);
                }
                if self.goal_filter.get_untracked().as_deref() == Some(id.as_str()) {
                    self.goal_filter.set(None);
                }
                self.goals.set(ipc::list_goals(w.path.clone()).await);
                self.tasks.set(ipc::list_tasks(w.path).await);
                self.show_toast(
                    format!("deleted {id}"),
                    Some(UndoEntry {
                        kind: EntryKind::Goal,
                        id,
                        content,
                    }),
                );
            }
        });
    }
}
