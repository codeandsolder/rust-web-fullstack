//! Database types, pool management, and `PostgreSQL` LISTEN/NOTIFY integration.
//!
//! The SSR binary uses a global [`sqlx::PgPool`] test seam and a background
//! listener task that subscribes to the `search_results` channel and forwards
//! notifications into a [`tokio::sync::broadcast::Sender`] consumed by the SSE
//! handler. LISTEN/NOTIFY is used only as the low-latency wakeup path; reconnect
//! recovery is driven by the durable monotonic `event_seq` column.

#[cfg(feature = "ssr")]
use chrono::{DateTime, Utc};
#[cfg(feature = "ssr")]
use uuid::Uuid;

/// Re-export the canonical domain type.
pub use rwf_domain::SearchResult;

/// Row-level representation for `sqlx` queries, mirroring the public database
/// columns used by search results.
#[cfg(feature = "ssr")]
#[derive(Debug, sqlx::FromRow)]
#[must_use]
#[doc(hidden)]
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

#[cfg(feature = "ssr")]
mod server {
    use std::sync::Arc;
    use std::time::Duration;

    #[cfg(feature = "test-seams")]
    use std::sync::OnceLock;

    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use chrono::{DateTime, Utc};
    use serde::Deserialize;
    use sqlx::PgPool;
    use sqlx::postgres::{PgListener, PgPoolOptions};
    use tokio::sync::broadcast;
    use tokio_util::sync::CancellationToken;
    use uuid::Uuid;

    use crate::cache::CacheHandle;
    use crate::events::SseEvent;

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
    /// # Errors
    /// Returns [`PoolInitError::AlreadyInitialized`] if startup tries to set
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

    #[derive(Debug, sqlx::FromRow)]
    struct NotificationRow {
        id: Uuid,
        title: String,
        url: String,
        snippet: String,
        event_seq: i64,
    }

    async fn connect_and_listen(pool: &PgPool) -> Result<PgListener, sqlx::Error> {
        let mut listener = PgListener::connect_with(pool).await?;
        listener.listen("search_results").await?;
        Ok(listener)
    }

    const PAYLOAD_LOG_PREVIEW_BYTES: usize = 200;
    const RESYNC_BATCH_SIZE: usize = 100;
    const RESYNC_BATCH_SIZE_SQL: i64 = 100;

    fn broadcast_row(
        tx: &broadcast::Sender<SseEvent>,
        row: &NotificationRow,
        context: &'static str,
    ) {
        let event = SseEvent::SearchResult {
            title: Arc::from(row.title.as_str()),
            url: Arc::from(row.url.as_str()),
            snippet: Arc::from(row.snippet.as_str()),
        };
        if let Ok(receivers) = tx.send(event) {
            tracing::debug!(
                row_id = %row.id,
                event_seq = row.event_seq,
                receivers,
                context,
                "forwarded search result event"
            );
        } else {
            tracing::debug!(
                row_id = %row.id,
                event_seq = row.event_seq,
                context,
                "search result event had no SSE receivers"
            );
        }
    }

    /// Forward one live notification unless reconnect replay has already
    /// delivered the row.
    async fn forward_notification(
        pool: &PgPool,
        tx: &broadcast::Sender<SseEvent>,
        notification: &sqlx::postgres::PgNotification,
        cache: &CacheHandle,
        last_seen_seq: i64,
    ) -> Result<Option<i64>, sqlx::Error> {
        let payload = notification.payload();
        let id = match serde_json::from_str::<SearchResultNotification>(payload) {
            Ok(row) => row.id,
            Err(e) => {
                let preview: String = payload.chars().take(PAYLOAD_LOG_PREVIEW_BYTES).collect();
                tracing::error!(
                    channel = %notification.channel(),
                    payload_len = payload.len(),
                    payload_preview = %preview,
                    error = %e,
                    "invalid search_results notification payload"
                );
                return Ok(None);
            }
        };

        let row = sqlx::query_as::<_, NotificationRow>(
            "SELECT id, title, url, snippet, event_seq \
             FROM search_results WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await?;

        let Some(row) = row else {
            tracing::warn!(
                notification_id = %id,
                "search_results notification referenced a row that no longer exists"
            );
            return Ok(None);
        };

        // Notifications that arrived while a reconnect replay was running are
        // queued by PostgreSQL. The replay may already have emitted the same
        // row, so use the durable sequence to suppress that duplicate.
        if row.event_seq <= last_seen_seq {
            tracing::debug!(
                row_id = %row.id,
                event_seq = row.event_seq,
                last_seen_seq,
                "skipping notification already covered by reconnect replay"
            );
            return Ok(None);
        }

        broadcast_row(tx, &row, "notify");
        cache.invalidate_all();
        Ok(Some(row.event_seq))
    }

    async fn sleep_or_shutdown(dur: Duration, shutdown: &CancellationToken) -> bool {
        tokio::select! {
            () = shutdown.cancelled() => true,
            () = tokio::time::sleep(dur) => false,
        }
    }

    /// Listen on the `search_results` channel and forward notifications.
    ///
    /// LISTEN/NOTIFY is not durable delivery. After every successful connect,
    /// the listener pages through rows with `event_seq > last_seen_seq`, then
    /// resumes the live stream. The monotonic sequence avoids `UUIDv4` ordering
    /// bugs, the pagination avoids the old 100-row truncation, and advancing
    /// the cursor avoids replaying the same rows on consecutive reconnects.
    #[tracing::instrument(skip_all)]
    pub async fn run_pg_listener(
        pool: PgPool,
        tx: broadcast::Sender<SseEvent>,
        cache: CacheHandle,
        shutdown: CancellationToken,
    ) {
        let mut backoff = Duration::from_millis(250);
        let max_backoff = Duration::from_secs(30);
        const BACKOFF_FLOOR_MS: u64 = 250;
        let mut last_seen_seq: i64 = 0;

        while !shutdown.is_cancelled() {
            let mut listener = match connect_and_listen(&pool).await {
                Ok(listener) => {
                    tracing::info!("Listening on search_results channel");
                    backoff = Duration::from_millis(BACKOFF_FLOOR_MS);
                    listener
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

            match resync_after_reconnect(&pool, &tx, &cache, last_seen_seq).await {
                Ok(seq) => last_seen_seq = seq,
                Err(e) => tracing::warn!(
                    error = %e,
                    last_seen_seq,
                    "reconnect replay failed; continuing with live stream"
                ),
            }

            loop {
                tokio::select! {
                    biased;
                    () = shutdown.cancelled() => {
                        tracing::info!("PgListener shutting down");
                        return;
                    }
                    recv = listener.recv() => match recv {
                        Ok(notification) => {
                            backoff = Duration::from_millis(BACKOFF_FLOOR_MS);
                            match forward_notification(
                                &pool,
                                &tx,
                                &notification,
                                &cache,
                                last_seen_seq,
                            ).await {
                                Ok(Some(seq)) => last_seen_seq = seq,
                                Ok(None) => {}
                                Err(e) => tracing::error!(
                                    error = %e,
                                    "failed to fetch notified search result"
                                ),
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
                            break;
                        }
                    }
                }
            }
        }

        tracing::info!("PgListener exited cleanly");
    }

    /// Replay every row newer than `last_seen_seq`, in bounded batches.
    async fn resync_after_reconnect(
        pool: &PgPool,
        tx: &broadcast::Sender<SseEvent>,
        cache: &CacheHandle,
        last_seen_seq: i64,
    ) -> sqlx::Result<i64> {
        let mut cursor = last_seen_seq;
        let mut replayed_any = false;

        loop {
            let rows = sqlx::query_as::<_, NotificationRow>(
                "SELECT id, title, url, snippet, event_seq \
                 FROM search_results \
                 WHERE event_seq > $1 \
                 ORDER BY event_seq ASC \
                 LIMIT $2",
            )
            .bind(cursor)
            .bind(RESYNC_BATCH_SIZE_SQL)
            .fetch_all(pool)
            .await?;

            let batch_len = rows.len();
            for row in rows {
                broadcast_row(tx, &row, "reconnect-resync");
                cursor = row.event_seq;
                replayed_any = true;
            }

            if batch_len < RESYNC_BATCH_SIZE {
                break;
            }
        }

        if replayed_any {
            cache.invalidate_all();
        }
        Ok(cursor)
    }

    /// Search results with cursor-based pagination.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] if the database query fails.
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
    #[must_use]
    pub fn base64url_encode(input: &[u8]) -> String {
        URL_SAFE_NO_PAD.encode(input)
    }

    /// Decode a base64url string back to bytes.
    ///
    /// # Errors
    /// Returns an error string if the input contains invalid base64url
    /// characters.
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
    /// Returns an error string if decoding or parsing fails.
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

    #[cfg(test)]
    mod tests {
        use super::*;

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
            assert!(super::decode_cursor("!!!notbase64!!!").is_err());
        }
    }
}

#[cfg(feature = "ssr")]
pub use server::{
    PoolTunables, base64url_decode, base64url_encode, close_pool, create_pool, decode_cursor,
    encode_cursor, run_pg_listener, search_with_cursor,
};

#[cfg(all(feature = "ssr", feature = "test-seams"))]
pub use server::{PoolInitError, get_pool, set_pool};
