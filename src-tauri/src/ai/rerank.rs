use fastembed::{RerankInitOptions, RerankerModel, TextRerank};
use tokio::sync::Mutex as AsyncMutex;

pub struct Reranker {
    model: AsyncMutex<TextRerank>,
}

impl Reranker {
    pub fn new() -> Result<Self, String> {
        let model = TextRerank::try_new(
            RerankInitOptions::new(RerankerModel::JINARerankerV1TurboEn)
                .with_show_download_progress(false),
        )
        .map_err(|e| format!("failed to init reranker model: {e}"))?;
        log::info!("Reranker initialized with JINARerankerV1TurboEn");
        Ok(Self {
            model: AsyncMutex::new(model),
        })
    }

    /// Reranks `candidates` against `query`. Returns (original_index, score)
    /// pairs sorted by rerank score descending.
    pub async fn rerank(
        &self,
        query: &str,
        candidates: &[String],
    ) -> Result<Vec<(usize, f32)>, String> {
        if candidates.is_empty() {
            return Ok(vec![]);
        }
        let owned_query = query.to_string();
        let owned_candidates = candidates.to_vec();
        let guard = self.model.lock().await;
        tokio::task::block_in_place(|| {
            let results = guard
                .rerank(owned_query, owned_candidates, false, None)
                .map_err(|e| format!("reranking failed: {e}"))?;
            log::debug!(
                "Reranker: query='{query}', {} candidates → first score={}",
                results.len(),
                results.first().map(|r| r.score).unwrap_or(0.0)
            );
            Ok(results.into_iter().map(|r| (r.index, r.score)).collect())
        })
    }
}

#[cfg(feature = "integration-tests")]
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reranker_creation() {
        let r = Reranker::new();
        assert!(r.is_ok(), "reranker should initialize: {:?}", r.err());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_rerank_orders_relevant_document_first() {
        let r = Reranker::new().unwrap();
        let candidates = vec![
            "products.sku TEXT stock keeping unit".to_string(),
            "invoices.status TEXT values pending paid overdue".to_string(),
        ];
        let ranked = r.rerank("unpaid invoices", &candidates).await.unwrap();
        assert_eq!(ranked.len(), 2);
        assert_eq!(
            ranked[0].0, 1,
            "the invoices.status candidate should rank first for this query"
        );
        assert!(
            ranked[0].1 > ranked[1].1,
            "scores must be sorted descending"
        );
    }
}
