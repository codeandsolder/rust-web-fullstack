//! Database types, pool management, and `PostgreSQL` LISTEN/NOTIFY integration.
//!
//! The SSR binary uses a global [`PgPool`] (guarded by [`OnceLock`]) and a
//! background listener task that subscribes to the `search_results` channel
//! and forwards notifications into a [`broadcast::Sender`] consumed by the SSE
//! handler.
//!
//! A parallel watchdog task monitors liveness of the `PgListener` and triggers
//! a reconnection when no notifications have been received for a threshold
//! period. The watchdog is a **separate** task with its own `CancellationToken`
//! and an `Arc<AtomicU64>` last-seen timestamp — it is NOT inside the existing
//! `biased;` select! (per oracle I3).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A search result as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ssr", derive(sqlx::FromRow))]
#[must_use]
pub struct SearchResult {
    pub id: Uuid,
    pub title: String,
    pub url: String,
    pub snippet: String,
    pub created_at: DateTime<Utc>,
}

// ---------------------------------------------------------------------------
// Server‑only — compiled only when building for the SSR server.
// ---------------------------------------------------------------------------

#[cfg(feature = "ssr")]
mod server {
    use std::sync::PoisonError;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use std::time::{Duration, Instant};

    use serde::Deserialize;
    use sqlx::PgPool;
    use sqlx::postgres::PgListener;
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

    use crate::cache;
    use crate::events::SseEvent;

    /// Global database pool, initialized once on startup.
    static POOL: OnceLock<PgPool> = OnceLock::new();

    /// Error returned when the global database pool cannot be initialized.
    #[derive(Debug, thiserror::Error)]
    #[non_exhaustive]
    pub enum PoolInitError {
        /// The pool was already set earlier in the process lifetime.
        #[error("database pool already initialized")]
        AlreadyInitialized,
    }

    /// Sets the global database pool.
    ///
    /// `PgPool` is an `Arc`-backed handle — cloning it is a cheap refcount
    /// bump, so a single pool can be handed to [`set_pool`] (which stashes
    /// the handle in a [`OnceLock`]) and simultaneously driven by a long-lived
    /// task like [`run_pg_listener`] without owning contention.
    ///
    /// # Errors
    /// Returns [`PoolInitError::AlreadyInitialized`] if startup tries to set
    /// the pool more than once.
    pub fn set_pool(pool: PgPool) -> Result<(), PoolInitError> {
        POOL.set(pool)
            .map_err(|_| PoolInitError::AlreadyInitialized)
    }

    /// Returns a reference to the global database pool.
    #[must_use]
    pub fn get_pool() -> Option<&'static PgPool> {
        POOL.get()
    }

    /// Create a new connection pool from the given database URL.
    ///
    /// At all times one pool connection is held by the [`PgListener`] task,
    /// reducing the effective request-handler capacity by one. The pool's
    /// `max_connections` includes that connection; total handler-request
    /// capacity is `max_connections - 1` under load.
    ///
    /// Hardening applied:
    /// - `test_before_acquire(true)` — verifies a connection is alive before
    ///   handing it to a request handler.
    /// - `idle_timeout(600s)` — closes idle connections after 10 minutes.
    /// - `max_lifetime(1800s)` — recycles connections after 30 minutes.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if connecting to `PostgreSQL`
    /// fails.
    pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
        PgPoolOptions::new()
            .max_connections(20)
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .test_before_acquire(true)
            .idle_timeout(Duration::from_secs(600))
            .max_lifetime(Duration::from_secs(1800))
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
    fn forward_notification(
        tx: &broadcast::Sender<SseEvent>,
        notification: &sqlx::postgres::PgNotification,
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

                // Data has changed — invalidate the search cache so the next
                // search query re-fetches from the database.
                cache::invalidate_all();
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
                                forward_notification(&tx, &notification);
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
    /// notifications have been received for [`WATCHDOG_STALE_THRESHOLD`].
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
    // Tests
    // ------------------------------------------------------------------

    #[cfg(test)]
    mod tests {
        use super::*;

        /// Verify the watchdog fires when `last_recv` is older than the stale
        /// threshold. The check uses `Instant` (monotonic) so NTP step-back
        /// cannot silently reset the timer.
        #[tokio::test]
        async fn watchdog_uses_monotonic_time() {
            let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let reconnect_requested = Arc::new(AtomicU64::new(0));

            // Set last_recv to 5 minutes ago — well past the 90s threshold.
            #[allow(clippy::unchecked_time_subtraction)]
            let five_min_ago = Instant::now() - Duration::from_secs(300);
            *last_recv.lock().unwrap_or_else(PoisonError::into_inner) = Some(five_min_ago);

            run_watchdog_check(&last_recv, &reconnect_requested);

            assert!(
                reconnect_requested.load(Ordering::Acquire) >= 1,
                "watchdog should fire for a 5-minute-old last_recv"
            );
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
        async fn watchdog_does_not_fire_for_recent_recv() {
            let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));
            let reconnect_requested = Arc::new(AtomicU64::new(0));

            // Just a few seconds ago — well within the 90s threshold.
            #[allow(clippy::unchecked_time_subtraction)]
            let recent = Instant::now() - Duration::from_secs(5);
            *last_recv.lock().unwrap_or_else(PoisonError::into_inner) = Some(recent);

            run_watchdog_check(&last_recv, &reconnect_requested);

            assert_eq!(
                reconnect_requested.load(Ordering::Acquire),
                0,
                "watchdog should NOT fire for a 5-second-old last_recv"
            );
        }

        /// `PgPool` capacity test: verify the docstring claim that a
        /// `PgListener` holds one pool connection, reducing effective
        /// request-handler capacity by one.
        ///
        /// Sets `max_connections = 3`. After the listener borrows 1, the
        /// pool can serve at most 2 concurrent queries. Launching 3
        /// concurrent queries should result in at most 2 succeeding.
        ///
        /// **Requires a live `PostgreSQL` instance**. The test is ignored by
        /// default. Run with:
        ///
        /// ```sh
        /// DATABASE_URL="postgres://…" cargo test -p live-search \
        ///     --features ssr --lib -- --ignored \
        ///     pool_with_listener_holds_one_connection
        /// ```
        #[tokio::test]
        #[ignore = "requires DATABASE_URL pointing to a live PostgreSQL instance"]
        async fn pool_with_listener_holds_one_connection() {
            let Ok(url) = std::env::var("DATABASE_URL") else {
                eprintln!("SKIP: DATABASE_URL not set");
                return;
            };

            let pool = PgPoolOptions::new()
                .max_connections(3)
                .acquire_timeout(Duration::from_millis(500))
                .connect(&url)
                .await
                .expect("connect to database");

            let _listener = PgListener::connect_with(&pool)
                .await
                .expect("create listener");

            // max_connections=3, listener holds 1 → 2 connections available.
            // Launch 3 concurrent queries; the 3rd should time out.
            let handles = vec![
                sqlx::query("SELECT 1").fetch_one(&pool),
                sqlx::query("SELECT 1").fetch_one(&pool),
                sqlx::query("SELECT 1").fetch_one(&pool),
            ];
            let results = futures::future::join_all(handles).await;

            // At most 2 of the 3 queries should succeed.
            let success_count = results.iter().filter(|r| r.is_ok()).count();
            assert!(
                success_count <= 2,
                "at most 2 of 3 concurrent queries should succeed \
                 (max_connections=3, 1 held by listener); got {success_count}"
            );

            pool.close().await;
        }
    }
}

// Re-export server functions at the module level so callers can write
// `db::create_pool(…)` etc. without changing import paths.
#[cfg(feature = "ssr")]
pub use server::{close_pool, create_pool, get_pool, run_pg_listener, run_watchdog, set_pool};
