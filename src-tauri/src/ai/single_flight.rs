//! Single-flight dedup for ONNX embedding calls: concurrent callers with the
//! same doc text share one in-flight future. Uses futures::future::shared —
//! tokio::broadcast is the wrong primitive here (a receiver subscribed after
//! the send never receives the value).

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use futures::future::FutureExt;
use tokio::sync::Mutex;

use crate::ai::cache_store::PersistentVectorCache;
use crate::ai::embed::Embedder;

/// Async embedding capability. Implemented by the real ONNX `Embedder`
/// (blocking inference via `block_in_place`) and by counting mocks in tests.
/// The boxed-future signature is the interface contract (same shape as the
/// plan's `Embed` trait); the complexity is inherent to async trait methods
/// without async-trait's macro.
#[allow(clippy::type_complexity)]
pub trait Embed: Send + Sync {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, String>> + Send + 'a>>;
}

impl Embed for Embedder {
    fn embed<'a>(
        &'a self,
        texts: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, String>> + Send + 'a>> {
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        Box::pin(async move { self.embed(&refs).await })
    }
}

type EmbedFuture = Pin<Box<dyn Future<Output = Result<Vec<f32>, String>> + Send>>;

#[derive(Clone)]
pub struct SingleFlightEmbedder {
    inner: Arc<dyn Embed>,
    in_flight: Arc<Mutex<HashMap<String, futures::future::Shared<EmbedFuture>>>>,
}

impl SingleFlightEmbedder {
    pub fn new(inner: Arc<dyn Embed>) -> Self {
        Self {
            inner,
            in_flight: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Embed one text, sharing the in-flight ONNX call with concurrent
    /// callers of the identical doc text.
    pub async fn embed_one(&self, text: &str) -> Result<Vec<f32>, String> {
        let key = PersistentVectorCache::compute_doc_hash(text);
        let shared: futures::future::Shared<EmbedFuture> = {
            let mut guard = self.in_flight.lock().await;
            if let Some(existing) = guard.get(&key) {
                existing.clone()
            } else {
                let inner = self.inner.clone();
                let text = text.to_string();
                let fut: EmbedFuture = Box::pin(async move {
                    let mut out = inner.embed(&[text]).await?;
                    out.pop()
                        .ok_or_else(|| "empty embedding output".to_string())
                });
                let shared = fut.shared();
                guard.insert(key.clone(), shared.clone());
                shared
            }
        };
        let result = match shared.await {
            Ok(vec) => Ok(vec),
            Err(_canceled) => Err("embedding task was cancelled".to_string()),
        };
        // Remove the entry whether or not the future succeeded; late callers
        // fall back to the disk cache, never to a stale in-flight entry.
        self.in_flight.lock().await.remove(&key);
        result
    }

    /// Embed unique texts (dedup by doc hash), returning one vector per input
    /// text in input order.
    ///
    /// Batched: unique texts are chunked (≤128) and each chunk goes through a
    /// SINGLE `inner.embed(&chunk)` call — ONNX is dramatically faster on one
    /// large batch than on N single-text calls (a 2,000-column cold start is
    /// ~16 batched inferences, not ~2,000 sequential ones). The single-flight
    /// map still serves `embed_one` callers; the batch path bypasses it because
    /// the caller (enrich) already dedups by hash.
    pub async fn embed_missing(&self, texts: &[String]) -> Result<Vec<Vec<f32>>, String> {
        const BATCH_SIZE: usize = 128;

        let mut unique: Vec<&String> = Vec::new();
        let mut seen = std::collections::HashSet::new();
        for t in texts {
            let key = PersistentVectorCache::compute_doc_hash(t);
            if seen.insert(key) {
                unique.push(t);
            }
        }
        let mut by_text: HashMap<String, Vec<f32>> = HashMap::new();
        for chunk in unique.chunks(BATCH_SIZE) {
            let chunk_texts: Vec<String> = chunk.iter().map(|t| t.to_string()).collect();
            let embeddings = self.inner.embed(&chunk_texts).await?;
            for (t, v) in chunk.iter().zip(embeddings.iter()) {
                by_text.insert(t.to_string(), v.clone());
            }
        }
        Ok(texts
            .iter()
            .map(|t| by_text.get(t).cloned().unwrap_or_default())
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    struct CountingEmbed {
        calls: Arc<AtomicUsize>,
        delay: Duration,
    }

    impl Embed for CountingEmbed {
        fn embed<'a>(
            &'a self,
            texts: &'a [String],
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Vec<f32>>, String>> + Send + 'a>> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(self.delay).await;
                Ok(texts
                    .iter()
                    .map(|t| vec![t.len() as f32, 1.0, 0.0])
                    .collect())
            })
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn concurrent_identical_texts_embed_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed {
            calls: calls.clone(),
            delay: Duration::from_millis(50),
        }));
        let text = "public.users.status TEXT".to_string();
        let mut handles = Vec::new();
        for _ in 0..10 {
            let e = embedder.clone();
            let t = text.clone();
            handles.push(tokio::spawn(async move { e.embed_one(&t).await }));
        }
        for h in handles {
            let v = h.await.unwrap().unwrap();
            assert_eq!(v.len(), 3);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "10 concurrent callers, 1 ONNX invocation"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn embed_missing_dedups_preserves_order_and_batches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed {
            calls: calls.clone(),
            delay: Duration::ZERO,
        }));
        let texts = vec!["a".to_string(), "b".to_string(), "a".to_string()];
        let out = embedder.embed_missing(&texts).await.unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0], out[2], "duplicate text embeds identically");
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "two unique texts batch into ONE embed call"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn embed_missing_chunks_oversized_batches() {
        let calls = Arc::new(AtomicUsize::new(0));
        let embedder = SingleFlightEmbedder::new(Arc::new(CountingEmbed {
            calls: calls.clone(),
            delay: Duration::ZERO,
        }));
        // 129 unique texts → two chunks (128 + 1).
        let texts: Vec<String> = (0..129).map(|i| format!("text-{i}")).collect();
        let out = embedder.embed_missing(&texts).await.unwrap();
        assert_eq!(out.len(), 129);
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "129 unique texts chunk into 2 embed calls"
        );
    }
}
