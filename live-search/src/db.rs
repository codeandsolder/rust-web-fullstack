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
    use std::sync::Arc;
    use std::time::Duration;

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
        id: Uuid,
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
    /// The notification payload is `{ "id": "<uuid>" }` (see
    /// `live-search/migrations/001_create_search_results.up.sql`). We
    /// look up the row by id so the payload stays small and bounded,
    /// regardless of how much text the row contains. Returns the row id
    /// on success so the caller can update its `last_seen_id` cursor.
    ///
    /// Intentionally **not** `#[tracing::instrument]` — the record-via-current-span
    /// pattern is fragile because fmt layers **append** field values instead of
    /// replacing them; a single `tracing::debug!` at the call site avoids that.
    async fn forward_notification(
        pool: &PgPool,
        tx: &broadcast::Sender<SseEvent>,
        notification: &sqlx::postgres::PgNotification,
        cache: &CacheHandle,
    ) -> Option<Uuid> {
        let payload = notification.payload();

        let id = match serde_json::from_str::<SearchResultNotification>(payload) {
            Ok(row) => row.id,
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
                return None;
            }
        };

        // Look up the row by id so the broadcast event carries the full
        // typed payload, not whatever the trigger chose to include.
        let row = sqlx::query_as::<_, super::SearchResultRow>(
            "SELECT id, title, url, snippet, created_at FROM search_results WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await
        .ok()
        .flatten();

        let Some(row) = row else {
            tracing::warn!(
                notification_id = %id,
                "search_results notification referenced a row that no longer exists",
            );
            return None;
        };

        let event = SseEvent::SearchResult {
            title: Arc::from(row.title.as_str()),
            url: Arc::from(row.url.as_str()),
            snippet: Arc::from(row.snippet.as_str()),
        };
        match tx.send(event) {
            Ok(receivers) => {
                tracing::debug!(
                    channel = %notification.channel(),
                    row_id = %row.id,
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

        cache.invalidate_all();
        Some(row.id)
    }

    /// Sleep for `dur`, but return early if `shutdown` is cancelled.
    async fn sleep_or_shutdown(dur: Duration, shutdown: &CancellationToken) -> bool {
        tokio::select! {
            () = shutdown.cancelled() => true,
            () = tokio::time::sleep(dur) => false,
        }
    }

    // ------------------------------------------------------------------
    // Watchdog constants — REMOVED. The "no notification for 90s" watchdog
    // was deleted: it conflated "quiet database" with "dead connection".
    // The replacement is `PgListener::try_recv()` in `run_pg_listener`,
    // which surfaces `ConnectionLost` directly and lets the listener
    // reconnect through the existing outer loop. See commit history.
    // ------------------------------------------------------------------

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
    /// After a `ConnectionLost` (or any other recv error indicating the
    /// listener must re-establish), the outer loop reconnects and then
    /// calls [`resync_after_reconnect`] to replay any rows the server
    /// missed while disconnected (LISTEN/NOTIFY is not durable delivery).
    #[tracing::instrument(skip_all)]
    pub async fn run_pg_listener(
        pool: PgPool,
        tx: broadcast::Sender<SseEvent>,
        cache: CacheHandle,
        shutdown: CancellationToken,
    ) {
        // Exponential backoff: 250 ms → 30 s, doubling on each consecutive
        // failure, reset to the floor on a successful connect/recv.
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(30);
        const BACKOFF_FLOOR_MS: u64 = 250;

        // Tracks the highest row id we've successfully forwarded. Used by
        // resync_after_reconnect to fetch rows we missed while disconnected.
        let mut last_seen_id: Uuid = Uuid::nil();

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

            // Resync after each (re)connect: replay anything we missed.
            // First connect uses last_seen_id = Uuid::nil() so this fetches
            // the entire table once.
            if let Err(e) = resync_after_reconnect(&pool, &tx, &cache, last_seen_id).await {
                tracing::warn!(
                    error = %e,
                    "resync_after_reconnect failed; continuing with live stream",
                );
            }

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
                    recv = listener.recv() => match recv {
                        Ok(notification) => {
                            backoff = Duration::from_millis(BACKOFF_FLOOR_MS);
                            if let Some(new_id) = forward_notification(&pool, &tx, &notification, &cache).await {
                                last_seen_id = new_id;
                            }
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

        tracing::info!("PgListener exited cleanly");
    }

    /// After a (re)connect, replay any rows newer than `last_seen_id` into the
    /// broadcast channel. LISTEN/NOTIFY is not durable delivery: any events
    /// that fired while the listener was disconnected are lost, so we
    /// reconcile by reading the table directly.
    ///
    /// Returns `Err` if the SELECT fails; the caller logs and continues with
    /// the live stream rather than killing the listener.
    async fn resync_after_reconnect(
        pool: &PgPool,
        tx: &broadcast::Sender<SseEvent>,
        cache: &CacheHandle,
        last_seen_id: Uuid,
    ) -> sqlx::Result<()> {
        let rows = sqlx::query_as::<_, super::SearchResultRow>(
            "SELECT id, title, url, snippet, created_at \
             FROM search_results \
             WHERE id > $1 \
             ORDER BY id ASC \
             LIMIT 100",
        )
        .bind(last_seen_id)
        .fetch_all(pool)
        .await?;
        for row in rows {
            let event = SseEvent::SearchResult {
                title: Arc::from(row.title.as_str()),
                url: Arc::from(row.url.as_str()),
                snippet: Arc::from(row.snippet.as_str()),
            };
            if tx.send(event).is_err() {
                tracing::debug!("resync: no SSE receivers");
            }
            cache.invalidate_all();
        }
        Ok(())
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

        // Watchdog tests removed — the watchdog itself was deleted. The
        // "no notification for 90s" pattern conflated quiet databases with
        // dead connections. The replacement is `PgListener::try_recv()`,
        // which surfaces `ConnectionLost` directly.

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
    encode_cursor, run_pg_listener, search_with_cursor,
};

// Test-seam API — only available when the feature is enabled (e2e-tests).
#[cfg(all(feature = "ssr", feature = "test-seams"))]
pub use server::{PoolInitError, get_pool, set_pool};
