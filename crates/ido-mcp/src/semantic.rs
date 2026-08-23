//! Semantic search's process-level state: the resident embedder, the index
//! freshness policy, and the one honest sentence to say when none of it is
//! available (`docs/mcp-server.md` §6.1, §6.3, §6.6).
//!
//! Everything model-shaped lives behind the `semantic` cargo feature, in one of
//! the two [`backend`] modules below — the real one, and a stub whose
//! `unavailable()` is always `Some`. The rest of the server is written against
//! the same [`Semantic`] handle either way, so a keyword-only build has no
//! `#[cfg]` scattered through it and no branch that can silently claim more
//! than it can do.
//!
//! **What this writes.** Nothing, unless the feature is on *and* the model is
//! downloaded — and then only `<well>/.ido/index/**`, which is rebuildable
//! cache, never well content. §6.6 sanctions either process maintaining it, and
//! `store::update_index` takes the advisory lock, so the app and this server
//! can both sweep without fighting.
//!
//! **When it sweeps** (§6.6): once in the background at startup, and again
//! before a semantic/hybrid search when the index is stale *and* it has been
//! more than [`SWEEP_DEBOUNCE`] since the last sweep — the same debounce
//! instinct `search_gen` uses for the app's palette. A held lock is not an
//! error: the other process is already doing the work, so this one answers from
//! the index it has and reports `stale` in `well_info`.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ido_store::index::embed::Embedder;

/// How long after a sweep the next search trusts the index without re-checking.
/// Long enough that a burst of searches costs one stat sweep, short enough that
/// a note written in the app shows up in the next answer but one.
const SWEEP_DEBOUNCE: Duration = Duration::from_secs(30);

/// One well's semantic-search state, cheap to clone (the SDK clones the handler
/// per connection).
#[derive(Clone)]
pub struct Semantic {
    /// The well being served — the `well` argument every store call takes.
    well: String,
    /// When the last sweep was *claimed* (not finished), so concurrent searches
    /// debounce against each other rather than all queueing a rebuild.
    last_sweep: Arc<Mutex<Option<Instant>>>,
}

impl Semantic {
    /// The state for `well`. Loads nothing — the model is loaded lazily, off
    /// the request path, so a keyword-only client never pays for it.
    pub fn new(well: &str) -> Self {
        Self {
            well: well.to_string(),
            last_sweep: Arc::new(Mutex::new(None)),
        }
    }

    /// Why semantic search can't run, or `None` when it can. Cheap: it checks
    /// for the model's files, never loads them.
    pub fn unavailable(&self) -> Option<String> {
        backend::unavailable()
    }

    /// The resident embedder, loading it on first use. `None` — for any reason
    /// [`Semantic::unavailable`] would name — means every mode degrades to
    /// keyword. **Blocking**: call it from a blocking context.
    pub fn embedder(&self) -> Option<&'static dyn Embedder> {
        backend::embedder()
    }

    /// Kick the startup sweep (§6.6) on a background blocking thread and return
    /// immediately: the server must be answering `tools/list` in milliseconds,
    /// not after a cold index build. A no-op when semantic search is
    /// unavailable, so a default-feature build spawns nothing at all.
    pub fn start(&self) {
        if self.unavailable().is_some() {
            return;
        }
        self.claim_sweep();
        let well = self.well.clone();
        // Detached on purpose: nothing waits on this, and the runtime outlives
        // it (main is parked on `serve`).
        tokio::task::spawn_blocking(move || backend::sweep(&well));
    }

    /// Before a semantic/hybrid search: sweep if the index is stale and the
    /// debounce has elapsed. Awaited, so the search that follows sees the
    /// refreshed index — but bounded by the debounce, so a chatty client can't
    /// turn every query into a stat sweep of the whole well.
    pub async fn refresh(&self) {
        if self.unavailable().is_some() || !self.claim_sweep() {
            return;
        }
        let well = self.well.clone();
        let _ = tokio::task::spawn_blocking(move || backend::sweep(&well)).await;
    }

    /// Take the debounce slot: `true` when this caller should sweep. Claiming
    /// *before* the work (rather than stamping after) is what keeps a burst of
    /// concurrent searches from all deciding to rebuild.
    fn claim_sweep(&self) -> bool {
        let Ok(mut last) = self.last_sweep.lock() else {
            return false; // poisoned: another sweep panicked — don't pile on
        };
        if last.is_some_and(|at| at.elapsed() < SWEEP_DEBOUNCE) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }

    /// The `well_info` lines describing search: which modes are live, and the
    /// index's model / size / last build / staleness (§5.2). Two lines, no
    /// trailing newline.
    pub fn status_lines(&self) -> String {
        let status = ido_store::index::index_status(&self.well);
        let search = match self.unavailable() {
            Some(reason) => format!(
                "- search: keyword-only — semantic search unavailable: {reason}. Every `mode` \
                 falls back to keyword and says so."
            ),
            None if status.is_none() => "- search: keyword-only for now — the semantic index \
                 has not been built yet (it builds in the background; ask again shortly). \
                 `mode` falls back to keyword until then."
                .to_string(),
            None => "- search: modes `hybrid` (default), `semantic`, `keyword` are all live."
                .to_string(),
        };
        let index = match status {
            None => "- semantic index: not built".to_string(),
            Some(s) => format!(
                "- semantic index: {} ({}-dim), {} chunks across {} files, built {} ({})",
                s.model,
                s.dim,
                s.chunks,
                s.files,
                crate::render::iso_date(Some(s.built_ms)),
                if s.stale {
                    "stale — a sweep runs before the next semantic search"
                } else {
                    "fresh"
                }
            ),
        };
        format!("{search}\n{index}")
    }
}

// --- the two backends -------------------------------------------------------

/// The real thing: the loaded-once candle embedder handle — now
/// `ido_store::index::engine`, moved out of here once the desktop app became
/// a second consumer of the exact same load-if-present/never-download
/// contract — plus this server's own manifest-driven sweep policy, which
/// stays put (§6.6's sweep/debounce is a serving concern, not a store one).
#[cfg(feature = "semantic")]
mod backend {
    use ido_store::index::embed::Embedder;
    use ido_store::index::engine;
    use ido_store::index::store::{index_status, update_index};

    /// Delegates to [`engine::unavailable`] — the handle (and its
    /// error-message logic) lives in the store now.
    pub fn unavailable() -> Option<String> {
        engine::unavailable()
    }

    /// Delegates to [`engine::embedder`].
    pub fn embedder() -> Option<&'static dyn Embedder> {
        engine::embedder()
    }

    /// Bring `.ido/index/` up to date if it is missing or stale. Blocking, and
    /// deliberately quiet: a lock held by the app (or another server) means the
    /// work is already happening, which is a normal state, not a warning.
    pub fn sweep(well: &str) {
        let Some(embedder) = embedder() else {
            return;
        };
        if index_status(well).is_some_and(|s| !s.stale) {
            return;
        }
        match update_index(well, embedder) {
            Ok(status) => eprintln!(
                "ido-mcp: index up to date — {} chunks across {} files ({})",
                status.chunks, status.files, status.model
            ),
            Err(reason) if reason.contains("locked") => {}
            Err(reason) => eprintln!("ido-mcp: index update failed: {reason}"),
        }
    }
}

/// The keyword-only build: no model, no sweep, **no writes anywhere**.
#[cfg(not(feature = "semantic"))]
mod backend {
    use ido_store::index::embed::Embedder;

    pub fn unavailable() -> Option<String> {
        Some(
            "this build has no semantic support (compiled without the `semantic` cargo feature)"
                .to_string(),
        )
    }

    pub fn embedder() -> Option<&'static dyn Embedder> {
        None
    }

    pub fn sweep(_well: &str) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A keyword-only build must refuse, and say why. A semantic build's answer
    /// depends on whether this machine happens to have the model cached, so
    /// only the invariant that holds either way is asserted: unavailability is
    /// always explained, and it always implies no embedder.
    #[test]
    fn unavailability_is_always_explained() {
        let semantic = Semantic::new("./no-such-well");
        if let Some(reason) = semantic.unavailable() {
            assert!(
                reason.len() > 20,
                "a reason must actually say something: `{reason}`"
            );
            assert!(
                semantic.embedder().is_none(),
                "an unavailable backend cannot hand out an embedder"
            );
        }
    }

    #[cfg(not(feature = "semantic"))]
    #[test]
    fn a_keyword_only_build_never_claims_semantic_search() {
        let semantic = Semantic::new("./no-such-well");
        let reason = semantic.unavailable().expect("no feature, no semantic");
        assert!(reason.contains("semantic"), "{reason}");
        semantic.start(); // must not panic, must not spawn, must not write
    }

    #[test]
    fn status_lines_are_two_and_name_the_index_state() {
        let semantic = Semantic::new("./no-such-well");
        let status = semantic.status_lines();
        let lines: Vec<&str> = status.lines().collect();
        assert_eq!(lines.len(), 2, "one search line, one index line");
        assert!(lines[0].starts_with("- search: "), "{}", lines[0]);
        assert_eq!(
            lines[1], "- semantic index: not built",
            "a well with no .ido/index has no index to describe"
        );
    }

    #[test]
    fn the_debounce_admits_one_caller_then_holds() {
        let semantic = Semantic::new("./no-such-well");
        assert!(semantic.claim_sweep(), "the first caller sweeps");
        assert!(
            !semantic.claim_sweep(),
            "a second caller inside the window does not"
        );
    }
}
