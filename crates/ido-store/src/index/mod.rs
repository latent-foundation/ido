//! The semantic index (docs/mcp-server.md §6): chunking, local embedding,
//! brute-force vector search, and hybrid fusion over a well's markdown.
//!
//! The pipeline: markdown → [`chunk`] (heading-scoped sections) → [`embed`]
//! (an [`embed::Embedder`] behind a trait — candle in production, a
//! deterministic fake in tests) → [`store`] (`.ido/index/`: `manifest.json` +
//! `vectors.bin` + `chunks.jsonl`, cosine top-k) → [`fusion`] (reciprocal rank
//! fusion with the lexical scan in [`crate::search`]).
//!
//! Every stage is local and optional-degradable: a missing index or model
//! means keyword-only search, reported honestly via [`index_status`] /
//! `well_info` — never an error. Only the embedder itself (the ~80-crate
//! candle tree) sits behind the `semantic` cargo feature; chunking, storage
//! and fusion compile and test everywhere so CI proves the plumbing with
//! [`embed::FakeEmbedder`].
//!
//! Layout:
//! - [`chunk`] — markdown → heading-scoped [`chunk::Chunk`]s (pure, tested)
//! - [`embed`] — the [`embed::Embedder`] trait, the [`embed::ModelSpec`]
//!   registry (pooling + asymmetric prefixes are per-model and load-bearing),
//!   and the test fake
//! - [`fusion`] — reciprocal rank fusion, `k = 60`, never tuned
//! - [`store`] — the on-disk index + incremental (manifest-driven) rebuild +
//!   cosine top-k
//! - [`hybrid`] — the three retrieval modes over one [`crate::model::SearchHit`]
//!   shape, plus the degradation contract every caller reports
//! - [`eval`] — the query set, recall@5 / MRR, and §6.7's gate
//! - [`candle`] *(feature `semantic`)* — the real embedder
//! - [`download`] *(feature `semantic`)* — the one-time model fetch + cache

pub mod chunk;
pub mod embed;
pub mod eval;
pub mod fusion;
pub mod hybrid;
pub mod store;

#[cfg(feature = "semantic")]
pub mod candle;
#[cfg(feature = "semantic")]
pub mod download;

pub use store::index_status;

/// A snapshot of the on-disk index, for `well_info` and (later, P3) the
/// settings pane. `None` from [`index_status`] means "no index yet" — keyword
/// search only.
#[derive(Debug, Clone, serde::Serialize)]
pub struct IndexStatus {
    /// The model id that built the index (`ModelSpec::repo`, or the fake's id
    /// in tests). A different live model triggers a full rebuild rather than
    /// silently mixing vector spaces.
    pub model: String,
    /// Vector width.
    pub dim: usize,
    /// Total chunks indexed.
    pub chunks: usize,
    /// Markdown files covered by the manifest.
    pub files: usize,
    /// When the index was last (re)built, epoch milliseconds.
    pub built_ms: u64,
    /// True when at least one well file changed since `built_ms` — the index
    /// still answers, but a sweep is due.
    pub stale: bool,
}
