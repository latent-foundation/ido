//! Semantic search: the model download, the index build, and the progress
//! polling that watches them. `impl State` block — see [`super`].
//!
//! The backend runs both long jobs on their own thread and exposes only a
//! snapshot (`job_status`), so this module is the timer that reads it. Polling
//! stops the moment the job does, and the *finish* — not each tick — is what
//! refreshes the derived state the settings pane renders.

use leptos::task::spawn_local;

use super::*;

use crate::ipc;

/// How often to read the running job's progress. Fast enough that a download
/// bar moves smoothly, slow enough to be invisible next to the work itself.
const POLL_MS: u64 = 400;

impl State {
    /// Re-read whether semantic search is available and what this well's index
    /// looks like. Cheap — it stats the index rather than opening it — so the
    /// settings modal calls it on open and after every job.
    pub fn refresh_semantic(self) {
        let well = self.well.get_untracked().map(|w| w.path);
        spawn_local(async move {
            let info = ipc::semantic_info(well).await;
            self.semantic.set(info);
        });
    }

    /// Download the embedding model — the one network request ido ever makes,
    /// and only ever because the user pressed this button.
    pub fn start_model_download(self) {
        spawn_local(async move {
            match ipc::download_model().await {
                Ok(()) => self.watch_job(),
                Err(e) => self.set_job_error(e),
            }
        });
    }

    /// Build (or refresh) the open well's semantic index.
    pub fn start_index_build(self) {
        let Some(well) = self.well.get_untracked().map(|w| w.path) else {
            return;
        };
        spawn_local(async move {
            match ipc::build_index(well).await {
                Ok(()) => self.watch_job(),
                Err(e) => self.set_job_error(e),
            }
        });
    }

    /// Start the progress poll, unless one is already running — two timers
    /// would double the command rate and race each other's writes.
    pub fn watch_job(self) {
        if self.job_polling.get_untracked() {
            return;
        }
        self.job_polling.set(true);
        self.poll_job();
    }

    /// One tick: read the job, and either schedule the next tick or wrap up.
    fn poll_job(self) {
        spawn_local(async move {
            let status = ipc::job_status().await.unwrap_or_default();
            let running = status.running;
            self.job.set(status);
            if running {
                set_timeout(move || self.poll_job(), Duration::from_millis(POLL_MS));
            } else {
                // The job is over: stop polling and re-read the state it
                // changed. Doing this here rather than per tick means the
                // settings pane recomputes once, not ten times a second.
                self.job_polling.set(false);
                self.refresh_semantic();
            }
        });
    }

    /// Record a failure that happened before the job ever started (a refused
    /// claim, an unsupported build), in the same place the UI reads job errors.
    fn set_job_error(self, error: String) {
        self.job.update(|j| {
            j.running = false;
            j.error = Some(error);
        });
    }
}
