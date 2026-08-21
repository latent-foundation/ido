//! The on-disk vector index under `<well>/.ido/index/`, its manifest-driven
//! incremental rebuild, and brute-force cosine top-k (docs/mcp-server.md
//! §6.4, §6.6).
//!
//! Format:
//! - `manifest.json` — schema version, model id, dim, per-file
//!   `{sha256, mtime, size, chunk ids}`
//! - `vectors.bin` — raw little-endian f32, row-major, one row per
//!   `chunks.jsonl` line, in line order
//! - `chunks.jsonl` — one serialized [`Chunk`] per line
//! - `.lock` — advisory lock around rebuilds (stale after a timeout); a
//!   reader with a stale index answers from what it has rather than blocking
//!
//! No ANN structure, deliberately: a personal well is two orders of magnitude
//! too small for HNSW to earn its complexity (§6.4's napkin math) — a dot
//! product sweep over an in-memory f32 matrix answers in well under a
//! millisecond. Revisit only past ~100k chunks; `IndexStatus::chunks` makes
//! the trigger observable.
//!
//! Incremental strategy: the expensive stage is embedding, not IO. A rebuild
//! re-embeds only files whose `(mtime, size)` — then sha256 — changed,
//! reuses every other file's existing vectors, and rewrites all three files
//! atomically (temp + rename), so chunk ids stay dense and the trio is never
//! mutually inconsistent.

use std::collections::BTreeMap;

use super::IndexStatus;
use super::chunk::Chunk;
use super::embed::Embedder;

/// The index directory, well-relative. Lives under `.ido/` because it is
/// rebuildable cache — never index `.ido/` or `assets/` themselves.
pub const INDEX_DIR: &str = ".ido/index";

/// Schema version stamped in the manifest; bump on any format change to force
/// a clean rebuild instead of a misread.
pub const SCHEMA_VERSION: u32 = 1;

/// One indexed file's fingerprint + its rows.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    /// Content hash — the change detector of record.
    pub sha256: String,
    /// Modified time (epoch ms) at index time — the cheap first-pass filter.
    pub mtime_ms: u64,
    /// File size in bytes — second cheap filter.
    pub size: u64,
    /// Row indices (into `chunks.jsonl` / `vectors.bin`) holding this file's
    /// chunks.
    pub chunks: Vec<u32>,
}

/// `manifest.json`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct IndexManifest {
    /// [`SCHEMA_VERSION`] at write time.
    pub schema: u32,
    /// The embedder id that built every vector in this index.
    pub model: String,
    /// Vector width.
    pub dim: usize,
    /// Last (re)build, epoch ms.
    pub built_ms: u64,
    /// Well-relative file path → fingerprint, sorted for stable diffs.
    pub files: BTreeMap<String, FileEntry>,
}

/// A loaded index: the manifest, every chunk, and the vector matrix
/// (row-major, `manifest.dim` wide, one row per chunk).
pub struct VectorIndex {
    pub manifest: IndexManifest,
    pub chunks: Vec<Chunk>,
    pub vectors: Vec<f32>,
}

impl VectorIndex {
    /// Brute-force cosine top-k: `query` is a unit vector (so dot product =
    /// cosine); returns `(chunk index, score)` best-first, at most `top_k`.
    pub fn query(&self, _query: &[f32], _top_k: usize) -> Vec<(usize, f32)> {
        todo!("P2 wave 1: the vector store (docs/mcp-server.md §6.4)")
    }

    /// The chunk behind a [`VectorIndex::query`] hit.
    pub fn chunk(&self, i: usize) -> &Chunk {
        &self.chunks[i]
    }
}

/// Walk the well and chunk every indexable entry, grouped by the
/// well-relative path of the backing file (the manifest key). Covers notes
/// (recursive), wiki pages (recursive — folders are cosmetic but real
/// directories), tasks and goals (via the task store, so titles/tags/goal
/// links resolve); skips hidden entries, `.ido/`, `assets/`, and archived
/// tasks/goals (lexical search excludes them too — the two halves must see
/// the same corpus). Never follows a path out of the well.
pub fn collect_chunks(_well: &str) -> Vec<(String, Vec<Chunk>)> {
    todo!("P2 wave 1: the vector store (docs/mcp-server.md §6.6)")
}

/// Bring `.ido/index/` up to date against the well, re-embedding only what
/// changed (see the module doc). Creates the index when absent; a manifest
/// whose `model`/`dim`/`schema` disagree with `embedder` forces a full
/// rebuild. Takes the advisory lock; if another process holds a fresh lock,
/// returns `Err` without touching anything (callers answer from the stale
/// index instead).
pub fn update_index(_well: &str, _embedder: &dyn Embedder) -> Result<IndexStatus, String> {
    todo!("P2 wave 1: the vector store (docs/mcp-server.md §6.6)")
}

/// Load the index for querying. `None` when absent or unreadable (wrong
/// schema, truncated vectors, dim mismatch) — callers degrade to keyword
/// search, never error.
pub fn load_index(_well: &str) -> Option<VectorIndex> {
    todo!("P2 wave 1: the vector store (docs/mcp-server.md §6.4)")
}

/// The manifest's summary + a cheap staleness sweep (stat every indexed and
/// indexable file; no hashing, no reads). `None` when no index exists.
pub fn index_status(_well: &str) -> Option<IndexStatus> {
    todo!("P2 wave 1: the vector store (docs/mcp-server.md §6.6)")
}
