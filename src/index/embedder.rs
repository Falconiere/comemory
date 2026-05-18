//! Local ONNX embedders via fastembed. `nomic_text` covers prose memory bodies;
//! `jina_code` is reserved for the code-layer indexer in later tasks. The
//! embedder owns the model session and is `!Sync` because `embed` mutates the
//! internal arena.

use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};

use crate::prelude::*;

/// Thin wrapper around `fastembed::TextEmbedding`. Holds the loaded ONNX
/// session plus the model's output dimensionality so callers can pre-size
/// arrow `FixedSizeList` columns without re-querying the model.
pub struct Embedder {
    inner: TextEmbedding,
    pub dim: usize,
}

impl Embedder {
    /// Build a nomic-embed-text-v1.5 (quantized) embedder. 768-dim output, tuned
    /// for prose; used by the memory-layer indexer.
    pub fn nomic_text() -> Result<Self> {
        let inner = TextEmbedding::try_new(InitOptions::new(EmbeddingModel::NomicEmbedTextV15Q))
            .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { inner, dim: 768 })
    }

    /// Build a jina-embeddings-v2-base-code embedder. 768-dim output, tuned for
    /// code; reserved for the code-layer indexer in later tasks.
    pub fn jina_code() -> Result<Self> {
        let inner =
            TextEmbedding::try_new(InitOptions::new(EmbeddingModel::JinaEmbeddingsV2BaseCode))
                .map_err(|e| Error::Other(e.to_string()))?;
        Ok(Self { inner, dim: 768 })
    }

    /// Embed a single text input. Convenience wrapper around `embed_many` for
    /// the common single-document case.
    pub fn embed_one(&mut self, text: &str) -> Result<Vec<f32>> {
        let mut out = self
            .inner
            .embed(vec![text.to_string()], None)
            .map_err(|e| Error::Other(e.to_string()))?;
        if out.is_empty() {
            return Err(Error::Other("embedder returned no vectors".into()));
        }
        Ok(out.remove(0))
    }

    /// Batch-embed many texts. Order of outputs matches the input iterator.
    pub fn embed_many<I, S>(&mut self, texts: I) -> Result<Vec<Vec<f32>>>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let owned: Vec<String> = texts.into_iter().map(Into::into).collect();
        self.inner
            .embed(owned, None)
            .map_err(|e| Error::Other(e.to_string()))
    }
}
