use anyhow::{Context, Result};
use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tokio::task;

#[derive(Clone)]
pub struct MemoryEmbedder {
    pool: Arc<EmbedderPool>,
}

struct EmbedderPool {
    models: Vec<Arc<Mutex<TextEmbedding>>>,
    next_model: AtomicUsize,
}

impl MemoryEmbedder {
    pub fn new() -> Result<Self> {
        Self::new_with_pool_size(1)
    }

    pub fn new_with_pool_size(pool_size: usize) -> Result<Self> {
        if pool_size == 0 {
            return Err(anyhow::anyhow!("embedder pool size must be >= 1"));
        }

        let mut models = Vec::with_capacity(pool_size);
        for idx in 0..pool_size {
            let model = build_model(idx == 0)
                .with_context(|| format!("failed to initialize embedder model {}", idx + 1))?;
            models.push(Arc::new(Mutex::new(model)));
        }

        Ok(Self {
            pool: Arc::new(EmbedderPool {
                models,
                next_model: AtomicUsize::new(0),
            }),
        })
    }

    pub fn pool_size(&self) -> usize {
        self.pool.models.len()
    }

    fn pick_model(&self) -> Arc<Mutex<TextEmbedding>> {
        let len = self.pool.models.len();
        let idx = self.pool.next_model.fetch_add(1, Ordering::Relaxed) % len;
        Arc::clone(&self.pool.models[idx])
    }

    pub async fn embed_single(&self, text: String) -> Result<Vec<f32>> {
        let model_arc = self.pick_model();

        let embedding = task::spawn_blocking(move || {
            let mut model = model_arc
                .lock()
                .map_err(|_| anyhow::anyhow!("embedding model mutex poisoned"))?;
            let mut embeddings = model.embed(vec![text], None)
                .context("Failed to embed text")?;
            
            embeddings.pop().context("No embedding returned")
        })
        .await
        .context("Tokio join error during embedding")??;

        Ok(embedding)
    }

    pub async fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>> {
        let model_arc = self.pick_model();

        let embeddings = task::spawn_blocking(move || {
            let mut model = model_arc
                .lock()
                .map_err(|_| anyhow::anyhow!("embedding model mutex poisoned"))?;
            model.embed(texts, None)
                .context("Failed to embed batch of texts")
        })
        .await
        .context("Tokio join error during embedding")??;

        Ok(embeddings)
    }
}

fn build_model(show_download_progress: bool) -> Result<TextEmbedding> {
    let init = InitOptions::new(EmbeddingModel::AllMiniLML6V2)
        .with_show_download_progress(show_download_progress);
    TextEmbedding::try_new(init)
        .or_else(|_| TextEmbedding::try_new(InitOptions::new(EmbeddingModel::AllMiniLML6V2)))
        .context("Failed to initialize fastembed model")
}
