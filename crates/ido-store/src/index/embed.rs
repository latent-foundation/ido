//! The embedding seam: the [`Embedder`] trait, the per-model [`ModelSpec`]
//! registry, and a deterministic test fake.
//!
//! The trait is the design decision §6.3 actually forces: candle, tract and
//! model2vec then differ in one file, the eval harness can score them on the
//! same well, and the manifest's model id already forces a clean reindex on a
//! switch. Everything model-specific — pooling, the asymmetric
//! query/document prefixes — lives in the [`ModelSpec`] both the index build
//! path and the query path read, because the two traps in this file are
//! silent: mean-pooling a CLS model or dropping a prefix produces a working,
//! normalized, entirely-mediocre index and no error.

use std::hash::{DefaultHasher, Hash, Hasher};

/// Which side of retrieval a text is on. Asymmetric models (BGE, E5) prefix
/// the two sides differently, and applying the query prefix to documents
/// costs real recall — so every [`Embedder::embed`] call declares its side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// A search query — gets `ModelSpec::query_prefix`.
    Query,
    /// A corpus chunk being indexed — gets `ModelSpec::doc_prefix`.
    Document,
}

/// Anything that can turn texts into unit vectors. `texts` arrive already
/// composed ([`super::chunk::Chunk::embedding_text`]) but *not* prefixed —
/// role prefixes are the embedder's job, per its spec.
pub trait Embedder {
    /// Stable id recorded in the index manifest (`ModelSpec::repo` for real
    /// models). A mismatch with an existing index forces a full rebuild.
    fn id(&self) -> &str;
    /// Vector width.
    fn dim(&self) -> usize;
    /// Embed a batch. Returns one `dim()`-wide L2-normalized vector per text,
    /// in order.
    fn embed(&self, texts: &[String], role: Role) -> Result<Vec<Vec<f32>>, String>;

    /// Embed, reporting progress as batches finish. `on_batch(n)` receives
    /// the number of texts newly completed and may be called from several
    /// threads at once (candle's rayon-parallel batches — see
    /// `candle::CandleEmbedder`'s override). Default: one call with
    /// everything, at the end — correct for any embedder that doesn't batch
    /// internally, just not informative mid-flight.
    fn embed_with_progress(
        &self,
        texts: &[String],
        role: Role,
        on_batch: &(dyn Fn(usize) + Sync),
    ) -> Result<Vec<Vec<f32>>, String> {
        let out = self.embed(texts, role)?;
        on_batch(texts.len());
        Ok(out)
    }
}

/// Model architecture, naming the `candle-transformers` implementation that
/// loads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Arch {
    /// `candle_transformers::models::bert` — BGE, MiniLM.
    Bert,
}

/// How token embeddings collapse to one vector — **per-model, never a global
/// choice** (§6.3): BGE pools the CLS token; MiniLM/E5/Nomic take the
/// attention-masked mean (divide by the mask sum, never the sequence length).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pooling {
    /// The first (`[CLS]`) token's hidden state.
    Cls,
    /// Attention-masked mean over the sequence.
    Mean,
}

/// Everything the pipeline must know about one model, encoded as data so the
/// build path and the query path cannot disagree (§6.3 "the model registry").
pub struct ModelSpec {
    /// HuggingFace repo — also the model id in the index manifest.
    pub repo: &'static str,
    /// Which architecture loads it.
    pub arch: Arch,
    /// Vector width.
    pub dim: usize,
    /// Tokenizer truncation length.
    pub max_tokens: usize,
    /// See [`Pooling`] — wrong pooling is a silent quality bug.
    pub pooling: Pooling,
    /// Prepended to [`Role::Query`] texts. Load-bearing for BGE/E5.
    pub query_prefix: &'static str,
    /// Prepended to [`Role::Document`] texts (empty for BGE).
    pub doc_prefix: &'static str,
    /// L2-normalize the pooled vector (true for every shipped candidate).
    pub normalize: bool,
    /// The pinned HuggingFace revision — a commit sha, never `main`. §6.3's
    /// "pin the model revision": `resolve/{revision}/` takes one, so an
    /// upstream re-upload can neither change the bytes we fetch nor silently
    /// invalidate the [`ModelSpec::files`] hashes below.
    pub revision: &'static str,
    /// The files to fetch from the repo, each pinned to its lowercase-hex
    /// sha256 (`(name, sha256)`). Names are plain — no path separators — so
    /// one flat directory holds one model. The download verifies these before
    /// a file is moved into place ([`super::download::ensure_model`]).
    pub files: &'static [(&'static str, &'static str)],
    /// Total bytes of `files`, for a "downloading 133 MB" UI. Informational
    /// only — integrity is the per-file sha256's job, not this number's.
    pub download_bytes: u64,
}

/// The shipped default (§6.3): best retrieval quality per byte, ~133 MB,
/// 384-dim, English-first.
pub const BGE_SMALL_EN_V15: ModelSpec = ModelSpec {
    repo: "BAAI/bge-small-en-v1.5",
    arch: Arch::Bert,
    dim: 384,
    max_tokens: 512,
    pooling: Pooling::Cls,
    query_prefix: "Represent this sentence for searching relevant passages: ",
    doc_prefix: "",
    normalize: true,
    // The repo's `main` as of 2026-08-21 — pinned by sha so it stays that.
    revision: "5c38ec7c405ec4b44b94cc5a9bb96e735b38267a",
    files: &[
        (
            "config.json",
            "094f8e891b932f2000c92cfc663bac4c62069f5d8af5b5278c4306aef3084750",
        ),
        (
            "tokenizer.json",
            "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
        ),
        (
            "model.safetensors",
            "3c9f31665447c8911517620762200d2245a2518d6e7208acc78cd9db317e21ad",
        ),
    ],
    // Measured, not estimated: the sum of the three files actually on disk
    // at this revision (config.json 743 B + tokenizer.json 711,396 B +
    // model.safetensors 133,466,304 B).
    download_bytes: 134_178_443,
};

/// The model the index builds with unless configured otherwise.
pub const DEFAULT_MODEL: &ModelSpec = &BGE_SMALL_EN_V15;

/// A deterministic, dependency-free embedder for tests and CI: hashed
/// bag-of-words, L2-normalized. Real overlap semantics — identical texts are
/// cosine 1, texts sharing words score above unrelated ones — so the storage,
/// staleness and fusion plumbing is provable without the candle tree or a
/// model download (§6.3's de-risking play, kept permanently as the test
/// double). Never ships: its id can't collide with a real repo.
pub struct FakeEmbedder {
    /// Vector width; small is fine (collisions only soften scores).
    pub dim: usize,
}

impl Default for FakeEmbedder {
    fn default() -> Self {
        Self { dim: 64 }
    }
}

impl Embedder for FakeEmbedder {
    fn id(&self) -> &str {
        "fake-bow-v1"
    }

    fn dim(&self) -> usize {
        self.dim
    }

    fn embed(&self, texts: &[String], _role: Role) -> Result<Vec<Vec<f32>>, String> {
        Ok(texts
            .iter()
            .map(|text| {
                let mut v = vec![0f32; self.dim];
                let tokens = text
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|t| !t.is_empty())
                    .map(str::to_owned)
                    .collect::<Vec<_>>();
                for token in tokens {
                    let mut h = DefaultHasher::new();
                    token.hash(&mut h);
                    v[(h.finish() % self.dim as u64) as usize] += 1.0;
                }
                let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
                if norm > 0.0 {
                    for x in &mut v {
                        *x /= norm;
                    }
                }
                v
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }

    #[test]
    fn fake_embedder_is_deterministic_and_normalized() {
        let e = FakeEmbedder::default();
        let texts = vec!["session keys live in the OS keychain".to_string()];
        let a = e.embed(&texts, Role::Document).unwrap();
        let b = e.embed(&texts, Role::Query).unwrap();
        assert_eq!(a, b, "role never changes the fake's output");
        let norm: f32 = a[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-6, "unit norm, got {norm}");
    }

    #[test]
    fn shared_words_score_above_unrelated_text() {
        let e = FakeEmbedder::default();
        let vs = e
            .embed(
                &[
                    "auth tokens stored in the keychain".to_string(),
                    "keychain storage for auth".to_string(),
                    "the kanban board drag indicator".to_string(),
                ],
                Role::Document,
            )
            .unwrap();
        assert!(
            cosine(&vs[0], &vs[1]) > cosine(&vs[0], &vs[2]),
            "overlapping vocabulary must rank closer"
        );
    }

    #[test]
    fn embed_with_progress_default_reports_the_full_count_once() {
        // No embedder here overrides `embed_with_progress`, so this exercises
        // the trait's default body: one call, at the end, with everything.
        let e = FakeEmbedder::default();
        let texts = vec!["a".to_string(), "b".to_string(), "c".to_string()];
        let calls: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());
        let on_batch = |n: usize| calls.lock().unwrap().push(n);
        let out = e
            .embed_with_progress(&texts, Role::Document, &on_batch)
            .unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(*calls.lock().unwrap(), vec![3]);
    }

    #[test]
    fn default_model_spec_carries_the_bge_traps() {
        // The two silent-quality-bug fields (§6.3) — pin them so a refactor
        // can't quietly flip one.
        assert_eq!(DEFAULT_MODEL.pooling, Pooling::Cls, "BGE pools CLS");
        assert!(
            DEFAULT_MODEL
                .query_prefix
                .starts_with("Represent this sentence"),
            "BGE's asymmetric query prefix is load-bearing"
        );
        assert!(DEFAULT_MODEL.doc_prefix.is_empty());
        assert!(DEFAULT_MODEL.normalize);
    }
}
