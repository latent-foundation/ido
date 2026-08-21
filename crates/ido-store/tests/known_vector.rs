//! The gate on the whole semantic index (docs/mcp-server.md §10 open question
//! 1): **does candle reproduce the reference embeddings?**
//!
//! candle reimplements BERT rather than binding PyTorch, so a model that loads
//! and returns 384 normalized floats proves nothing — a config key it doesn't
//! parse shifts behavior with no error, and the resulting index is plausible
//! and quietly worse. So we diff against the model's own reference
//! implementation: `scripts/gen_known_vectors.py` embeds a fixed set of
//! strings with `sentence-transformers` and commits the vectors to
//! `tests/reference/bge-known-vectors.json`; this file asserts candle lands
//! within 1e-4 of them, per component.
//!
//! The fixture also carries the *prefix* check by construction. Python
//! embedded the query case as the literal prefixed string; Rust embeds only
//! the bare query with [`Role::Query`]. They agree only if the embedder read
//! `ModelSpec::query_prefix` — §6.3's second silent quality bug.
//!
//! Both tests are `#[ignore]`d: they need ~133 MB of model weights, which CI
//! has no business fetching. Run the gate explicitly:
//!
//! ```sh
//! cargo test -p ido-store --features semantic -- --ignored
//! ```

#![cfg(feature = "semantic")]

use std::path::PathBuf;

use ido_store::index::candle::CandleEmbedder;
use ido_store::index::download::ensure_model;
use ido_store::index::embed::{DEFAULT_MODEL, Embedder, Role};
use serde::Deserialize;

/// Per-component agreement required against the reference vectors. §6.3's
/// "honest costs" row names 1e-4; f32 CPU kernels reorder accumulations, but
/// on unit-norm 384-dim vectors that noise lands two orders below this.
const TOLERANCE: f32 = 1e-4;

#[derive(Deserialize)]
struct Fixture {
    meta: Meta,
    vectors: Vec<Case>,
}

#[derive(Deserialize)]
struct Meta {
    model: String,
    revision: String,
}

#[derive(Deserialize)]
struct Case {
    /// What *this* side embeds — the bare text, prefix-free.
    text: String,
    /// `"query"` or `"document"`.
    role: String,
    /// What `sentence-transformers` produced for `python_input`.
    vector: Vec<f32>,
}

impl Case {
    fn role(&self) -> Role {
        match self.role.as_str() {
            "query" => Role::Query,
            "document" => Role::Document,
            other => panic!("unknown role `{other}` in the fixture"),
        }
    }
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("reference")
        .join("bge-known-vectors.json");
    let raw = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
    serde_json::from_str(&raw).expect("the fixture is valid JSON")
}

/// Download (once) and load the default model. Uses the real per-machine cache
/// — later phases reuse the same weights, and re-fetching 133 MB per test run
/// would make the gate too expensive to run.
fn embedder() -> CandleEmbedder {
    let dir = ensure_model(DEFAULT_MODEL).expect("model download");
    CandleEmbedder::load(DEFAULT_MODEL, &dir).expect("model load")
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

#[test]
#[ignore = "downloads ~133 MB of model weights"]
fn candle_reproduces_the_sentence_transformers_vectors() {
    let fixture = fixture();
    assert_eq!(
        fixture.meta.model, DEFAULT_MODEL.repo,
        "the fixture was generated for a different model"
    );
    assert_eq!(
        fixture.meta.revision, DEFAULT_MODEL.revision,
        "the fixture was generated for a different revision — regenerate it \
         with scripts/gen_known_vectors.py rather than loosening the tolerance"
    );

    let embedder = embedder();
    let mut worst = 0f32;
    let mut worst_case = String::new();

    for case in &fixture.vectors {
        assert_eq!(case.vector.len(), DEFAULT_MODEL.dim);
        let got = embedder
            .embed(std::slice::from_ref(&case.text), case.role())
            .expect("embed");
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].len(), DEFAULT_MODEL.dim, "vector width");

        let diff = got[0]
            .iter()
            .zip(&case.vector)
            .map(|(a, b)| (a - b).abs())
            .fold(0f32, f32::max);
        if diff > worst {
            worst = diff;
            worst_case = case.text.clone();
        }

        let norm: f32 = got[0].iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-5,
            "`{}` is not unit-norm ({norm}) — normalize is on in the spec",
            case.text
        );
        assert!(
            diff < TOLERANCE,
            "`{}` ({:?}) differs from the reference by {diff} (> {TOLERANCE}). \
             Suspect pooling (BGE is CLS, not mean) or the role prefix.",
            case.text,
            case.role()
        );
    }

    // Printed so a run of the gate reports the margin, not just a pass.
    println!("max abs diff {worst:e} (worst case: `{worst_case}`)");
}

#[test]
#[ignore = "downloads ~133 MB of model weights"]
fn a_paraphrase_outranks_an_unrelated_note() {
    // The vectors matching the reference proves the forward pass; this proves
    // the thing the feature is *for* — retrieval on meaning, with no shared
    // vocabulary between the query and the passage it should find.
    let embedder = embedder();
    let query = embedder
        .embed(&["where do we store credentials".to_string()], Role::Query)
        .expect("embed query");
    let docs = embedder
        .embed(
            &[
                "session keys are kept in the OS keychain".to_string(),
                "the kanban board's drag indicator uses card midpoints".to_string(),
            ],
            Role::Document,
        )
        .expect("embed documents");

    let hit = cosine(&query[0], &docs[0]);
    let miss = cosine(&query[0], &docs[1]);
    println!("paraphrase {hit:.4} vs unrelated {miss:.4}");
    assert!(
        hit > miss,
        "semantic retrieval failed: paraphrase scored {hit}, unrelated {miss}"
    );
}
