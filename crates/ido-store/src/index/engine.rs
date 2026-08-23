//! The process-wide semantic embedder handle (`docs/mcp-server.md` §6.3,
//! §6.6): loaded at most once per process, and only from a model already on
//! disk.
//!
//! This is store-shaped, not server-shaped — it started life inside
//! `ido-mcp::semantic::backend`, but once the desktop app grows an in-app
//! semantic palette (P3) it needs the exact same loaded-once/never-download
//! handle, and a well is opened by one process either way. Living here means
//! there is one cache per process, not a copy per binary that could drift.
//!
//! **Never downloads.** §6.3 makes the model fetch the one network event in
//! the system's life, and it is always user-triggered (a settings-pane
//! button, `--reindex` after a manual fetch) — a lazily-loading handle is not
//! a user. If the model isn't on disk, [`embedder`] returns `None` and
//! [`unavailable`] says why; the caller (ido-mcp's `Semantic`, and later the
//! app) turns that into a degrade-to-keyword report, never an error.

use std::sync::OnceLock;

use super::candle::CandleEmbedder;
use super::download::{model_dir, model_present};
use super::embed::{DEFAULT_MODEL, Embedder};

/// Loaded at most once per process and kept resident for its lifetime (the
/// per-call overhead of reloading a ~130 MB model dominates otherwise). One
/// well per process today, so one live model is the right scope for a
/// `static` rather than a field threaded through every caller. `Err`
/// remembers *why* loading failed, so the reason survives to a status line
/// instead of collapsing to a bare `None`.
static EMBEDDER: OnceLock<Result<CandleEmbedder, String>> = OnceLock::new();

/// The message shown when the model simply isn't on this machine. Never an
/// auto-download — see the module doc.
fn not_downloaded() -> String {
    format!(
        "the embedding model `{}` is not downloaded on this machine (a settings-pane download \
         button fetches it; nothing here downloads automatically)",
        DEFAULT_MODEL.repo
    )
}

/// Why semantic search cannot run here, or `None` when it can. Cheap: it
/// checks for the model's files on disk, never loads them — so a status
/// query never pays the load cost just to answer "is it available".
pub fn unavailable() -> Option<String> {
    match EMBEDDER.get() {
        // Already tried: report exactly what happened.
        Some(Err(reason)) => Some(reason.clone()),
        Some(Ok(_)) => None,
        // Not tried yet — answer from the files on disk rather than loading
        // 133 MB of weights to answer a status question.
        None if !model_present(DEFAULT_MODEL) => Some(not_downloaded()),
        None => None,
    }
}

/// The process-wide embedder: loaded once on first use, and ONLY if the
/// model is already on disk. Never downloads — that is always
/// user-triggered. `None` for any reason [`unavailable`] would name.
pub fn embedder() -> Option<&'static dyn Embedder> {
    let loaded = EMBEDDER.get_or_init(|| {
        if !model_present(DEFAULT_MODEL) {
            return Err(not_downloaded());
        }
        let dir = model_dir(DEFAULT_MODEL)?;
        CandleEmbedder::load(DEFAULT_MODEL, &dir)
    });
    match loaded {
        Ok(embedder) => Some(embedder as &dyn Embedder),
        Err(reason) => {
            eprintln!("semantic embedder unavailable: {reason}");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// This machine may or may not have the model cached, so only the
    /// invariant that holds either way is asserted: unavailability is always
    /// explained, and it always implies no embedder — mirrors the equivalent
    /// test that used to live in `ido-mcp::semantic`.
    #[test]
    fn unavailability_is_always_explained() {
        if let Some(reason) = unavailable() {
            assert!(
                reason.len() > 20,
                "a reason must actually say something: `{reason}`"
            );
            assert!(
                embedder().is_none(),
                "an unavailable backend cannot hand out an embedder"
            );
        }
    }
}
