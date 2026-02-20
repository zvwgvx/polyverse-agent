use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Clone)]
pub struct MemoryEmbedder {
    model: Arc<Mutex<TextEmbedding>>,
}

impl MemoryEmbedder {
    pub fn new() -> Result<Self> {
        // all-MiniLM-L6-v2 produces 384-dimensional vectors
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::AllMiniLML6V2)
                .with_show_download_progress(true)
        )
        .or_else(|_| TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2))) // fallback without with_show_download_progress
        .context("Failed to initialize fastembed model")?;

        Ok(Self {
            model: Arc::new(Mutex::new(model)),
        })
    }

    /// Embeds a single text string into a 384-dimensional Float32 vector.
    pub async fn embed_single(&self, text: String) -> Result<Vec<f32>> {
        let model_arc = Arc::clone(&self.model);

        let embedding = task::spawn_blocking(move || {
            let mut model = model_arc.lock().unwrap();
            let mut embeddings = model.embed(vec![text], None)
                .context("Failed to embed text")?;
            
            embeddings.pop().context("No embedding returned")
        })
        .await
        .context("Tokio join error during embedding")??;

        Ok(embedding)
    }

    /// Embeds multiple text strings.
    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model_arc = Arc::clone(&self.model);

        let embeddings = task::spawn_blocking(move || {
            let mut model = model_arc.lock().unwrap();
            model.embed(texts, None)
                .context("Failed to embed batch of texts")
        })
        .await
        .context("Tokio join error during embedding")??;

        Ok(embeddings)
    }
}
