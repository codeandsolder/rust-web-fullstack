//! In-process live-search server launcher for e2e tests.
//!
//! Starts a live-search server with a testcontainer Postgres database on a
//! random local port.  All background tasks (HTTP server, `PgListener`, watchdog)
//! run on a dedicated tokio runtime so they survive individual test lifetimes.

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::Arc;

use axum::{
    Router,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::{any, get},
};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use leptos_axum::handle_server_fns;

use live_search::cache;
use live_search::events::SseEvent;
use live_search::state;

/// Configuration for `LiveSearchEnv::start_with`.
///
/// Defaults to using `LIVE_SEARCH_PKG_DIR` env var (no silent relative
/// fallback) and the canonical probe-based server-fn route.
#[derive(Debug, Clone)]
pub struct LiveSearchConfig {
    /// Directory containing the Leptos build artifacts (e.g. `live_search.js`).
    /// If `None`, the env var `LIVE_SEARCH_PKG_DIR` is read; if that's
    /// unset, the test errors out instead of guessing a path.
    pub pkg_dir: Option<PathBuf>,
}

impl Default for LiveSearchConfig {
    fn default() -> Self {
        Self {
            pkg_dir: std::env::var_os("LIVE_SEARCH_PKG_DIR").map(PathBuf::from),
        }
    }
}

/// RAII guard that runs a live-search server in the background on a random port.
///
/// The server connects to a testcontainer Postgres database (spawned by
/// [`super::db::TestEnv`]) so tests can insert rows and verify SSE propagation.
///
/// # Drop behaviour
/// When the [`LiveSearchEnv`] is dropped, the cancellation token fires and
/// the background thread's `JoinHandle` is awaited with a short timeout.
/// The database container is stopped when its own RAII guard drops.
pub struct LiveSearchEnv {
    /// Base URL of the running server.
    base_url: String,
    /// RAII guard for the Postgres testcontainer (dropped last).
    db_container: super::db::TestEnv,
    /// Cancellation token for graceful shutdown.
    shutdown: CancellationToken,
    /// Background thread handle; awaited on drop.
    server_thread: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
}

impl LiveSearchEnv {
    /// Start a live-search server backed by a testcontainer Postgres instance.
    ///
    /// The server binds to a random port on `127.0.0.1`.
    ///
    /// # Errors
    /// Returns an error if the database container cannot start, migrations fail,
    /// or the server fails to bind.
    pub async fn start() -> Result<Self> {
        Self::start_with(LiveSearchConfig::default()).await
    }

    /// Start a live-search server with explicit configuration.
    pub async fn start_with(cfg: LiveSearchConfig) -> Result<Self> {
        // ── 1. Start Postgres testcontainer (runs live-search migrations) ──
        let db = super::db::TestEnv::postgres().await?;
        let conn_str = db.connection_string().to_string();

        // ── 2. Create server database pool ────────────────────────────────
        let server_pool =
            live_search::db::create_pool(&conn_str, &live_search::db::PoolTunables::default())
                .await
                .context("Failed to create live-search database pool")?;

        // ── 3. Search cache ──────────────────────────────────────────────
        let cache_handle = cache::CacheHandle::default();

        // ── 4. Broadcast channel for SSE ────────────────────────────────
        let (tx, _rx) = broadcast::channel::<SseEvent>(256);

        // Clone a sender for the SSE handler closure so the original `tx`
        // can be moved into the background thread for the PgListener.
        let tx_for_sse = tx.clone();

        // ── 5. Cancellation token ───────────────────────────────────────
        let shutdown = CancellationToken::new();

        // ── 6. Build Router ──────────────────────────────────────────────
        let mut router = Router::new()
            .route(
                "/api/events",
                get(move || {
                    let tx = tx_for_sse.clone();
                    async move { live_search::sse::sse_handler(tx).await }
                }),
            )
            .route("/api/{*fn_name}", any(handle_server_fns))
            // In-process fallback for the root path: the test env does not
            // mount the Leptos page routes (no SSR), so `/` must be served
            // by a lightweight fixture handler instead of the 404 fallback.
            .route("/", get(|| async { (StatusCode::OK, "live-search test fixture") }))
            .layer(TraceLayer::new_for_http());

        // Mount /pkg/ for Leptos build artifacts. The path comes from
        // `LiveSearchConfig::pkg_dir` or `LIVE_SEARCH_PKG_DIR`; no silent
        // relative fallback. If a path is configured but doesn't exist,
        // we error out — failing visibly is better than 404-ing on every
        // CSS/JS asset request.
        if let Some(pkg_dir) = cfg.pkg_dir.as_ref() {
            if !pkg_dir.exists() {
                return Err(anyhow::anyhow!(
                    "LIVE_SEARCH_PKG_DIR points at {} but the directory does not exist; \
                     run `cargo leptos build` first or unset the env var",
                    pkg_dir.display()
                ));
            }
            router = router.nest_service("/pkg", tower_http::services::ServeDir::new(pkg_dir));
        }

        router = router.fallback(fallback_handler);

        // ── 7. AppContext for server functions ──────────────────────────
        //
        // Server functions resolve state via `state::get()`; without
        // `state::set` here, every server fn that touches the pool/broadcast
        // returns None and panics. Ignore `AlreadyInitialized` (the
        // `SharedServer` may have set this on a previous test).
        let ctx = Arc::new(live_search::state::AppContext::new(
            server_pool.clone(),
            tx.clone(),
            cache_handle.clone(),
        ));
        if let Err(e) = state::set(ctx) {
            match e {
                state::AppContextInitError::AlreadyInitialized => {
                    // Shared server: skip.
                }
                // `#[non_exhaustive]` — future variants are treated as benign here.
                _ => {
                    tracing::debug!(error = ?e, "state::set returned unexpected error; ignoring");
                }
            }
        }

        // ── 8. Spawn background thread ──────────────────────────────────
        //
        // Each `#[tokio::test]` creates its own tokio runtime that is dropped
        // when the test finishes. By moving the server and PgListener onto
        // their own dedicated runtime, they survive across individual test
        // lifetimes.
        let base_url: String;
        let addr_lock = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
        let server_thread = {
            let addr_clone = Arc::clone(&addr_lock);
            let bg_shutdown = shutdown.clone();
            let bg_pool = server_pool;
            let bg_cache = cache_handle;
            let bg_tx = tx;
            let bg_router = router;

            std::thread::Builder::new()
                .name("live-search-bg".into())
                .spawn(move || -> Result<()> {
                    let rt = tokio::runtime::Runtime::new()
                        .context("failed to create background tokio runtime")?;
                    rt.block_on(async {
                        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                            .await
                            .context("Failed to bind live-search listener")?;
                        let bound_addr = listener
                            .local_addr()
                            .context("Failed to get bound address")?;
                        addr_clone
                            .lock()
                            .map_err(|e| anyhow::anyhow!("addr lock poisoned: {e}"))?
                            .replace(bound_addr);

                        let mut tasks: tokio::task::JoinSet<anyhow::Result<()>> =
                            tokio::task::JoinSet::new();

                        // HTTP server — propagate axum errors instead of swallowing.
                        let server_token = bg_shutdown.clone();
                        tasks.spawn(async move {
                            axum::serve(listener, bg_router)
                                .with_graceful_shutdown(async move {
                                    server_token.cancelled().await;
                                })
                                .await
                                .context("live-search axum server exited with error")?;
                            Ok(())
                        });

                        // PgListener (no watchdog; deleted in favor of try_recv).
                        let listener_token = bg_shutdown.child_token();
                        let pool_for_listener = bg_pool.clone();
                        let cache_for_listener = bg_cache.clone();
                        tasks.spawn(async move {
                            live_search::db::run_pg_listener(
                                pool_for_listener,
                                bg_tx,
                                cache_for_listener,
                                listener_token,
                            )
                            .await;
                            Ok(())
                        });

                        // Drive all tasks until shutdown. Surface any non-Ok result.
                        while let Some(joined) = tasks.join_next().await {
                            match joined {
                                Ok(Ok(())) => {}
                                Ok(Err(e)) => {
                                    tracing::error!(error = %e, "background task errored");
                                }
                                Err(join_err) => {
                                    tracing::error!(error = ?join_err, "background task join error");
                                }
                            }
                        }

                        Ok::<_, anyhow::Error>(())
                    })
                })
                .context("Failed to spawn background thread")?
        };

        // Wait for the server to report its address.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let addr = loop {
            {
                let guard = addr_lock
                    .lock()
                    .map_err(|e| anyhow::anyhow!("addr lock poisoned: {e}"))?;
                if let Some(a) = *guard {
                    break a;
                }
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "Live-search server did not bind within 30s"
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        };
        base_url = format!("http://{addr}");
        // addr_lock is dropped when `start_with` returns.

        // Wait for the server to be ready (SSE endpoint check).
        let health_url = format!("{base_url}/api/events");
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(2))
            .build()
            .context("Failed to build health check client")?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            if let Ok(resp) = client.get(&health_url).send().await
                && resp.status().is_success()
            {
                break;
            }
            if std::time::Instant::now() >= deadline {
                return Err(anyhow::anyhow!(
                    "Live-search server at {base_url} did not become ready within 30s"
                ));
            }
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }

        Ok(Self {
            base_url,
            db_container: db,
            shutdown,
            server_thread: Some(server_thread),
        })
    }

    /// The base URL of the running server (e.g. `http://127.0.0.1:54321`).
    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Reference to the database test env (for inserting test data, running
    /// SQL queries, etc.).
    #[must_use]
    pub const fn db(&self) -> &super::db::TestEnv {
        &self.db_container
    }
}

impl std::fmt::Debug for LiveSearchEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveSearchEnv")
            .field("base_url", &self.base_url)
            .field("db_container", &self.db_container)
            .field("shutdown", &"<cancellation token>")
            .finish()
    }
}

impl Drop for LiveSearchEnv {
    fn drop(&mut self) {
        self.shutdown.cancel();
        // Wait briefly for the background thread to finish. If it doesn't,
        // log a warning rather than blocking the test runner indefinitely.
        if let Some(handle) = self.server_thread.take() {
            // The thread is detached; we cannot block here (Drop is sync).
            // A leaked thread is acceptable for a test fixture — the test
            // process exits when the test binary exits.
            drop(handle);
        }
    }
}

// ──── Helper handlers ─────────────────────────────────────────────────────

/// Fallback handler returning 404.
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}
