//! The one-time model download and the per-machine model cache
//! (docs/mcp-server.md §6.3 "model download and cache").
//!
//! This is the **only** network event in the system's entire life (§9): three
//! files from one public HuggingFace repo, fetched with ureq + rustls into
//! `<data_dir>/latent.ido/models/{repo}/` — beside the recent-wells registry,
//! *not* `.ido/` (models are per-machine and shared across wells; `.ido/` is
//! per-well rebuildable cache). Each file downloads to a temp path, is
//! sha256-verified against the pin, then renamed into place — a partial
//! download is treated as absent.

use std::path::PathBuf;

use super::embed::ModelSpec;

/// Where `spec`'s files live (or would live) on this machine. Honors an
/// `IDO_MODEL_DIR` override (tests, evals); the repo's `/` is flattened so
/// one directory holds one model.
pub fn model_dir(_spec: &ModelSpec) -> Result<PathBuf, String> {
    todo!("P2 wave 1: model download (docs/mcp-server.md §6.3)")
}

/// True when every required file is present (existence + size — the sha256
/// was verified at download time).
pub fn model_present(_spec: &ModelSpec) -> bool {
    todo!("P2 wave 1: model download (docs/mcp-server.md §6.3)")
}

/// Ensure `spec`'s files are on disk, downloading whatever is missing.
/// Returns the model directory. User-triggered only — never called on a
/// server's startup path.
pub fn ensure_model(_spec: &'static ModelSpec) -> Result<PathBuf, String> {
    todo!("P2 wave 1: model download (docs/mcp-server.md §6.3)")
}
