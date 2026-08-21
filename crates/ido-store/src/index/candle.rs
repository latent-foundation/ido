//! The real embedder: candle running a BERT-family model on CPU
//! (docs/mcp-server.md §6.3 "the embedding path, concretely").
//!
//! Loaded once and kept resident — the per-call overhead dominates otherwise.
//! Everything model-specific (pooling, prefixes, truncation length) comes
//! from the [`ModelSpec`]; this file only executes it. The two silent quality
//! bugs to guard: CLS-vs-mean pooling is per-model, and masked mean divides
//! by the **mask sum**, never the sequence length.
//!
//! Correctness here is not self-evident — candle reimplements architectures
//! rather than binding PyTorch, so wrong-but-plausible vectors are the failure
//! mode to fear. `tests/known_vector.rs` is the gate (§10 open question 1):
//! it diffs this file's output against `sentence-transformers`' for a fixed
//! set of strings. The pure helpers below are unit-tested here without a model.

use std::fs;
use std::path::Path;

use candle_core::{Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config, DTYPE};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use super::embed::{Arch, Embedder, ModelSpec, Pooling, Role};

/// Texts per forward pass. §6.3 says 16–64; 32 keeps the padded batch's
/// widest chunk from wasting much compute while still amortizing the
/// per-call overhead that dominates a naive one-at-a-time loop.
const MAX_BATCH: usize = 32;

/// A resident model + tokenizer pair implementing [`Embedder`].
pub struct CandleEmbedder {
    /// The registry entry this was loaded from — read on every `embed` for
    /// prefixes and pooling, so the build path and the query path cannot
    /// disagree.
    spec: &'static ModelSpec,
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
}

impl CandleEmbedder {
    /// Load `spec` from `model_dir` (which holds `config.json`,
    /// `tokenizer.json`, `model.safetensors` — see [`super::download`]).
    /// CPU-only, f32 (§6.3: no useful f16 story on candle's CPU path).
    pub fn load(spec: &'static ModelSpec, model_dir: &Path) -> Result<Self, String> {
        // Exhaustive on purpose: adding an `Arch` variant should fail to
        // compile here rather than silently load it as a BERT.
        match spec.arch {
            Arch::Bert => {}
        }

        let cfg_path = model_dir.join("config.json");
        let raw = fs::read_to_string(&cfg_path)
            .map_err(|e| format!("reading {}: {e}", cfg_path.display()))?;
        let cfg: Config = serde_json::from_str(&raw)
            .map_err(|e| format!("parsing {}: {e}", cfg_path.display()))?;
        if cfg.hidden_size != spec.dim {
            return Err(format!(
                "{}: config.json says {} dims, the registry says {} — the index would be \
                 the wrong width",
                spec.repo, cfg.hidden_size, spec.dim
            ));
        }

        let tok_path = model_dir.join("tokenizer.json");
        let mut tokenizer = Tokenizer::from_file(&tok_path)
            .map_err(|e| format!("reading {}: {e}", tok_path.display()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: spec.max_tokens,
                ..Default::default()
            }))
            .map_err(|e| format!("configuring truncation: {e}"))?;
        // Pad to the batch's longest sequence, not to `max_tokens` — padding
        // is wasted compute, and with a correct mask it changes no vector.
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            ..Default::default()
        }));

        let device = Device::Cpu;
        let weights = model_dir.join("model.safetensors");
        // Safety: mmap of a file we downloaded and sha256-verified ourselves.
        // Candle offers no safe loader for safetensors; the risk it names is
        // the file mutating underneath us, which nothing here does.
        let vb = unsafe { VarBuilder::from_mmaped_safetensors(&[&weights], DTYPE, &device) }
            .map_err(|e| format!("loading {}: {e}", weights.display()))?;
        let model =
            BertModel::load(vb, &cfg).map_err(|e| format!("building {}: {e}", spec.repo))?;

        Ok(Self {
            spec,
            model,
            tokenizer,
            device,
        })
    }

    /// One forward pass over an already-prefixed batch (`texts.len()` at most
    /// [`MAX_BATCH`]). Returns one pooled — and, per the spec, normalized —
    /// vector per text, in order.
    fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        let encodings = self
            .tokenizer
            .encode_batch(texts.to_vec(), true)
            .map_err(|e| format!("tokenizing: {e}"))?;
        let batch = encodings.len();
        let Some(seq) = encodings.first().map(|e| e.get_ids().len()) else {
            return Ok(Vec::new());
        };

        let mut ids = Vec::with_capacity(batch * seq);
        let mut mask = Vec::with_capacity(batch * seq);
        for enc in &encodings {
            if enc.get_ids().len() != seq {
                // BatchLongest padding guarantees this; if it ever stops
                // holding, the tensor reshape below would silently misalign
                // rows rather than fail.
                return Err("tokenizer returned a ragged batch".to_string());
            }
            ids.extend_from_slice(enc.get_ids());
            mask.extend_from_slice(enc.get_attention_mask());
        }

        let token_ids = Tensor::from_vec(ids, (batch, seq), &self.device).map_err(tensor_err)?;
        let attention = Tensor::from_vec(mask, (batch, seq), &self.device).map_err(tensor_err)?;
        // Single-sequence inputs: every token is segment 0.
        let token_types = token_ids.zeros_like().map_err(tensor_err)?;

        let hidden = self
            .model
            .forward(&token_ids, &token_types, Some(&attention))
            .map_err(tensor_err)?;

        let pooled = match self.spec.pooling {
            Pooling::Cls => hidden.i((.., 0)).map_err(tensor_err)?,
            Pooling::Mean => masked_mean(&hidden, &attention).map_err(tensor_err)?,
        };
        let pooled = if self.spec.normalize {
            normalize_l2(&pooled).map_err(tensor_err)?
        } else {
            pooled
        };
        pooled.to_vec2::<f32>().map_err(tensor_err)
    }
}

impl Embedder for CandleEmbedder {
    fn id(&self) -> &str {
        self.spec.repo
    }

    fn dim(&self) -> usize {
        self.spec.dim
    }

    fn embed(&self, texts: &[String], role: Role) -> Result<Vec<Vec<f32>>, String> {
        let prefix = role_prefix(self.spec, role);
        let mut out = Vec::with_capacity(texts.len());
        for batch in texts.chunks(MAX_BATCH) {
            let prefixed: Vec<String> = batch.iter().map(|t| with_prefix(prefix, t)).collect();
            out.extend(self.embed_batch(&prefixed)?);
        }
        Ok(out)
    }
}

/// The prefix `role` gets under `spec` — §6.3's second silent quality bug:
/// omitting BGE's query prefix costs recall, and applying it to documents
/// costs more.
fn role_prefix(spec: &ModelSpec, role: Role) -> &'static str {
    match role {
        Role::Query => spec.query_prefix,
        Role::Document => spec.doc_prefix,
    }
}

/// Apply a role prefix, allocating nothing extra when there isn't one (BGE's
/// document side is prefix-free, which is most of the corpus).
fn with_prefix(prefix: &str, text: &str) -> String {
    if prefix.is_empty() {
        text.to_string()
    } else {
        format!("{prefix}{text}")
    }
}

/// Attention-masked mean over the sequence: `Σ(h·m) / Σm`. Dividing by the
/// **mask sum** and not the sequence length is the whole point — padding
/// tokens otherwise drag every short chunk toward the same point in the space.
/// Unused by BGE (which pools CLS) and load-bearing for MiniLM/E5/Nomic.
fn masked_mean(hidden: &Tensor, mask: &Tensor) -> candle_core::Result<Tensor> {
    // [batch, seq] -> [batch, seq, 1], in the hidden states' dtype.
    let m = mask.to_dtype(hidden.dtype())?.unsqueeze(2)?;
    let summed = hidden.broadcast_mul(&m)?.sum(1)?; // [batch, dim]
    let counts = m.sum(1)?.clamp(1e-9, f32::INFINITY)?; // [batch, 1]
    summed.broadcast_div(&counts)
}

/// Row-wise L2 normalization, so a dot product is a cosine.
fn normalize_l2(v: &Tensor) -> candle_core::Result<Tensor> {
    v.broadcast_div(&v.sqr()?.sum_keepdim(1)?.sqrt()?)
}

/// Candle's error carries enough context on its own; the store's convention is
/// `Result<_, String>` everywhere, so flatten at the boundary.
fn tensor_err(e: candle_core::Error) -> String {
    format!("embedding: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::embed::DEFAULT_MODEL;

    #[test]
    fn query_texts_get_the_prefix_and_documents_do_not() {
        let q = role_prefix(DEFAULT_MODEL, Role::Query);
        let d = role_prefix(DEFAULT_MODEL, Role::Document);
        assert_eq!(
            with_prefix(q, "where are the keys"),
            "Represent this sentence for searching relevant passages: where are the keys"
        );
        assert_eq!(
            with_prefix(d, "where are the keys"),
            "where are the keys",
            "BGE's document side is prefix-free — prefixing it costs recall"
        );
    }

    #[test]
    fn masked_mean_divides_by_the_mask_sum() {
        // Two real tokens ([1,1] and [3,3]) plus one padding token whose
        // hidden state is deliberately extreme. The mean of the *real* tokens
        // is [2,2]; dividing by the sequence length instead would give
        // [1.33, 1.33], and including the pad would give [34.33, 34.33].
        let hidden =
            Tensor::from_vec(vec![1f32, 1., 3., 3., 99., 99.], (1, 3, 2), &Device::Cpu).unwrap();
        let mask = Tensor::from_vec(vec![1u32, 1, 0], (1, 3), &Device::Cpu).unwrap();
        let out = masked_mean(&hidden, &mask)
            .unwrap()
            .to_vec2::<f32>()
            .unwrap();
        assert_eq!(out.len(), 1);
        for x in &out[0] {
            assert!((x - 2.0).abs() < 1e-6, "expected 2.0, got {x}");
        }
    }

    #[test]
    fn masked_mean_survives_an_all_padding_row() {
        // Degenerate but reachable (an empty chunk): the clamp must keep this
        // from dividing by zero and poisoning the index with NaNs.
        let hidden = Tensor::from_vec(vec![5f32, 7.], (1, 1, 2), &Device::Cpu).unwrap();
        let mask = Tensor::from_vec(vec![0u32], (1, 1), &Device::Cpu).unwrap();
        let out = masked_mean(&hidden, &mask)
            .unwrap()
            .to_vec2::<f32>()
            .unwrap();
        assert!(out[0].iter().all(|x| x.is_finite()), "got {:?}", out[0]);
    }

    #[test]
    fn normalize_l2_makes_every_row_a_unit_vector() {
        let v = Tensor::from_vec(vec![3f32, 4., 0., 0., -5., 12.], (2, 3), &Device::Cpu).unwrap();
        let out = normalize_l2(&v).unwrap().to_vec2::<f32>().unwrap();
        for row in &out {
            let norm: f32 = row.iter().map(|x| x * x).sum::<f32>().sqrt();
            assert!((norm - 1.0).abs() < 1e-6, "unit norm, got {norm}");
        }
        // Row-wise, not whole-tensor: [3,4,0] normalizes to [0.6, 0.8, 0].
        assert!((out[0][0] - 0.6).abs() < 1e-6);
        assert!((out[0][1] - 0.8).abs() < 1e-6);
    }
}
