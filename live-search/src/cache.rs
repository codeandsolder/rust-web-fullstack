//! In-memory search results cache backed by `moka`, with **version-based**
//! invalidation.
//!
//! A `moka::future::Cache<String, Arc<CachedEntry>>` keyed by the normalized
//! query string (trimmed, lower-cased). Each entry carries the version it was
//! inserted at; `get` ignores entries whose version no longer matches.
//! Each entry has a TTL (default 60 s) and the cache holds at most 1000 entries
//! by default.
//!
//! On every `PostgreSQL` `NOTIFY` (row insert/update/delete) we bump the
//! version by one (a single `fetch_add(1)`). No cache walk is needed.
//! Stale entries linger harmlessly until moka's TTL expires them and the
//! `get` comparison short-circuits the cost. This is cheaper than
//! `cache.invalidate_all()` (which iterates the whole cache) when the cache
//! is warm and write-heavy.
//!
//! # Usage
//!
//! Create a [`CacheHandle`] with [`CacheHandle::new`] (or
//! [`CacheHandle::default`] for 1000-entry / 60‑s TTL) and store it in
//! [`crate::state::AppContext`].  Call [`get`](CacheHandle::get) /
//! [`insert`](CacheHandle::insert) /
//! [`invalidate_all`](CacheHandle::invalidate_all) on the handle.
//!
//! # Test seam
//!
//! Tests create a local `CacheHandle` directly — no global setup required:
//!
//! ```rust,ignore
//! let handle = CacheHandle::new(1000, Duration::from_secs(60));
//! assert!(handle.get("test").await.is_none());
//! ```

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moka::future::Cache;

use rwf_domain::SearchResult;

/// A cached search result bundle plus the version of the cache it was
/// inserted at. Wrap in `Arc` so moka stores a refcount bump per
/// subscriber rather than cloning the inner `Vec` on every `get`.
struct CachedEntry {
    data: Arc<Vec<SearchResult>>,
    version: u64,
}

/// Thread-safe handle to a versioned search result cache.
///
/// Cloning is cheap (the inner moka `Cache` is `Arc`-backed).
#[derive(Clone)]
#[must_use = "a CacheHandle does nothing unless get/insert/invalidate_all is called"]
pub struct CacheHandle {
    cache: Cache<String, Arc<CachedEntry>>,
    version: Arc<AtomicU64>,
}

impl CacheHandle {
    /// Create a new cache with the given capacity and entry TTL.
    pub fn new(max_capacity: u64, ttl: Duration) -> Self {
        Self {
            cache: Cache::builder()
                .time_to_live(ttl)
                .max_capacity(max_capacity)
                .build(),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Retrieve cached results for a query, if present **and** the entry's
    /// version still matches the current cache version.
    ///
    /// A version mismatch returns `None`; the caller refetches and re-inserts.
    /// Stale entries are dropped lazily by moka's TTL once the version bumps
    /// age them out of every subsequent read.
    ///
    /// The query is used as-is (the caller should normalise before calling).
    #[must_use]
    pub async fn get(&self, query: &str) -> Option<Arc<Vec<SearchResult>>> {
        let entry = self.cache.get(query).await?;
        if entry.version != self.version.load(Ordering::Acquire) {
            return None;
        }
        Some(Arc::clone(&entry.data))
    }

    /// Insert results into the cache tagged with the current version.
    pub async fn insert(&self, query: String, results: Arc<Vec<SearchResult>>) {
        let entry = Arc::new(CachedEntry {
            data: results,
            version: self.version.load(Ordering::Acquire),
        });
        self.cache.insert(query, entry).await;
    }

    /// Invalidate every cached entry.
    ///
    /// Called on every `NOTIFY` from the `search_results` channel so that
    /// subsequent searches reflect the updated data. Implemented as a single
    /// `fetch_add` on the version — no walk through the cache is required
    /// because each `get` re-checks the version and skips stale entries
    /// automatically. Stale entries linger until moka's TTL expires them.
    pub fn invalidate_all(&self) {
        let prev = self.version.fetch_add(1, Ordering::AcqRel);
        tracing::debug!(
            prev_version = prev,
            "search cache version bumped via NOTIFY"
        );
    }

    /// Read the current cache version. Exposed for tests and observability.
    #[must_use]
    pub fn current_version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

impl Default for CacheHandle {
    fn default() -> Self {
        Self::new(1000, Duration::from_secs(60))
    }
}

impl std::fmt::Debug for CacheHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CacheHandle")
            .field("version", &self.version.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use anyhow::Context;
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

    /// Prove that the cache preserves Arc identity of the inner results Vec
    /// (no inner-Vec clone) when the entry is fresh.
    #[tokio::test]
    async fn cache_hit_returns_same_arc_instance() -> anyhow::Result<()> {
        let handle = CacheHandle::new(1000, Duration::from_secs(60));

        let key = "c2-arc-identity-key".to_string();
        let results = sample_results();
        let arc: Arc<Vec<SearchResult>> = Arc::new(results);

        handle.insert(key.clone(), arc.clone()).await;
        let retrieved = handle
            .get(&key)
            .await
            .context("cache hit should return Some")?;
        // The retrieved Arc's inner Vec must point to the SAME allocation
        // as the one we inserted.
        assert!(
            Arc::ptr_eq(&retrieved, &arc),
            "cache should preserve Arc identity (no inner-Vec clone)"
        );
        drop(retrieved);
        assert_eq!(
            Arc::strong_count(&arc),
            2,
            "cache holds one reference; arc holds one; no stray clones"
        );
        Ok(())
    }

    /// A cache handle returns `None` for a nonexistent key.
    #[tokio::test]
    async fn cache_miss_returns_none() {
        let handle = CacheHandle::new(1000, Duration::from_secs(60));
        assert!(
            handle
                .get("test-miss-key-nonexistent-version-untouched")
                .await
                .is_none()
        );
    }

    /// After `invalidate_all` (a single version bump), every previously-
    /// inserted entry must read as a miss because the version recorded in
    /// the cached entry no longer matches `current_version`.
    #[tokio::test]
    async fn cache_invalidate_all_clears_entries() {
        let handle = CacheHandle::new(1000, Duration::from_secs(60));
        let prefix = Uuid::new_v4().to_string();
        let key1 = format!("{prefix}-inv-key1");
        let key2 = format!("{prefix}-inv-key2");
        let arc = Arc::new(sample_results());
        handle.insert(key1.clone(), arc.clone()).await;
        handle.insert(key2.clone(), arc.clone()).await;
        assert!(
            handle.get(&key1).await.is_some(),
            "pre-bump: key1 must be present"
        );
        assert!(
            handle.get(&key2).await.is_some(),
            "pre-bump: key2 must be present"
        );
        handle.invalidate_all();
        assert!(
            handle.get(&key1).await.is_none(),
            "post-bump: key1 must be a stale-entry miss"
        );
        assert!(
            handle.get(&key2).await.is_none(),
            "post-bump: key2 must be a stale-entry miss"
        );
    }

    /// After `invalidate_all`, the underlying moka entry is still stored —
    /// only the version check makes `get` reject it. A subsequent `insert`
    /// at the same key, before the TTL expires, returns the *new* payload
    /// (verified via pointer inequality).
    #[expect(
        clippy::expect_used,
        reason = "test assertion must hard-fail with a clear message if the preconditions are not met"
    )]
    #[tokio::test]
    async fn cache_invalidate_then_reinsert_returns_fresh_payload() {
        let handle = CacheHandle::new(1000, Duration::from_secs(60));
        let key = format!("{prefix}-reinsert", prefix = Uuid::new_v4());

        let first = Arc::new(sample_results());
        handle.insert(key.clone(), first.clone()).await;
        let pre_bump_first = handle
            .get(&key)
            .await
            .expect("first insert must be visible");
        assert!(Arc::ptr_eq(&first, &pre_bump_first));

        handle.invalidate_all();
        assert!(
            handle.get(&key).await.is_none(),
            "post-bump: stale entry must read as None"
        );

        let second = Arc::new(sample_results());
        handle.insert(key.clone(), second.clone()).await;
        let post_bump_second = handle.get(&key).await.expect("reinsert must be visible");
        assert!(
            Arc::ptr_eq(&second, &post_bump_second),
            "reinsert should share the new Arc"
        );
        assert!(
            !Arc::ptr_eq(&first, &post_bump_second),
            "must NOT return the stale payload after a reinsert"
        );
    }

    /// `current_version` reflects bumps monotonically.
    #[tokio::test]
    async fn cache_version_monotonic() {
        let handle = CacheHandle::new(1000, Duration::from_secs(60));
        let v0 = handle.current_version();
        handle.invalidate_all();
        let v1 = handle.current_version();
        assert!(v1 > v0, "version must increase after fetch_add");
        handle.invalidate_all();
        let v2 = handle.current_version();
        assert!(v2 > v1, "version must keep increasing on each bump");
    }
}
