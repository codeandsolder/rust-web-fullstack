//! In-process live-search server launcher for e2e tests.
//!
//! The fixture uses the production Leptos application routes and SSR shell,
//! backed by an isolated Postgres testcontainer. Only the listener address and
//! injected application state differ from production.

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
use leptos::config::get_configuration;
use leptos_axum::{LeptosRoutes, generate_route_list, handle_server_fns};
use tokio::sync::broadcast;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;

use live_search::cache;
use live_search::events::SseEvent;
use live_search::state;

#[derive(Debug, Clone)]
pub struct LiveSearchConfig {
    /// Directory containing cargo-leptos build artifacts.
    pub pkg_dir: Option<PathBuf>,
}

impl Default for LiveSearchConfig {
    fn default() -> Self {
        Self {
            pkg_dir: std::env::var_os("LIVE_SEARCH_PKG_DIR").map(PathBuf::from),
        }
    }
}

pub struct LiveSearchEnv {
    base_url: String,
    db_container: super::db::TestEnv,
    shutdown: CancellationToken,
    server_thread: Option<std::thread::JoinHandle<anyhow::Result<()>>>,
}

impl LiveSearchEnv {
    /// Start the production live-search route tree on a random local port.
    ///
    /// # Errors
    /// Returns an error if Postgres, Leptos configuration, build assets,
    /// application state, binding, or readiness checks fail.
    pub async fn start() -> Result<Self> {
        Self::start_with(LiveSearchConfig::default()).await
    }

    pub async fn start_with(cfg: LiveSearchConfig) -> Result<Self> {
        let db = super::db::TestEnv::postgres().await?;
        let conn_str = db.connection_string().to_string();
        let server_pool =
            live_search::db::create_pool(&conn_str, &live_search::db::PoolTunables::default())
                .await
                .context("Failed to create live-search database pool")?;

        let cache_handle = cache::CacheHandle::default();
        let (tx, _rx) = broadcast::channel::<SseEvent>(256);
        let tx_for_sse = tx.clone();
        let shutdown = CancellationToken::new();

        // Server functions use the same AppContext mechanism as production.
        // SharedServer constructs this fixture once per integration-test binary,
        // so an existing value indicates a broken duplicate bootstrap rather
        // than something to silently ignore.
        let ctx = Arc::new(live_search::state::AppContext::new(
            server_pool.clone(),
            tx.clone(),
            cache_handle.clone(),
        ));
        state::set(Arc::clone(&ctx)).context("live-search AppContext already initialized")?;

        let conf = get_configuration(None).context("failed to load Leptos configuration")?;
        let leptos_options = conf.leptos_options;
        let leptos_routes = generate_route_list(live_search::app::App);

        let mut router = Router::new()
            .route(
                "/api/events",
                get(move || {
                    let tx = tx_for_sse.clone();
                    async move { live_search::sse::sse_handler(tx).await }
                }),
            )
            .route("/api/{*fn_name}", any(handle_server_fns));

        // Browser tests are real hydration tests, so missing client artifacts
        // are a fixture error rather than a reason to serve SSR-only HTML.
        let pkg_dir = cfg.pkg_dir.ok_or_else(|| {
            anyhow::anyhow!(
                "LIVE_SEARCH_PKG_DIR must point to cargo-leptos site/pkg for browser E2E tests"
            )
        })?;
        if !pkg_dir.exists() {
            return Err(anyhow::anyhow!(
                "LIVE_SEARCH_PKG_DIR points at {} but the directory does not exist",
                pkg_dir.display()
            ));
        }
        router = router.nest_service("/pkg", tower_http::services::ServeDir::new(pkg_dir));

        let ctx_for_shell = Arc::clone(&ctx);
        let shell_options = leptos_options.clone();
        let router = router
            .with_state(leptos_options.clone())
            .leptos_routes(&leptos_options, leptos_routes, move || {
                leptos::context::provide_context(Arc::clone(&ctx_for_shell));
                live_search::app::shell(shell_options.clone())
            })
            .route("/health", get(|| async { (StatusCode::OK, "ok") }))
            .fallback(fallback_handler)
            // Router::layer only covers routes already present; keep global
            // tracing last just like production bootstrap.
            .layer(TraceLayer::new_for_http());
        let router: Router<()> = router.with_state(leptos_options);

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
        let base_url = format!("http://{addr}");

        let health_url = format!("{base_url}/health");
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

    #[must_use]
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

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
        if let Some(handle) = self.server_thread.take() {
            drop(handle);
        }
    }
}

async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}
