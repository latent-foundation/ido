//! The real embedder: candle running a BERT-family model on CPU
//! (docs/mcp-server.md §6.3 "the embedding path, concretely").
//!
//! Loaded once and kept resident — the per-call overhead dominates otherwise.
//! Everything model-specific (pooling, prefixes, truncation length) comes
//! from the [`ModelSpec`]; this file only executes it. The two silent quality
//! bugs to guard: CLS-vs-mean pooling is per-model, and masked mean divides
//! by the **mask sum**, never the sequence length.

use std::path::Path;

use super::embed::{Embedder, ModelSpec, Role};

/// A resident model + tokenizer pair implementing [`Embedder`].
pub struct CandleEmbedder;

impl CandleEmbedder {
    /// Load `spec` from `model_dir` (which holds `config.json`,
    /// `tokenizer.json`, `model.safetensors` — see [`super::download`]).
    /// CPU-only, f32 (§6.3: no useful f16 story on candle's CPU path).
    pub fn load(_spec: &'static ModelSpec, _model_dir: &Path) -> Result<Self, String> {
        todo!("P2 wave 1: the candle embedder (docs/mcp-server.md §6.3)")
    }
}

impl Embedder for CandleEmbedder {
    fn id(&self) -> &str {
        todo!("P2 wave 1: the candle embedder")
    }

    fn dim(&self) -> usize {
        todo!("P2 wave 1: the candle embedder")
    }

    fn embed(&self, _texts: &[String], _role: Role) -> Result<Vec<Vec<f32>>, String> {
        todo!("P2 wave 1: the candle embedder")
    }
}
