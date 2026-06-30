//! In-memory search results cache backed by `moka`.
//!
//! A `moka::future::Cache<String, Arc<Vec<SearchResult>>>` keyed by the
//! normalized query string (trimmed, lower-cased). Each entry has a 60-second
//! TTL and the cache holds at most 1000 entries.
//!
//! On every `PostgreSQL` `NOTIFY` (row insert/update/delete) the entire cache
//! is invalidated so the next search re-fetches from the database. This is
//! conservative but simple — we do not know which queries a new row would
//! match, and the cache is small enough that a full invalidation is cheap.
//!
//! # Initialisation
//! Call [`init_cache`] during server startup before the first search request.
//! The cache lives in a `OnceLock` and is safe to call from any task.
//!
//! # dev-tools feature note
//! The `dev-tools` feature (behind `RUSTFLAGS="--cfg tokio_unstable"`) does
//! not affect this module. It is purely a `moka`-backed cache.

use std::sync::Arc;
use std::sync::OnceLock;
use std::time::Duration;

use moka::future::Cache;

use crate::db::SearchResult;

/// Shared cache instance, initialised once at startup.
static SEARCH_CACHE: OnceLock<Cache<String, Arc<Vec<SearchResult>>>> = OnceLock::new();

/// Initialise the search results cache.
///
/// Must be called once during server startup before accepting search requests.
/// Subsequent calls are a no-op (the `OnceLock` retains the first value).
pub fn init_cache() {
    SEARCH_CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .max_capacity(1000)
            .build()
    });
}

/// Retrieve cached results for a query, if present.
///
/// The query is used as-is (the caller should normalise before calling).
#[must_use]
pub async fn get(query: &str) -> Option<Arc<Vec<SearchResult>>> {
    let cache = SEARCH_CACHE.get()?;
    cache.get(query).await
}

/// Insert results into the cache for a given query.
///
/// This is a no-op if the cache has not been initialised (defensive).
pub async fn insert(query: String, results: Arc<Vec<SearchResult>>) {
    if let Some(cache) = SEARCH_CACHE.get() {
        cache.insert(query, results).await;
    }
}

/// Invalidate every cached entry.
///
/// Called on every `NOTIFY` from the `search_results` channel so that
/// subsequent searches reflect the updated data.
pub fn invalidate_all() {
    if let Some(cache) = SEARCH_CACHE.get() {
        cache.invalidate_all();
        tracing::debug!("search cache invalidated via NOTIFY");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    fn sample_results() -> Vec<SearchResult> {
        vec![
            SearchResult {
                id: Uuid::new_v4(),
                title: "Result 1".into(),
                url: "https://example.com/1".into(),
                snippet: "snippet 1".into(),
                created_at: Utc::now(),
            },
            SearchResult {
                id: Uuid::new_v4(),
                title: "Result 2".into(),
                url: "https://example.com/2".into(),
                snippet: "snippet 2".into(),
                created_at: Utc::now(),
            },
        ]
    }

    /// Prove that the cache preserves Arc identity (no inner-Vec clone).
    ///
    /// Creates a local `moka` cache matching the production builder and
    /// verifies that the `Arc` retrieved is the same allocation as the one
    /// inserted. This is the property the C2 fix relies on.
    #[tokio::test]
    async fn cache_hit_returns_same_arc_instance() {
        let cache: Cache<String, Arc<Vec<SearchResult>>> = Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .max_capacity(1000)
            .build();

        let key = "c2-arc-identity-key".to_string();
        let results = sample_results();
        let arc: Arc<Vec<SearchResult>> = Arc::new(results);

        cache.insert(key.clone(), arc.clone()).await;
        let retrieved = cache.get(&key).await;
        assert!(retrieved.is_some(), "cache hit should return Some");

        let retrieved = retrieved.unwrap();
        // The retrieved Arc must point to the SAME allocation as the one we
        // inserted. If the cache cloned the inner Vec, the pointer would differ.
        assert!(
            Arc::ptr_eq(&retrieved, &arc),
            "cache should preserve Arc identity (no inner-Vec clone)"
        );
        // After we drop our local handle, the cache should still hold one
        // reference alongside the original `arc` — strong count drops to 2.
        drop(retrieved);
        assert_eq!(
            Arc::strong_count(&arc),
            2,
            "cache holds one reference; arc holds one; no stray clones"
        );
    }

    /// The global cache wrapper returns `None` for a nonexistent key.
    #[tokio::test]
    async fn cache_miss_returns_none() {
        init_cache();
        assert!(get("test-miss-key-nonexistent").await.is_none());
    }

    /// `invalidate_all` removes every entry from the global cache.
    #[tokio::test]
    async fn cache_invalidate_all_clears_entries() {
        init_cache();
        let arc = Arc::new(sample_results());
        insert("test-inv-key1".to_string(), arc.clone()).await;
        insert("test-inv-key2".to_string(), arc.clone()).await;
        assert!(get("test-inv-key1").await.is_some());
        invalidate_all();
        assert!(get("test-inv-key1").await.is_none());
        assert!(get("test-inv-key2").await.is_none());
    }
}
