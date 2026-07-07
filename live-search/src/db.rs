//! Database types, pool management, and `PostgreSQL` LISTEN/NOTIFY integration.
//!
//! The SSR binary uses a global [`sqlx::PgPool`] (guarded by
//! [`std::sync::OnceLock`]) and a background listener task that subscribes
//! to the `search_results` channel and forwards notifications into a
//! [`tokio::sync::broadcast::Sender`] consumed by the SSE handler.
//!
//! A parallel watchdog task monitors liveness of the `PgListener` and triggers
//! a reconnection when no notifications have been received for a threshold
//! period. The watchdog is a **separate** task with its own `CancellationToken`
//! and an `Arc<AtomicU64>` last-seen timestamp — it is NOT inside the existing
//! `biased;` select! (per oracle I3).

#[cfg(feature = "ssr")]
use chrono::{DateTime, Utc};
#[cfg(feature = "ssr")]
use uuid::Uuid;

/// Re-export the canonical domain type.
pub use rwf_domain::SearchResult;

/// Row-level representation for `sqlx` queries, mirroring the database columns.
///
/// The canonical [`SearchResult`] lives in the `rwf-domain` crate with no
/// `sqlx` dependency, so we map through this wrapper when executing queries
/// that need `#[derive(sqlx::FromRow)]`.
#[cfg(feature = "ssr")]
#[derive(Debug, sqlx::FromRow)]
#[must_use]
#[doc(hidden)] // Published for benches in the same package; not part of the public API.
pub struct SearchResultRow {
    pub id: Uuid,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub created_at: DateTime<Utc>,
}

#[cfg(feature = "ssr")]
impl From<SearchResultRow> for SearchResult {
    fn from(row: SearchResultRow) -> Self {
        Self {
            id: row.id,
            title: row.title,
            url: row.url,
            snippet: row.snippet,
            created_at: row.created_at,
        }
    }
}

// ---------------------------------------------------------------------------
// Server‑only — compiled only when building for the SSR server.
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
mod server {
    use std::sync::PoisonError;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    #[cfg(feature = "test-seams")]
    use std::sync::OnceLock;

    use serde::Deserialize;
    use sqlx::PgPool;
    use sqlx::postgres::PgListener;
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;

    use chrono::{DateTime, Utc};
    use uuid::Uuid;

    use crate::cache::CacheHandle;
    use crate::events::SseEvent;

    // ------------------------------------------------------------------
    // Test-seam: global pool for e2e tests
    // ------------------------------------------------------------------
    //
    // Production code uses `crate::state::AppContext::pool` instead. These
    // items are only available when the `test-seams` feature is active
    // (enabled by `e2e-tests/Cargo.toml`).

    /// Global database pool, initialized once on startup.
    #[cfg(feature = "test-seams")]
    static POOL: OnceLock<PgPool> = OnceLock::new();

    /// Error returned when the global database pool cannot be initialized.
    #[cfg(feature = "test-seams")]
    #[derive(Debug, thiserror::Error)]
    #[non_exhaustive]
    pub enum PoolInitError {
        /// The pool was already set earlier in the process lifetime.
        #[error("database pool already initialized")]
        AlreadyInitialized,
    }

    /// Sets the global database pool (test seam — prefer `AppContext`).
    ///
    /// `PgPool` is an `Arc`-backed handle — cloning it is a cheap refcount
    /// bump.
    ///
    /// # Errors
    /// Returns `PoolInitError::AlreadyInitialized` if startup tries to set
    /// the pool more than once.
    #[cfg(feature = "test-seams")]
    pub fn set_pool(pool: PgPool) -> Result<(), PoolInitError> {
        POOL.set(pool)
            .map_err(|_| PoolInitError::AlreadyInitialized)
    }

    /// Returns a reference to the global database pool (test seam).
    #[cfg(feature = "test-seams")]
    #[must_use]
    pub fn get_pool() -> Option<&'static PgPool> {
        POOL.get()
    }

    /// Pool-tuning knobs as the `db` module understands them.
    ///
    /// This struct is the local mirror of `rwf_config::LiveSearchConfig`'s
    /// pool fields, flattened so callers (mostly bootstrap) can pass in the
    /// tunables without depending on `rwf-config` from inside `db`. A
    /// `From<&LiveSearchConfig>` impl lives in the `bootstrap` module.
    #[derive(Debug, Clone, Copy)]
    pub struct PoolTunables {
        /// Hard cap on connections the pool will open.
        pub max_connections: u32,
        /// Pre-warmed connections maintained in the background.
        pub min_connections: u32,
        /// Max time to wait for a free connection before timing out.
        pub acquire_timeout_secs: u64,
        /// Close idle connections older than this.
        pub idle_timeout_secs: u64,
        /// Close and replace connections older than this.
        pub max_lifetime_secs: u64,
    }

    impl Default for PoolTunables {
        fn default() -> Self {
            Self {
                max_connections: 20,
                min_connections: 2,
                acquire_timeout_secs: 5,
                idle_timeout_secs: 600,
                max_lifetime_secs: 1800,
            }
        }
    }

    /// Create a new connection pool from the given database URL.
    ///
    /// At all times one pool connection is held by the [`PgListener`] task,
    /// reducing the effective request-handler capacity by one. The pool's
    /// `max_connections` includes that connection; total handler-request
    /// capacity is `max_connections - 1` under load.
    ///
    /// Hardening applied (all fields overridable via [`PoolTunables`]):
    /// - `test_before_acquire(true)` — verifies a connection is alive before
    ///   handing it to a request handler.
    /// - `idle_timeout(cfg.idle_timeout_secs)` — closes idle connections
    ///   after that many seconds.
    /// - `max_lifetime(cfg.max_lifetime_secs)` — recycles connections after
    ///   that many seconds.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if connecting to `PostgreSQL`
    /// fails.
    pub async fn create_pool(
        database_url: &str,
        tunables: &PoolTunables,
    ) -> Result<PgPool, sqlx::Error> {
        PgPoolOptions::new()
            .max_connections(tunables.max_connections)
            .min_connections(tunables.min_connections)
            .acquire_timeout(Duration::from_secs(tunables.acquire_timeout_secs))
            .test_before_acquire(true)
            .idle_timeout(Duration::from_secs(tunables.idle_timeout_secs))
            .max_lifetime(Duration::from_secs(tunables.max_lifetime_secs))
            .connect(database_url)
            .await
    }

    /// Close the database pool with a 5-second timeout.
    ///
    /// If the pool does not close within the timeout, the caller should proceed
    /// regardless (the pool's `Arc`-backed handle will be dropped).
    ///
    /// This mirrors the `force_flush` / `shutdown` timeout pattern from
    /// rust-tracing §1.7 / §3.2.
    pub async fn close_pool(pool: &PgPool) {
        if tokio::time::timeout(Duration::from_secs(5), pool.close())
            .await
            .is_err()
        {
            tracing::warn!("database pool did not close within 5s timeout");
        }
    }

    #[derive(Debug, Deserialize)]
    struct SearchResultNotification {
        title: String,
        url: String,
        snippet: String,
    }

    /// Connect to `PostgreSQL` and subscribe to the `search_results` channel.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if connecting or `LISTEN` fails.
    async fn connect_and_listen(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen("search_results").await?;
        Ok(listener)
    }

    /// Maximum number of bytes of an unparseable NOTIFY payload we will echo
    /// into error logs. The full payload is user-supplied content (titles, URLs,
    /// snippets); it may be PII under GDPR, and a single multi-MB row can
    /// produce a multi-MB log line. Truncate aggressively.
    const PAYLOAD_LOG_PREVIEW_BYTES: usize = 200;

    /// Forward a single `NOTIFY` payload to the broadcast channel.
    ///
    /// Intentionally **not** `#[tracing::instrument]` — the record-via-current-span
    /// pattern is fragile because fmt layers **append** field values instead of
    /// replacing them; a single `tracing::debug!` at the call site avoids that.
    ///
    /// Synchronous: cache invalidation is a single atomic `fetch_add` and
    /// broadcast send is internally synchronous; we keep this non-`async`
    /// to avoid the runtime cost of an extra future state machine per
    /// notification.
    fn forward_notification(
        tx: &broadcast::Sender<SseEvent>,
        notification: &sqlx::postgres::PgNotification,
        cache: &CacheHandle,
    ) {
        let payload = notification.payload();

        match serde_json::from_str::<SearchResultNotification>(payload) {
            Ok(row) => {
                let event = SseEvent::SearchResult {
                    title: Arc::from(row.title),
                    url: Arc::from(row.url),
                    snippet: Arc::from(row.snippet),
                };
                match tx.send(event) {
                    Ok(receivers) => {
                        tracing::debug!(
                            channel = %notification.channel(),
                            payload_len = payload.len(),
                            receivers,
                            "forwarded search result notification"
                        );
                    }
                    Err(e) => {
                        tracing::warn!(
                            channel = %notification.channel(),
                            error = %e,
                            "search result notification had no SSE receivers"
                        );
                    }
                }

                // Data has changed — bump the search cache version so the
                // next search query re-fetches from the database. Synchronous
                // fetch_add on the version atomic; no .await needed.
                cache.invalidate_all();
            }
            Err(e) => {
                // Do NOT log the full payload: it is unbounded user content and
                // may be PII. Record only length and a bounded preview.
                let preview: String = payload.chars().take(PAYLOAD_LOG_PREVIEW_BYTES).collect();
                tracing::error!(
                    channel = %notification.channel(),
                    payload_len = payload.len(),
                    payload_preview = %preview,
                    error = %e,
                    "invalid search_results notification payload"
                );
            }
        }
    }

    /// Sleep for `dur`, but return early if `shutdown` is cancelled.
    async fn sleep_or_shutdown(dur: Duration, shutdown: &CancellationToken) -> bool {
        tokio::select! {
            () = shutdown.cancelled() => true,
            () = tokio::time::sleep(dur) => false,
        }
    }

    // ------------------------------------------------------------------
    // Watchdog constants
    // ------------------------------------------------------------------

    /// If no `NOTIFY` has been received for this duration, the watchdog
    /// triggers a reconnection of the `PgListener`.
    const WATCHDOG_STALE_THRESHOLD: Duration = Duration::from_secs(90);

    /// Interval at which the watchdog checks the last-seen timestamp.
    const WATCHDOG_CHECK_INTERVAL: Duration = Duration::from_secs(15);

    // ------------------------------------------------------------------
    // PgListener — main task
    // ------------------------------------------------------------------

    /// Listen on the `search_results` `PostgreSQL` channel and forward
    /// notifications as `SseEvent::SearchResult` into the broadcast channel.
    ///
    /// The task cooperatively exits when `shutdown` is cancelled, satisfying
    /// the `async-cancellation-token` and `async-structured-concurrency` rules.
    /// Uses **exponential backoff** with reset-on-success for both connect and
    /// recv failures, and `biased;` in the inner `select!` so shutdown always
    /// wins ties against an incoming NOTIFY.
    ///
    /// A `reconnect_requested` counter (shared with a watchdog task) is checked
    /// on each recv and periodically via a sleep branch: when the value differs
    /// from `last_reconnect_version`, the listener breaks the inner loop and
    /// re-establishes the connection. The `last_recv` timestamp in the shared
    /// `Arc<Mutex<Option<Instant>>>` is updated on every
    /// successfully received notification so the watchdog can detect staleness.
    #[tracing::instrument(skip_all)]
    pub async fn run_pg_listener(
        pool: PgPool,
        tx: broadcast::Sender<SseEvent>,
        cache: CacheHandle,
        shutdown: CancellationToken,
        reconnect_requested: Arc<AtomicU64>,
        last_recv: Arc<Mutex<Option<Instant>>>,
    ) {
        // Exponential backoff: 250 ms → 30 s, doubling on each consecutive
        // failure, reset to the floor on a successful connect/recv.
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(30);
        const BACKOFF_FLOOR_MS: u64 = 250;

        let mut last_reconnect_version: u64 = reconnect_requested.load(Ordering::Acquire);

        while !shutdown.is_cancelled() {
            let mut listener = match connect_and_listen(&pool).await {
                Ok(l) => {
                    tracing::info!("Listening on search_results channel");
                    backoff = Duration::from_millis(BACKOFF_FLOOR_MS);
                    l
                }
                Err(e) => {
                    tracing::error!(
                        backoff_ms = backoff.as_millis(),
                        error = %e,
                        "PG listener setup failed; will retry after backoff"
                    );
                    if sleep_or_shutdown(backoff, &shutdown).await {
                        return;
                    }
                    backoff = (backoff * 2).min(max_backoff);
                    continue;
                }
            };

            loop {
                // Periodically check the reconnect counter so the watchdog
                // can force a reconnect even without an incoming NOTIFY.
                let check_interval = tokio::time::sleep(WATCHDOG_CHECK_INTERVAL);

                // `biased;` ensures shutdown is checked first when both branches
                // are simultaneously ready, removing the branch-pick race that
                // can otherwise delay shutdown by one notification cycle.
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        tracing::info!("PgListener shutting down");
                        return;
                    }
                    () = check_interval => {
                        let current = reconnect_requested.load(Ordering::Acquire);
                        if current != last_reconnect_version {
                            last_reconnect_version = current;
                            tracing::warn!(
                                version = current,
                                "PgListener watchdog triggered reconnect from periodic check"
                            );
                            break; // reconnect outer loop
                        }
                    }
                    recv = listener.recv() => {
                        // Update last-seen timestamp for the watchdog.
                        *last_recv.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());

                        // Check reconnect AFTER updating last_recv so the
                        // watchdog sees the fresh timestamp on its next cycle.
                        let current = reconnect_requested.load(Ordering::Acquire);
                        if current != last_reconnect_version {
                            last_reconnect_version = current;
                            tracing::warn!(
                                version = current,
                                "PgListener watchdog triggered reconnect on recv"
                            );
                            break; // reconnect outer loop
                        }

                        match recv {
                            Ok(notification) => {
                                backoff = Duration::from_millis(BACKOFF_FLOOR_MS);
                                forward_notification(&tx, &notification, &cache);
                            }
                            Err(e) => {
                                tracing::error!(
                                    backoff_ms = backoff.as_millis(),
                                    error = %e,
                                    "PG listener receive failed; will reconnect after backoff"
                                );
                                if sleep_or_shutdown(backoff, &shutdown).await {
                                    return;
                                }
                                backoff = (backoff * 2).min(max_backoff);
                                break; // reconnect outer loop
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("PgListener exited cleanly");
    }

    // ------------------------------------------------------------------
    // Watchdog — separate parallel task
    // ------------------------------------------------------------------

    // ------------------------------------------------------------------
    // Watchdog check — extracted for unit testability
    // ------------------------------------------------------------------

    /// Perform a single watchdog check.
    ///
    /// If `last_recv` contains an [`Instant`] whose elapsed time exceeds
    /// [`WATCHDOG_STALE_THRESHOLD`], increments `reconnect_requested` to
    /// trigger a reconnection in the listener task.
    ///
    /// This function uses [`Instant`] (monotonic clock) so that NTP step-back
    /// events (which cause `SystemTime` to jump backward) do not silently
    /// reset the watchdog.
    ///
    /// The check is a no-op when `last_recv` is `None` (no notification has
    /// ever been received — the listener may still be establishing a
    /// connection).
    fn run_watchdog_check(last_recv: &Mutex<Option<Instant>>, reconnect_requested: &AtomicU64) {
        let guard = last_recv.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(instant) = *guard
            && instant.elapsed() > WATCHDOG_STALE_THRESHOLD
        {
            tracing::warn!(
                elapsed_ms = instant.elapsed().as_millis(),
                "PgListener watchdog detected stale connection; triggering reconnect",
            );
            reconnect_requested.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Monitors the `PgListener`'s liveness and triggers a reconnection when no
    /// notifications have been received for `WATCHDOG_STALE_THRESHOLD`
    /// (90 seconds).
    ///
    /// This is a **separate parallel task** (per oracle I3), NOT inside the
    /// existing `biased;` select! in [`run_pg_listener`]. It has its own
    /// `CancellationToken` and the same `Arc<Mutex<Option<Instant>>>` last-seen timestamp
    /// that the listener updates.
    ///
    /// When staleness is detected, the watchdog increments
    /// `reconnect_requested`, causing the listener's next select! iteration
    /// to break and reconnect.
    #[tracing::instrument(skip_all)]
    pub async fn run_watchdog(
        last_recv: Arc<Mutex<Option<Instant>>>,
        reconnect_requested: Arc<AtomicU64>,
        shutdown: CancellationToken,
    ) {
        while !shutdown.is_cancelled() {
            tokio::select! {
                biased;
                () = shutdown.cancelled() => {
                    tracing::info!("PgListener watchdog shutting down");
                    return;
                }
                () = tokio::time::sleep(WATCHDOG_CHECK_INTERVAL) => {
                    run_watchdog_check(&last_recv, &reconnect_requested);
                }
            }
        }

        tracing::info!("PgListener watchdog exited cleanly");
    }

    // ------------------------------------------------------------------
    // Cursor-based pagination
    // ------------------------------------------------------------------

    /// Search results with cursor-based pagination.
    ///
    /// `cursor` is the last row's `(created_at, id)` from the previous page;
    /// pass `None` for the first page. The result set is bounded by `limit`,
    /// and rows are ordered by `(created_at DESC, id DESC)` for a stable scan.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if the database query fails or
    /// the connection cannot be acquired.
    pub async fn search_with_cursor(
        pool: &PgPool,
        query: &str,
        cursor: Option<(DateTime<Utc>, Uuid)>,
        limit: i64,
    ) -> Result<Vec<super::SearchResult>, sqlx::Error> {
        if let Some((cursor_time, cursor_id)) = cursor {
            let rows = sqlx::query_as::<_, super::SearchResultRow>(
                r"SELECT id, title, url, snippet, created_at
                   FROM search_results
                   WHERE fts @@ plainto_tsquery('english', $1)
                     AND (created_at, id) < ($2, $3)
                   ORDER BY created_at DESC, id DESC
                   LIMIT $4",
            )
            .bind(query)
            .bind(cursor_time)
            .bind(cursor_id)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(Into::into).collect())
        } else {
            let rows = sqlx::query_as::<_, super::SearchResultRow>(
                r"SELECT id, title, url, snippet, created_at
                   FROM search_results
                   WHERE fts @@ plainto_tsquery('english', $1)
                   ORDER BY created_at DESC, id DESC
                   LIMIT $2",
            )
            .bind(query)
            .bind(limit)
            .fetch_all(pool)
            .await?;
            Ok(rows.into_iter().map(Into::into).collect())
        }
    }

    /// Base-64-url-encode `bytes` using the URL-safe no-pad alphabet.
    ///
    /// Thin wrapper around the `base64` crate's `URL_SAFE_NO_PAD` engine.
    #[must_use]
    pub fn base64url_encode(input: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(input)
    }

    /// Decode a base64url string back to bytes.
    ///
    /// Thin wrapper around the `base64` crate's `URL_SAFE_NO_PAD` engine.
    ///
    /// # Errors
    /// Returns an error string if the input contains invalid base64url characters.
    pub fn base64url_decode(input: &str) -> Result<Vec<u8>, String> {
        URL_SAFE_NO_PAD
            .decode(input)
            .map_err(|e| format!("base64 decode failed: {e}"))
    }

    /// Encode a cursor as base64url of `"{timestamp_micros}|{uuid}"`.
    #[must_use]
    pub fn encode_cursor(created_at: DateTime<Utc>, id: Uuid) -> String {
        let raw = format!("{}|{id}", created_at.timestamp_micros());
        base64url_encode(raw.as_bytes())
    }

    /// Decode a cursor string back to `(DateTime<Utc>, Uuid)`.
    ///
    /// # Errors
    /// Returns an error string if the base64 decoding fails, the parts are
    /// missing, the timestamp is invalid, or the UUID is invalid.
    pub fn decode_cursor(s: &str) -> Result<(DateTime<Utc>, Uuid), String> {
        let bytes = base64url_decode(s)?;
        let raw = std::str::from_utf8(&bytes).map_err(|e| format!("invalid utf-8: {e}"))?;
        let mut parts = raw.splitn(2, '|');
        let ts_str = parts
            .next()
            .ok_or_else(|| "missing timestamp".to_string())?;
        let id_str = parts.next().ok_or_else(|| "missing uuid".to_string())?;
        let micros: i64 = ts_str
            .parse()
            .map_err(|e| format!("invalid timestamp: {e}"))?;
        let ts = DateTime::<Utc>::from_timestamp_micros(micros)
            .ok_or_else(|| format!("invalid timestamp value: {micros}"))?;
        let id: Uuid = id_str.parse().map_err(|e| format!("invalid uuid: {e}"))?;
        Ok((ts, id))
    }

    // ------------------------------------------------------------------
    // Tests
    // ------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;
        use anyhow::Context;

        /// Verify the watchdog fires when `last_recv` is older than the stale
        /// threshold. The check uses `Instant` (monotonic) so NTP step-back
        /// cannot silently reset the timer.
        #[tokio::test]
        async fn watchdog_uses_monotonic_time() -> anyhow::Result<()> {
            let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let reconnect_requested = Arc::new(AtomicU64::new(0));

            // Set last_recv to 5 minutes ago — well past the 90s threshold.
            // `Instant` is monotonic since boot; `checked_sub` cannot underflow
            // on any platform that supports this test, but we propagate the
            // `None` case via `?` so the test fails clearly rather than panicking.
            let five_min_ago = Instant::now()
                .checked_sub(Duration::from_secs(300))
                .context("monotonic Instant should be at least 5 minutes past origin")?;
            *last_recv.lock().unwrap_or_else(PoisonError::into_inner) = Some(five_min_ago);

            run_watchdog_check(&last_recv, &reconnect_requested);

            assert!(
                reconnect_requested.load(Ordering::Acquire) >= 1,
                "watchdog should fire for a 5-minute-old last_recv"
            );
            Ok(())
        }

        /// The watchdog is a no-op when no notification has ever been
        /// received (`last_recv` is `None`).
        #[tokio::test]
        async fn watchdog_skips_when_last_recv_is_none() {
            let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let reconnect_requested = Arc::new(AtomicU64::new(0));

            run_watchdog_check(&last_recv, &reconnect_requested);

            assert_eq!(
                reconnect_requested.load(Ordering::Acquire),
                0,
                "watchdog should skip when no notification ever received"
            );
        }

        /// Verify the watchdog does NOT fire when `last_recv` is recent (within
        /// threshold).
        #[tokio::test]
        async fn watchdog_does_not_fire_for_recent_recv() -> anyhow::Result<()> {
            let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let reconnect_requested = Arc::new(AtomicU64::new(0));

            // Just a few seconds ago — well within the 90s threshold.
            let recent = Instant::now()
                .checked_sub(Duration::from_secs(5))
                .context("monotonic Instant should be at least 5 seconds past origin")?;
            *last_recv.lock().unwrap_or_else(PoisonError::into_inner) = Some(recent);

            run_watchdog_check(&last_recv, &reconnect_requested);

            assert_eq!(
                reconnect_requested.load(Ordering::Acquire),
                0,
                "watchdog should NOT fire for a 5-second-old last_recv"
            );
            Ok(())
        }

        #[test]
        fn cursor_encode_decode_roundtrip() -> anyhow::Result<()> {
            let original_time = Utc::now();
            let original_id = Uuid::new_v4();
            let encoded = super::encode_cursor(original_time, original_id);
            let (decoded_time, decoded_id) = super::decode_cursor(&encoded)
                .map_err(|e| anyhow::anyhow!("decode failed: {e}"))?;
            assert_eq!(
                decoded_time.timestamp_micros(),
                original_time.timestamp_micros()
            );
            assert_eq!(decoded_id, original_id);
            Ok(())
        }

        #[test]
        fn cursor_decode_rejects_garbage() {
            assert!(super::decode_cursor("not-a-cursor").is_err());
            assert!(super::decode_cursor("").is_err());
            // Characters not in the base64url alphabet → decode failure
            assert!(super::decode_cursor("!!!notbase64!!!").is_err());
        }
    }
}

// Re-export server functions at the module level so callers can write
// `db::create_pool(…)` etc. without changing import paths.
#[cfg(feature = "ssr")]
pub use server::{
    PoolTunables, base64url_decode, base64url_encode, close_pool, create_pool, decode_cursor,
    encode_cursor, run_pg_listener, run_watchdog, search_with_cursor,
};

// Test-seam API — only available when the feature is enabled (e2e-tests).
#[cfg(all(feature = "ssr", feature = "test-seams"))]
pub use server::{PoolInitError, get_pool, set_pool};
