use fastembed::{EmbeddingModel, InitOptions, TextEmbedding};
use tokio::sync::Mutex as AsyncMutex;

pub struct Embedder {
    model: AsyncMutex<TextEmbedding>,
}

impl Embedder {
    pub fn new() -> Result<Self, String> {
        let model = TextEmbedding::try_new(
            InitOptions::new(EmbeddingModel::BGESmallENV15)
                .with_show_download_progress(false)
                .with_max_length(128),
        )
        .map_err(|e| format!("failed to init embedding model: {e}"))?;
        Ok(Self {
            model: AsyncMutex::new(model),
        })
    }

    /// Blocking ONNX inference inside block_in_place.
    /// This is the single blocking boundary for the whole feature.
    pub async fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let owned: Vec<String> = texts.iter().map(|s| s.to_string()).collect();
        let guard = self.model.lock().await;
        tokio::task::block_in_place(|| {
            guard
                .embed(owned, None)
                .map_err(|e| format!("embedding failed: {e}"))
        })
    }

    pub async fn embed_query(&self, text: &str) -> Result<Vec<f32>, String> {
        let mut result = self.embed(&[text]).await?;
        result.pop().ok_or_else(|| "empty embedding result".into())
    }

    pub const fn dimension() -> usize {
        384
    }
}

#[cfg(feature = "integration-tests")]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_embedder_creation() {
        let e = Embedder::new();
        assert!(e.is_ok(), "embedder should initialize: {:?}", e.err());
    }

    #[tokio::test]
    async fn test_embed_single_text() {
        let e = Embedder::new().unwrap();
        let result = e.embed(&["users.status TEXT"]).await;
        assert!(result.is_ok());
        let vecs = result.unwrap();
        assert_eq!(vecs.len(), 1);
        assert_eq!(vecs[0].len(), 384);
    }

    #[tokio::test]
    async fn test_embed_empty_input() {
        let e = Embedder::new().unwrap();
        let result = e.embed(&[]).await.unwrap();
        assert!(result.is_empty());
    }

    #[tokio::test]
    async fn test_embed_query_returns_vector() {
        let e = Embedder::new().unwrap();
        let v = e.embed_query("show me active users").await.unwrap();
        assert_eq!(v.len(), 384);
    }

    #[tokio::test]
    async fn test_similar_queries_closer_than_dissimilar() {
        let e = Embedder::new().unwrap();
        let embs = e
            .embed(&[
                "invoices.status TEXT values pending paid overdue",
                "products.sku TEXT stock keeping unit",
            ])
            .await
            .unwrap();
        let q = e.embed_query("unpaid invoices").await.unwrap();
        let sim_inv = cosine(&q, &embs[0]);
        let sim_prod = cosine(&q, &embs[1]);
        assert!(sim_inv > sim_prod, "inv={sim_inv} prod={sim_prod}");
    }

    fn cosine(a: &[f32], b: &[f32]) -> f32 {
        let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
        let na: f32 = a.iter().map(|x| x * x).sum();
        let nb: f32 = b.iter().map(|x| x * x).sum();
        dot / (na.sqrt() * nb.sqrt() + 1e-8)
    }
}
