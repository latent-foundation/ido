//! The cross-section search palette + command dispatch. `impl State` block —
//! see [`super`].

use leptos::prelude::*;
use leptos::task::spawn_local;

use super::*;

use crate::ipc;

impl State {
    // --- search -----------------------------------------------------------

    /// Open the cross-section search palette, ready for input (offering the
    /// commands until something is typed).
    pub fn open_search(self) {
        self.search_query.set(String::new());
        self.search_results.set(command_hits(""));
        self.search_sel.set(0);
        self.search_open.set(true);
    }

    /// Close the search palette.
    pub fn close_search(self) {
        self.search_open.set(false);
    }

    /// Run `query`: matching commands first, then content hits (dropping a stale
    /// response if the query has changed). An empty query shows all commands.
    ///
    /// The disk scan is **debounced** — each keystroke bumps `search_gen` and
    /// schedules the scan a beat later, so a fast typist triggers one scan, not
    /// one per character.
    pub fn run_search(self, query: String) {
        self.search_query.set(query.clone());
        self.search_sel.set(0);
        if query.trim().is_empty() {
            self.search_gen.update(|g| *g = g.wrapping_add(1));
            self.search_results.set(command_hits(""));
            return;
        }
        let Some(w) = self.well.get_untracked() else {
            self.search_results.set(command_hits(&query));
            return;
        };
        let gen = self.search_gen.get_untracked().wrapping_add(1);
        self.search_gen.set(gen);
        set_timeout(
            move || {
                // Superseded by a newer keystroke — this scan is stale, skip it.
                if self.search_gen.get_untracked() != gen {
                    return;
                }
                let path = w.path.clone();
                spawn_local(async move {
                    let hits = ipc::search(path, query.clone()).await;
                    if self.search_query.get_untracked() == query {
                        let mut combined = command_hits(&query);
                        combined.extend(hits);
                        self.search_results.set(combined);
                        self.search_sel.set(0);
                    }
                });
            },
            std::time::Duration::from_millis(120),
        );
    }

    /// Move the highlighted result by `delta`, clamped to the result range.
    pub fn search_move(self, delta: i32) {
        let n = self.search_results.get_untracked().len();
        if n == 0 {
            return;
        }
        let cur = self.search_sel.get_untracked() as i32;
        self.search_sel
            .set((cur + delta).clamp(0, n as i32 - 1) as usize);
    }

    /// Open the currently-highlighted result (no-op when there are none).
    pub fn open_selected_search(self) {
        let results = self.search_results.get_untracked();
        if let Some(hit) = results.get(self.search_sel.get_untracked()) {
            self.open_search_hit(hit.kind.clone(), hit.id.clone());
        }
    }

    /// Switch to a `kind` (`note` / `wiki` / `task` / `goal`) entry's section and
    /// open it (a note/wiki tab in the focused pane, or the task/goal drawer),
    /// or run a `"cmd"` palette command.
    pub fn open_entry(self, kind: String, id: String) {
        match kind.as_str() {
            "cmd" => self.run_command(&id),
            "wiki" => {
                self.section.set(Section::Wiki);
                self.open_wiki(id);
            }
            "task" => {
                self.section.set(Section::Tasks);
                self.open_task(id);
            }
            "goal" => {
                self.section.set(Section::Tasks);
                self.open_goal(id);
            }
            _ => {
                self.section.set(Section::Notes);
                self.open_note(id);
            }
        }
    }

    /// Dispatch a palette command (closing the palette first).
    fn run_command(self, id: &str) {
        self.close_search();
        match id {
            "cmd:new-note" => {
                self.section.set(Section::Notes);
                self.add_note(String::new());
            }
            "cmd:new-page" => {
                self.section.set(Section::Wiki);
                self.add_page();
            }
            "cmd:new-task" => {
                self.section.set(Section::Tasks);
                let status = self
                    .columns
                    .get_untracked()
                    .first()
                    .cloned()
                    .unwrap_or_default();
                self.add_task(status);
            }
            "cmd:theme" => {
                if let Some(theme) = use_context::<RwSignal<bool>>() {
                    theme.update(|d| *d = !*d);
                }
            }
            "cmd:settings" => self.settings_open.set(true),
            _ => {}
        }
    }

    /// Open a search hit, then close the palette.
    pub fn open_search_hit(self, kind: String, id: String) {
        self.open_entry(kind, id);
        self.close_search();
    }
}
