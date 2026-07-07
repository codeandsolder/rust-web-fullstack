//! In-memory search results cache backed by `moka`, with **version-based**
//! invalidation.
//!
//! A `moka::future::Cache<String, Arc<CachedEntry>>` keyed by the normalized
//! query string (trimmed, lower-cased). Each entry carries the `CACHE_VERSION`
//! it was inserted at; `get` ignores entries whose version no longer matches.
//! Each entry has a 60-second TTL and the cache holds at most 1000 entries.
//!
//! On every `PostgreSQL` `NOTIFY` (row insert/update/delete) we bump
//! `CACHE_VERSION` by one (a single `fetch_add(1)`). No cache walk is needed.
//! Stale entries linger harmlessly until moka's TTL expires them and the
//! `get` comparison short-circuits the cost. This is cheaper than
//! `cache.invalidate_all()` (which iterates the whole cache) when the cache
//! is warm and write-heavy.
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
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use moka::future::Cache;

use rwf_domain::SearchResult;

/// Monotonic cache-generation counter.
///
/// Bumped by `invalidate_all` (a single `fetch_add`). All `get`s compare
/// their stored version against this counter and ignore stale entries.
/// Stored as a process-global atomic; there is no per-key counter because
/// we never invalidate a subset of keys (we don't know which queries a new
/// row would match).
static CACHE_VERSION: AtomicU64 = AtomicU64::new(0);

/// A cached search result bundle plus the version of `CACHE_VERSION` it
/// was inserted at. Wrap in `Arc` so moka stores a refcount bump per
/// subscriber rather than cloning the inner `Vec` on every `get`.
struct CachedEntry {
    data: Arc<Vec<SearchResult>>,
    version: u64,
}

/// Shared cache instance, initialised once at startup.
static SEARCH_CACHE: OnceLock<Cache<String, Arc<CachedEntry>>> = OnceLock::new();

/// Initialise the search results cache.
///
/// Must be called once during server startup before accepting search
/// requests. Subsequent calls are a no-op (the `OnceLock` retains the first
/// value).
pub fn init_cache() {
    SEARCH_CACHE.get_or_init(|| {
        Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .max_capacity(1000)
            .build()
    });
}

/// Read the current `CACHE_VERSION`. Exposed for tests and observability.
///
/// Tests use this to assert that version bumps invalidate entries. The
/// `cache_invalidate_all_clears_entries` test compares pre-/post-bump
/// versions without coupling to the internal atomic's exact value.
#[must_use]
pub fn current_cache_version() -> u64 {
    CACHE_VERSION.load(Ordering::Acquire)
}

/// Retrieve cached results for a query, if present **and** the entry's
/// version still matches the current `CACHE_VERSION`.
///
/// A version mismatch returns `None`; the caller refetches and re-inserts.
/// Stale entries are dropped lazily by moka's TTL once the version bumps
/// age them out of every subsequent read.
///
/// The query is used as-is (the caller should normalise before calling).
#[must_use]
pub async fn get(query: &str) -> Option<Arc<Vec<SearchResult>>> {
    let cache = SEARCH_CACHE.get()?;
    let entry = cache.get(query).await?;
    if entry.version != CACHE_VERSION.load(Ordering::Acquire) {
        return None;
    }
    Some(Arc::clone(&entry.data))
}

/// Insert results into the cache tagged with the current `CACHE_VERSION`.
///
/// This is a no-op if the cache has not been initialised (defensive).
pub async fn insert(query: String, results: Arc<Vec<SearchResult>>) {
    if let Some(cache) = SEARCH_CACHE.get() {
        let entry = Arc::new(CachedEntry {
            data: results,
            version: CACHE_VERSION.load(Ordering::Acquire),
        });
        cache.insert(query, entry).await;
    }
}

/// Invalidate every cached entry.
///
/// Called on every `NOTIFY` from the `search_results` channel so that
/// subsequent searches reflect the updated data. Implemented as a single
/// `fetch_add` on `CACHE_VERSION` — no walk through the cache is required
/// because each `get` re-checks the version against the atomic and skips
/// stale entries automatically. Stale entries linger until moka's TTL
/// expires them, at which point they are evicted.
///
/// The atomic increment itself is so cheap we don't bother rate-limiting
/// bursts (the previous design's `LAST_INVALIDATE` Mutex + Instant is now
/// redundant: even a tight loop of 10 000 NOTIFY events causes only 10 000
/// integer bumps and ten thousand `get` short-circuits, whereas the old
/// design did ten thousand full-cache scans).
pub fn invalidate_all() {
    let prev = CACHE_VERSION.fetch_add(1, Ordering::AcqRel);
    tracing::debug!(
        prev_version = prev,
        "search cache version bumped via NOTIFY",
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::OnceLock;
    use std::time::Duration;

    use anyhow::Context;
    use chrono::Utc;
    use uuid::Uuid;

    use super::*;

    /// Process-wide serialisation mutex for cache-invalidation tests.
    ///
    /// `CACHE_VERSION` and `SEARCH_CACHE` are global state, so any test that
    /// calls `invalidate_all` races with concurrent siblings. We use a
    /// [`tokio::sync::Mutex`] (rather than `std::sync::Mutex`) because the
    /// guard is held across `.await` points (`cache.get`, `cache.insert`),
    /// and `clippy::await_holding_lock` is denied at the workspace level.
    /// `tokio::sync::Mutex` is the correct primitive for serialising
    /// across `.await` rather than blocking the executor.
    fn cache_test_lock() -> &'static tokio::sync::Mutex<()> {
        static LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
    }

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
    ///
    /// Creates a local `moka` cache matching the production builder and
    /// verifies that the `Arc<Vec<SearchResult>>` retrieved is the same
    /// allocation as the one inserted. This is the property the production
    /// read path relies on (concurrent subscribers share the result heap).
    #[tokio::test]
    async fn cache_hit_returns_same_arc_instance() -> anyhow::Result<()> {
        let cache: Cache<String, Arc<CachedEntry>> = Cache::builder()
            .time_to_live(Duration::from_secs(60))
            .max_capacity(1000)
            .build();

        let key = "c2-arc-identity-key".to_string();
        let results = sample_results();
        let arc: Arc<Vec<SearchResult>> = Arc::new(results);

        cache
            .insert(
                key.clone(),
                Arc::new(CachedEntry {
                    data: arc.clone(),
                    version: 0,
                }),
            )
            .await;
        let retrieved = cache.get(&key).await;
        let retrieved = retrieved.context("cache hit should return Some")?;
        // The retrieved Arc's inner Vec must point to the SAME allocation
        // as the one we inserted. If the cache cloned the inner Vec, the
        // pointer would differ.
        assert!(
            Arc::ptr_eq(&retrieved.data, &arc),
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

    /// The global cache wrapper returns `None` for a nonexistent key.
    #[tokio::test]
    async fn cache_miss_returns_none() {
        let _guard = cache_test_lock().lock().await;
        init_cache();
        assert!(
            get("test-miss-key-nonexistent-version-untouched")
                .await
                .is_none()
        );
    }

    /// After `invalidate_all` (a single `CACHE_VERSION` bump), every
    /// previously-inserted entry must read as a miss because the version
    /// recorded in the cached entry no longer matches `current_cache_version`.
    ///
    /// Uses a unique key prefix per run so this test is safe to run in
    /// parallel with other tests that touch the global cache.
    #[tokio::test]
    async fn cache_invalidate_all_clears_entries() {
        let _guard = cache_test_lock().lock().await;
        init_cache();
        let prefix = Uuid::new_v4().to_string();
        let key1 = format!("{prefix}-inv-key1");
        let key2 = format!("{prefix}-inv-key2");
        let arc = Arc::new(sample_results());
        insert(key1.clone(), arc.clone()).await;
        insert(key2.clone(), arc.clone()).await;
        assert!(get(&key1).await.is_some(), "pre-bump: key1 must be present");
        assert!(get(&key2).await.is_some(), "pre-bump: key2 must be present");
        invalidate_all();
        assert!(
            get(&key1).await.is_none(),
            "post-bump: key1 must be a stale-entry miss"
        );
        assert!(
            get(&key2).await.is_none(),
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
        let _guard = cache_test_lock().lock().await;
        init_cache();
        let key = format!("{prefix}-reinsert", prefix = Uuid::new_v4());

        let first = Arc::new(sample_results());
        insert(key.clone(), first.clone()).await;
        let pre_bump_first = get(&key).await.expect("first insert must be visible");
        assert!(Arc::ptr_eq(&first, &pre_bump_first));

        invalidate_all();
        assert!(
            get(&key).await.is_none(),
            "post-bump: stale entry must read as None"
        );

        let second = Arc::new(sample_results());
        insert(key.clone(), second.clone()).await;
        let post_bump_second = get(&key).await.expect("reinsert must be visible");
        assert!(
            Arc::ptr_eq(&second, &post_bump_second),
            "reinsert should share the new Arc"
        );
        assert!(
            !Arc::ptr_eq(&first, &post_bump_second),
            "must NOT return the stale payload after a reinsert"
        );
    }

    /// `current_cache_version` reflects bumps monotonically.
    #[tokio::test]
    async fn cache_version_monotonic() {
        let _guard = cache_test_lock().lock().await;
        init_cache();
        let v0 = current_cache_version();
        invalidate_all();
        let v1 = current_cache_version();
        assert!(v1 > v0, "version must increase after fetch_add");
        invalidate_all();
        let v2 = current_cache_version();
        assert!(v2 > v1, "version must keep increasing on each bump");
    }
}
