//! Database types, pool management, and `PostgreSQL` LISTEN/NOTIFY integration.
//!
//! The SSR binary uses a global [`PgPool`] (guarded by [`OnceLock`]) and a
//! background listener task that subscribes to the `search_results` channel
//! and forwards notifications into a [`broadcast::Sender`] consumed by the SSE
//! handler.

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
    use std::sync::{Arc, OnceLock};
    use std::time::Duration;

    use serde::Deserialize;
    use sqlx::PgPool;
    use sqlx::postgres::PgListener;
    use sqlx::postgres::PgPoolOptions;
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;

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
    /// One connection is permanently reserved for the [`PgListener`] task, so
    /// the effective pool size available to request handlers is
    /// `max_connections - 1`.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if connecting to `PostgreSQL`
    /// fails.
    pub async fn create_pool(database_url: &str) -> Result<PgPool, sqlx::Error> {
        PgPoolOptions::new()
            .max_connections(20)
            .min_connections(2)
            .acquire_timeout(Duration::from_secs(5))
            .connect(database_url)
            .await
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

    /// Listen on the `search_results` `PostgreSQL` channel and forward
    /// notifications as `SseEvent::SearchResult` into the broadcast channel.
    ///
    /// The task cooperatively exits when `shutdown` is cancelled, satisfying
    /// the `async-cancellation-token` and `async-structured-concurrency` rules.
    /// Uses **exponential backoff** with reset-on-success for both connect and
    /// recv failures, and `biased;` in the inner `select!` so shutdown always
    /// wins ties against an incoming NOTIFY.
    #[tracing::instrument(skip_all)]
    pub async fn run_pg_listener(
        pool: PgPool,
        tx: broadcast::Sender<SseEvent>,
        shutdown: CancellationToken,
    ) {
        // Exponential backoff: 250 ms → 30 s, doubling on each consecutive
        // failure, reset to the floor on a successful connect/recv.
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(30);
        const BACKOFF_FLOOR_MS: u64 = 250;

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
                // `biased;` ensures shutdown is checked first when both branches
                // are simultaneously ready, removing the branch-pick race that
                // can otherwise delay shutdown by one notification cycle.
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        tracing::info!("PgListener shutting down");
                        return;
                    }
                    recv = listener.recv() => {
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
                                break; // breaks inner loop → reconnect
                            }
                        }
                    }
                }
            }
        }

        tracing::info!("PgListener exited cleanly");
    }
}

// Re-export server functions at the module level so callers can write
// `db::create_pool(…)` etc. without changing import paths.
#[cfg(feature = "ssr")]
pub use server::{create_pool, get_pool, run_pg_listener, set_pool};
