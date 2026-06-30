//! In-process live-search server launcher for e2e tests.
//!
//! Starts a live-search server with a testcontainer Postgres database on a
//! random local port.  All background tasks (HTTP server, `PgListener`, watchdog)
//! run on a dedicated tokio runtime so they survive individual test lifetimes.

#![allow(
    dead_code,
    reason = "Some helpers unused per test-binary compilation"
)]

use anyhow::{Context, Result};
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::Mutex;
use std::time::Instant;

use axum::{
    Router,
    body::Body,
    extract::Request,
    http::{StatusCode, Uri},
    response::IntoResponse,
    routing::{any, get},
};
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing::Instrument;

use live_search::events::SseEvent;

/// RAII guard that runs a live-search server in the background on a random port.
///
/// The server connects to a testcontainer Postgres database (spawned by
/// [`super::db::TestEnv`]) so tests can insert rows and verify SSE propagation.
///
/// # Drop behaviour
/// When the [`LiveSearchEnv`] is dropped, the background tasks (HTTP server,
/// `PgListener`, watchdog) are cancelled and the database container is stopped.
pub struct LiveSearchEnv {
    /// Base URL of the running server.
    base_url: String,
    /// RAII guard for the Postgres testcontainer (dropped last).
    db_container: super::db::TestEnv,
    /// Cancellation token for graceful shutdown.
    shutdown: CancellationToken,
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
        // ── 1. Start Postgres testcontainer (runs live-search migrations) ──
        let db = super::db::TestEnv::postgres().await?;
        let conn_str = db.connection_string().to_string();

        // ── 2. Create server database pool ────────────────────────────────
        let server_pool = live_search::db::create_pool(&conn_str)
            .await
            .context("Failed to create live-search database pool")?;

        // ── 3. Set global pool (OnceLock) ─────────────────────────────────
        live_search::db::set_pool(server_pool.clone())
            .context("set_pool already initialized")?;

        // ── 4. Initialise search cache (OnceLock) ─────────────────────────
        live_search::cache::init_cache();

        // ── 5. Broadcast channel for SSE (OnceLock) ───────────────────────
        let (tx, _rx) = broadcast::channel::<SseEvent>(256);
        live_search::sse::set_broadcast(tx.clone())
            .context("set_broadcast already initialized")?;

        // ── 6. Cancellation token ─────────────────────────────────────────
        let shutdown = CancellationToken::new();

        // PgListener state (Arc'd for thread safety)
        let reconnect_requested = Arc::new(AtomicU64::new(0));
        let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

        // ── 7. Build Router ───────────────────────────────────────────────
        let mut router = Router::new()
            .route("/api/events", get(live_search::sse::sse_handler))
            .route("/api/{*fn_name}", any(server_fn_handler))
            .route("/api/api/{*fn_name}", any(server_fn_handler))
            .layer(TraceLayer::new_for_http());

        // Mount /pkg/ for Leptos build artifacts if available.
        let pkg_dir = std::path::Path::new("../live-search/pkg");
        if pkg_dir.exists() {
            router = router.nest_service(
                "/pkg",
                tower_http::services::ServeDir::new(pkg_dir),
            );
            let pkg_abs = pkg_dir.canonicalize().unwrap_or_else(|_| pkg_dir.to_path_buf());
            tracing::info!("Mounted /pkg/ from {}", pkg_abs.display());
        }

        router = router.fallback(fallback_handler);

        // ── 8. Spawn all background tasks on a persistent runtime thread ──
        //
        // Each `#[tokio::test]` creates its own tokio runtime that is dropped
        // when the test finishes.  By moving the server, PgListener, and
        // watchdog onto their own dedicated runtime, they survive across
        // individual test lifetimes.
        //
        // Binding is done INSIDE the background runtime so the TcpListener
        // is registered with that runtime's reactor, not the test runtime's.
        let base_url: String;
        {
            let addr_lock = Arc::new(std::sync::Mutex::new(None::<SocketAddr>));
            let addr_clone = Arc::clone(&addr_lock);
            let bg_shutdown = shutdown.clone();
            let bg_pool = server_pool;
            let bg_tx = tx;
            let bg_reconnect = reconnect_requested;
            let bg_last_recv = last_recv;
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

                        let mut tasks: JoinSet<anyhow::Result<()>> = JoinSet::new();

                        // ── HTTP server ───────────────────────────────
                        let server_token = bg_shutdown.clone();
                        tasks.spawn(async move {
                            axum::serve(listener, bg_router)
                                .with_graceful_shutdown(async move {
                                    server_token.cancelled().await;
                                })
                                .await
                                .ok();
                            Ok(())
                        });

                        // ── PgListener ────────────────────────────────
                        let listener_token = bg_shutdown.child_token();
                        let pool_for_listener = bg_pool.clone();
                        let last_recv_for_listener = bg_last_recv.clone();
                        let reconnect_for_listener = bg_reconnect.clone();
                        let listener_span = tracing::info_span!("pg_listener");
                        tasks.spawn(
                            async move {
                                live_search::db::run_pg_listener(
                                    pool_for_listener,
                                    bg_tx,
                                    listener_token,
                                    reconnect_for_listener,
                                    last_recv_for_listener,
                                )
                                .await;
                                Ok(())
                            }
                            .instrument(listener_span),
                        );

                        // ── Watchdog ──────────────────────────────────
                        let watchdog_token = bg_shutdown.child_token();
                        let watchdog_span = tracing::info_span!("pg_listener_watchdog");
                        tasks.spawn(
                            async move {
                                live_search::db::run_watchdog(
                                    bg_last_recv,
                                    bg_reconnect,
                                    watchdog_token,
                                )
                                .await;
                                Ok(())
                            }
                            .instrument(watchdog_span),
                        );

                        // Drive all tasks until shutdown
                        while tasks.join_next().await.is_some() {}

                        Ok::<_, anyhow::Error>(())
                    })
                })
                .context("Failed to spawn background thread")?;

            // Wait for the server to report its address.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
            let addr = loop {
                {
                    let guard = addr_lock.lock().map_err(|e| anyhow::anyhow!("addr lock poisoned: {e}"))?;
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
        }

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
    }
}

// ──── Helper handlers ─────────────────────────────────────────────────────

/// Fallback handler returning 404.
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Server-function dispatch handler (Pattern 9 from the rust-web-fullstack
/// skill).  Probes the exact path first; if not found, tries a doubled-prefix
/// variant (e.g. `/api/search` when `#[server(endpoint = "/api/search")]`
/// registered `/api/api/search`).
async fn server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    let path_to_try = if leptos::server_fn::axum::get_server_fn_service(&original_path, method.clone()).is_none()
        && original_path.starts_with("/api/")
    {
        let doubled = format!("/api{original_path}");
        if leptos::server_fn::axum::get_server_fn_service(&doubled, method).is_some() {
            doubled
        } else {
            original_path
        }
    } else {
        original_path
    };

    if path_to_try != parts.uri.path() {
        // Path is derived from an existing valid URI; construction only fails
        // for truly invalid inputs (null bytes, etc.) which cannot occur here.
        if let Ok(uri) = Uri::try_from(&path_to_try) {
            parts.uri = uri;
        }
    }

    let req = Request::from_parts(parts, body);
    leptos_axum::handle_server_fns(req).await
}
