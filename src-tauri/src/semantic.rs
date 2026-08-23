//! In-app semantic search (`docs/mcp-server.md` P3): the one-time model
//! download, the index build, and the hybrid query — as commands the frontend
//! drives.
//!
//! **Progress is polled, not pushed.** A long job parks its state in [`JOB`]
//! and the frontend reads [`job_status`] on a timer. The app has no Tauri
//! event plumbing and this screen is the only thing that would want it: an
//! event stream would add a capability, a listener, and a dropped-event
//! failure mode, where a poll costs one command per tick and cannot lose an
//! update.
//!
//! Both long jobs run on a plain `std::thread`. They are blocking work — a
//! 133 MB fetch, then minutes of embedding — and must not sit on the async
//! runtime that serves every other command.
//!
//! **Keeping the index fresh** (§6.6). Editing a note or wiki page writes the
//! file and nothing else — re-embedding on every keystroke would be absurd, and
//! the store's own `update_index` is already incremental. So freshness is a
//! *serving* concern, handled exactly where the MCP server handles it: before a
//! semantic/hybrid search, sweep if the index is stale and the debounce has
//! elapsed ([`backend::refresh`]). One difference from the server, and it is
//! deliberate: this side **never builds an index that does not exist yet**.
//! Creating one is minutes of CPU and, before that, a 133 MB download — a thing
//! the user opts into in settings, with a progress bar, not something a
//! keystroke in the palette starts behind their back.
//!
//! The `semantic` feature (on by default here, unlike in `ido-mcp`) decides
//! whether any of this can actually run: [`backend`] has a real half and a
//! stub half, and every command below answers honestly either way rather than
//! pretending the feature exists. That is the same degradation contract the
//! MCP server follows — keyword results plus a reason, never an error.

use std::sync::{LazyLock, Mutex};

use ido_store::index::IndexStatus;
use ido_store::model::SearchHit;

/// The single background job's state. At most one runs at a time: the two jobs
/// are strictly ordered in practice (you cannot index without the model) and a
/// second concurrent embed would only contend for the same cores.
static JOB: LazyLock<Mutex<JobStatus>> = LazyLock::new(|| Mutex::new(JobStatus::idle()));

/// What the background job is doing, as the frontend polls it.
#[derive(Clone, serde::Serialize, Default)]
pub struct JobStatus {
    /// `""` when idle, else `"download"` or `"index"`.
    pub kind: String,
    /// Human-readable phase, shown verbatim ("fetching model.safetensors").
    pub phase: String,
    /// Progress numerator — bytes for a download, chunks for an index build.
    pub done: u64,
    /// Denominator, or `0` while it is still unknown.
    pub total: u64,
    /// True while the job is running.
    pub running: bool,
    /// Why the last job failed. Cleared when the next one starts.
    pub error: Option<String>,
    /// Bumped once per finished job, so a polling UI can tell "still the same
    /// failure" from "a new one" and refresh derived state exactly once.
    pub generation: u64,
}

impl JobStatus {
    /// The resting state: nothing running, nothing to report.
    fn idle() -> Self {
        Self::default()
    }
}

/// Read the job state, poisoning-tolerantly — a panicked job thread must not
/// wedge the UI's polling into an error state forever.
fn job() -> std::sync::MutexGuard<'static, JobStatus> {
    JOB.lock().unwrap_or_else(|e| e.into_inner())
}

/// Claim the job slot for `kind`, or report who already has it. Returns `Err`
/// with a message the UI can show rather than silently starting a second job.
fn claim(kind: &str, phase: &str) -> Result<(), String> {
    let mut state = job();
    if state.running {
        return Err(format!(
            "already {} — wait for it to finish",
            match state.kind.as_str() {
                "download" => "downloading the model",
                "index" => "building the index",
                _ => "busy",
            }
        ));
    }
    *state = JobStatus {
        kind: kind.to_string(),
        phase: phase.to_string(),
        running: true,
        generation: state.generation,
        ..JobStatus::idle()
    };
    Ok(())
}

/// Record progress on the running job. Called from worker threads, including
/// several at once during an index build.
fn progress(phase: &str, done: u64, total: u64) {
    let mut state = job();
    if !state.running {
        return;
    }
    if state.phase != phase {
        state.phase = phase.to_string();
    }
    state.done = done;
    state.total = total;
}

/// Release the job slot, recording `error` when it failed.
fn finish(error: Option<String>) {
    let mut state = job();
    state.running = false;
    state.error = error;
    state.phase = String::new();
    state.generation = state.generation.wrapping_add(1);
}

// --- what this build/machine can do ----------------------------------------

/// Everything the settings pane needs to describe semantic search: whether it
/// can run, what it would download, and the state of this well's index.
#[derive(Clone, serde::Serialize)]
pub struct SemanticInfo {
    /// True when a query could be answered semantically right now.
    pub available: bool,
    /// Why not, when `available` is false — shown to the user verbatim.
    pub reason: Option<String>,
    /// True when this build has the `semantic` feature compiled in at all.
    pub supported: bool,
    /// The embedding model's id (its HuggingFace repo).
    pub model: String,
    /// Whether the weights are already on disk.
    pub model_present: bool,
    /// What downloading them would cost, in bytes.
    pub download_bytes: u64,
    /// Where the weights live (or would live) on this machine.
    pub model_dir: Option<String>,
    /// This well's index, when one has been built.
    pub index: Option<IndexStatus>,
}

/// One `search_hybrid` response: the hits plus **which retrieval actually
/// ran**, so the palette can say "semantic" only when it means it.
///
/// Not `Clone`: it is built once per query and serialised straight out, and
/// `SearchHit` upstream isn't cloneable either.
#[derive(serde::Serialize)]
pub struct SearchResponse {
    /// Ranked hits, the same shape the keyword `search` command returns.
    pub hits: Vec<SearchHit>,
    /// The mode that produced them: `"hybrid"`, `"semantic"`, or `"keyword"`.
    pub mode: String,
    /// Why the requested mode could not run, when it could not.
    pub degraded: Option<String>,
}

// --- commands ---------------------------------------------------------------

/// Describe semantic search for `well`: availability, the model, the index.
/// Cheap enough to call whenever the settings modal opens.
#[tauri::command]
pub fn semantic_info(well: Option<String>) -> SemanticInfo {
    backend::info(well.as_deref())
}

/// Start the one-time model download on a background thread. Returns as soon
/// as the job is claimed; watch [`job_status`] for progress.
///
/// This is the only network request the app ever makes, and it happens solely
/// because a user pressed a button (`docs/mcp-server.md` §9).
#[tauri::command]
pub fn download_model() -> Result<(), String> {
    backend::start_download()
}

/// Start (or refresh) `well`'s semantic index on a background thread. Returns
/// as soon as the job is claimed; watch [`job_status`] for progress.
#[tauri::command]
pub fn build_index(well: String) -> Result<(), String> {
    backend::start_index(well)
}

/// The background job's current state. Polled by the settings pane.
#[tauri::command]
pub fn job_status() -> JobStatus {
    job().clone()
}

/// Search `well` in `mode` (`"hybrid"` by default), degrading to keyword — and
/// saying so — whenever the semantic half cannot run.
///
/// `async` + `spawn_blocking` because this can do real work before it answers:
/// a stale index gets a (debounced, incremental) sweep first, so the results
/// reflect what the user just wrote rather than what they wrote last time they
/// pressed the button in settings. The frontend already awaits this call, so
/// nothing changes on that side.
#[tauri::command]
pub async fn search_hybrid(
    well: String,
    query: String,
    mode: Option<String>,
    limit: Option<u32>,
) -> SearchResponse {
    tauri::async_runtime::spawn_blocking(move || {
        backend::search(&well, &query, mode.as_deref(), limit)
    })
    .await
    .unwrap_or_else(|_| SearchResponse {
        // The task can only fail by panicking, which is a bug rather than a
        // degradation — but the palette still has to render something, and an
        // empty list with no explanation is the one answer it must never give.
        hits: Vec::new(),
        mode: "keyword".to_string(),
        degraded: Some("the search task failed unexpectedly".to_string()),
    })
}

// --- the two halves ---------------------------------------------------------

/// The real implementation, when the `semantic` feature is on.
#[cfg(feature = "semantic")]
mod backend {
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::{Duration, Instant};

    use ido_store::index::download::{DownloadProgress, ensure_model_with_progress, model_dir};
    use ido_store::index::embed::DEFAULT_MODEL;
    use ido_store::index::hybrid::{SearchMode, search_with};
    use ido_store::index::store::{update_index, update_index_with_progress};
    use ido_store::index::{engine, index_status};

    use super::{JobStatus, SearchResponse, SemanticInfo, claim, finish, progress};

    pub fn info(well: Option<&str>) -> SemanticInfo {
        let reason = engine::unavailable();
        SemanticInfo {
            available: reason.is_none(),
            reason,
            supported: true,
            model: DEFAULT_MODEL.repo.to_string(),
            model_present: ido_store::index::download::model_present(DEFAULT_MODEL),
            download_bytes: DEFAULT_MODEL.download_bytes,
            model_dir: model_dir(DEFAULT_MODEL)
                .ok()
                .map(|p| p.to_string_lossy().into_owned()),
            index: well.and_then(index_status),
        }
    }

    pub fn start_download() -> Result<(), String> {
        claim("download", "starting")?;
        std::thread::spawn(|| {
            let result = ensure_model_with_progress(DEFAULT_MODEL, &mut |p: DownloadProgress| {
                // One bar for the whole model rather than one per file: the
                // user is waiting for "the model", and three bars that each
                // restart read as three downloads.
                progress(
                    &format!(
                        "fetching {} ({}/{})",
                        p.file,
                        p.file_index + 1,
                        p.file_count
                    ),
                    p.bytes,
                    p.file_total.unwrap_or(0),
                );
            });
            finish(result.err());
        });
        Ok(())
    }

    pub fn start_index(well: String) -> Result<(), String> {
        if let Some(reason) = engine::unavailable() {
            return Err(reason);
        }
        claim("index", "reading the well")?;
        std::thread::spawn(move || {
            let Some(embedder) = engine::embedder() else {
                finish(Some("the embedding model is not loaded".to_string()));
                return;
            };
            // Batches finish on several rayon threads, so the counter the
            // callback reports from has to be atomic; the mutex behind
            // `progress` then serialises the writes.
            let seen = AtomicU64::new(0);
            let result = update_index_with_progress(&well, embedder, &|done, total| {
                seen.store(done as u64, Ordering::Relaxed);
                progress("embedding", done as u64, total as u64);
            });
            let _ = seen;
            finish(result.err());
        });
        Ok(())
    }

    /// How long after a sweep the next search trusts the index without
    /// re-checking. Matches `ido-mcp`'s `SWEEP_DEBOUNCE` — the same policy in
    /// the same situation, so a well served by both behaves one way.
    const SWEEP_DEBOUNCE: Duration = Duration::from_secs(30);

    /// When the last sweep was claimed. `Instant` can't be a const, but `None`
    /// can, so this needs no `LazyLock`.
    pub(super) static LAST_SWEEP: Mutex<Option<Instant>> = Mutex::new(None);

    /// Take the debounce slot: `true` when this caller should sweep. Claimed
    /// *before* the work, so a burst of searches costs one sweep rather than
    /// each deciding independently to rebuild.
    pub(super) fn claim_sweep() -> bool {
        let Ok(mut last) = LAST_SWEEP.lock() else {
            return false; // poisoned: a previous sweep panicked — don't pile on
        };
        if last.is_some_and(|at| at.elapsed() < SWEEP_DEBOUNCE) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }

    /// Bring an **existing** index up to date before a semantic search, if it
    /// is stale and the debounce has elapsed. Blocking, and deliberately quiet.
    ///
    /// Three things it declines to do, each for its own reason:
    /// - **No index yet ⇒ do nothing.** Building one is the user's explicit
    ///   choice in settings (see the module doc); a search must not start it.
    /// - **A job already running ⇒ do nothing.** The settings pane owns the job
    ///   slot, and its build is about to make this sweep redundant anyway.
    /// - **A held lock is not an error.** `ido-mcp` may be sweeping the same
    ///   well; the work is happening, so answer from the index we have.
    pub(super) fn refresh(well: &str) {
        let Some(embedder) = engine::embedder() else {
            return;
        };
        // `Some(status)` = an index exists. `None` = there is none to refresh.
        if !index_status(well).is_some_and(|s| s.stale) {
            return;
        }
        if super::job().running || !claim_sweep() {
            return;
        }
        match update_index(well, embedder) {
            Ok(_) => {}
            // Another process is already on it — a normal state, not a warning.
            Err(reason) if reason.contains("locked") => {}
            Err(reason) => eprintln!("ido: index refresh failed: {reason}"),
        }
    }

    pub fn search(
        well: &str,
        query: &str,
        mode: Option<&str>,
        limit: Option<u32>,
    ) -> SearchResponse {
        let requested = match mode.map(SearchMode::parse) {
            Some(Ok(m)) => m,
            // An unknown mode from our own frontend is a bug, not user input;
            // answer it the safest way rather than failing the search.
            Some(Err(_)) | None => SearchMode::Hybrid,
        };
        // Only the two modes that actually consult the vectors pay for
        // freshness; an explicit keyword search always reads live files.
        if matches!(requested, SearchMode::Hybrid | SearchMode::Semantic) {
            refresh(well);
        }
        let outcome = search_with(
            well,
            query,
            requested,
            limit.unwrap_or(50) as usize,
            engine::embedder(),
        );
        SearchResponse {
            hits: outcome.hits,
            mode: outcome.mode.as_str().to_string(),
            degraded: outcome.degraded,
        }
    }

    /// Unused in this half — the stub half needs it, and a single `use` keeps
    /// the two signatures honest.
    #[allow(dead_code)]
    fn _assert_job_status(_: JobStatus) {}
}

/// The stub half, when the `semantic` feature is off: keyword search still
/// works, and everything else says why it doesn't.
#[cfg(not(feature = "semantic"))]
mod backend {
    use ido_store::model::SearchHit;

    use super::{SearchResponse, SemanticInfo};

    /// The one message this half has to give, phrased for a user rather than
    /// a compiler.
    const OFF: &str = "this build of ido was compiled without semantic search";

    pub fn info(_well: Option<&str>) -> SemanticInfo {
        SemanticInfo {
            available: false,
            reason: Some(OFF.to_string()),
            supported: false,
            model: String::new(),
            model_present: false,
            download_bytes: 0,
            model_dir: None,
            index: None,
        }
    }

    pub fn start_download() -> Result<(), String> {
        Err(OFF.to_string())
    }

    pub fn start_index(_well: String) -> Result<(), String> {
        Err(OFF.to_string())
    }

    pub fn search(
        well: &str,
        query: &str,
        _mode: Option<&str>,
        limit: Option<u32>,
    ) -> SearchResponse {
        let hits: Vec<SearchHit> = ido_store::search::search(well.to_string(), query.to_string())
            .into_iter()
            .take(limit.unwrap_or(50) as usize)
            .collect();
        SearchResponse {
            hits,
            mode: "keyword".to_string(),
            degraded: Some(OFF.to_string()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// [`JOB`] is process-global and cargo runs tests in parallel, so these
    /// three would interleave each other's claims and finishes. Serialising
    /// them here is the honest fix: the alternative — one giant test — would
    /// hide which behaviour broke.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Take the serialising lock and reset the slot. Poisoning is ignored: one
    /// failing test should not cascade into the others.
    fn exclusive() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *job() = JobStatus::idle();
        guard
    }

    /// Also clear the sweep debounce, which is process-global for the same
    /// reason [`JOB`] is.
    #[cfg(feature = "semantic")]
    fn reset_sweep() {
        *backend::LAST_SWEEP
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = None;
    }

    #[cfg(feature = "semantic")]
    #[test]
    fn the_sweep_debounce_admits_one_caller_then_holds() {
        let _exclusive = exclusive();
        reset_sweep();
        assert!(backend::claim_sweep(), "the first caller sweeps");
        assert!(
            !backend::claim_sweep(),
            "a burst of searches must cost one sweep, not one each"
        );
        reset_sweep();
        assert!(
            backend::claim_sweep(),
            "the slot frees once the window is up"
        );
    }

    /// The rule the palette depends on: searching a well that has never been
    /// indexed must not quietly start minutes of embedding. Building an index
    /// is the settings pane's job, with a progress bar the user asked for.
    #[cfg(feature = "semantic")]
    #[test]
    fn a_search_never_builds_an_index_that_does_not_exist() {
        let _exclusive = exclusive();
        reset_sweep();
        let dir = tempfile::tempdir().unwrap();
        for section in ["notes", "wiki", "tasks"] {
            std::fs::create_dir_all(dir.path().join(section)).unwrap();
        }
        std::fs::write(dir.path().join("notes/a.md"), "some text").unwrap();
        let well = dir.path().to_string_lossy().into_owned();

        backend::refresh(&well);

        assert!(
            !dir.path().join(".ido/index").exists(),
            "refresh created an index for a well that had none"
        );
        assert!(
            backend::claim_sweep(),
            "declining to build must not burn the debounce slot"
        );
    }

    /// A build running in the settings pane owns the index; a search must not
    /// race it, even once the debounce is free.
    #[cfg(feature = "semantic")]
    #[test]
    fn a_search_defers_to_a_running_settings_job() {
        let _exclusive = exclusive();
        reset_sweep();
        claim("index", "reading the well").unwrap();
        backend::refresh("./definitely-not-a-well-xyz");
        assert!(
            backend::claim_sweep(),
            "deferring to the job must leave the debounce slot untaken"
        );
        finish(None);
    }

    #[test]
    fn a_second_job_is_refused_while_one_runs() {
        let _exclusive = exclusive();
        claim("download", "starting").unwrap();
        let err = claim("index", "reading the well").unwrap_err();
        assert!(
            err.contains("downloading the model"),
            "the refusal names what is already running: {err}"
        );
        finish(None);
        claim("index", "reading the well").expect("the slot frees on finish");
        finish(None);
    }

    #[test]
    fn progress_is_ignored_once_a_job_has_finished() {
        let _exclusive = exclusive();
        claim("index", "reading the well").unwrap();
        progress("embedding", 5, 10);
        assert_eq!(job().done, 5);
        finish(None);
        // A worker thread that reports one last tick after the job was torn
        // down must not resurrect a running state in the UI.
        progress("embedding", 9, 10);
        let state = job();
        assert!(!state.running);
        assert_eq!(state.done, 5, "the late tick was dropped");
        drop(state);
    }

    #[test]
    fn a_failure_is_kept_until_the_next_job_starts() {
        let _exclusive = exclusive();
        claim("download", "starting").unwrap();
        finish(Some("no network".to_string()));
        assert_eq!(job().error.as_deref(), Some("no network"));
        let before = job().generation;
        claim("download", "starting").unwrap();
        assert!(job().error.is_none(), "a new job clears the old failure");
        finish(None);
        assert_eq!(
            job().generation,
            before.wrapping_add(1),
            "each finished job bumps the generation exactly once"
        );
    }
}
