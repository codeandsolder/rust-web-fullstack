//! Server bootstrap — initialises all subsystems and starts the HTTP listener.
//!
//! This module is compiled only under `feature = "ssr"`. It is the single
//! call that sets up:
//!
//! - Tracing subscriber (plain `fmt` or with OpenTelemetry via `otel` feature).
//! - Database connection pool with connection hardening.
//! - Sqlx migrations on startup.
//! - Search-result cache (`moka`).
//! - SSE broadcast channel.
//! - `PgListener` background task + liveness watchdog.
//! - Leptos SSR application shell and routes.
//! - Axum HTTP server with graceful shutdown wiring.
//!
//! # dev-tools feature note
//! The `dev-tools` feature (behind `RUSTFLAGS="--cfg tokio_unstable"`) enables
//! `console-subscriber` for Tokio task inspection. Bake the env var into the
//! dev Docker image (Phase 2).

use std::net::SocketAddr;
#[cfg(feature = "otel")]
use std::sync::OnceLock;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::Context;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use axum::{
    Router,
    routing::{any, get},
};
use leptos::config::get_configuration;
use leptos_axum::{LeptosRoutes, generate_route_list};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

use crate::app;
use crate::events::SseEvent;
use crate::{cache, db, sse};

/// Handle returned by [`run`]. The caller uses the [`CancellationToken`] to
/// signal shutdown and the [`JoinSet`] / [`PgPool`] for graceful draining.
#[derive(Debug)]
#[must_use]
pub struct ServerHandle {
    /// Cancel this token to trigger graceful shutdown of all long-running
    /// tasks (HTTP server, `PgListener`, watchdog).
    pub shutdown: CancellationToken,
    /// Collection of background tasks. The caller should drain them after
    /// signalling shutdown.
    pub tasks: JoinSet<anyhow::Result<()>>,
    /// Database connection pool. The caller should call [`db::close_pool`] on
    /// it during the shutdown sequence.
    pub pool: sqlx::PgPool,
}

/// Global OTel provider, stored so `main` can force-flush / shutdown.
#[cfg(feature = "otel")]
static PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

/// Retrieve the OTel provider (if any) for shutdown.
#[cfg(feature = "otel")]
#[must_use]
pub fn get_tracer_provider() -> Option<&'static opentelemetry_sdk::trace::SdkTracerProvider> {
    PROVIDER.get()
}

/// Initialise the tracing subscriber.
///
/// When the `otel` feature is active and [`crate::otel::init_telemetry`]
/// succeeds, an `OTel` layer is added to the subscriber. Otherwise a plain
/// `fmt` subscriber is installed.
///
/// # Panics
/// Panics on the second call (tracing's global subscriber guard).
fn init_tracing() {
    // Try OTel first; fall back to plain fmt if OTel init fails or is
    // disabled.
    #[cfg(feature = "otel")]
    {
        match crate::otel::init_telemetry() {
            Ok(provider) => {
                let _ = PROVIDER.set(provider);
                return;
            }
            Err(e) => {
                // OTel init is best-effort; log and fall back to fmt.
                // We can't use `tracing::warn!` here because the subscriber
                // isn't set up yet — use eprintln instead.
                eprintln!("OTel init failed, falling back to fmt subscriber: {e}");
            }
        }
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,live_search=debug,tower_http=debug,sqlx=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .compact()
        .init();
}

/// Fallback handler (404).
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Server-function dispatch handler (see main.rs for the rationale behind the
/// doubled-prefix probe).
#[expect(
    clippy::expect_used,
    reason = "Path rewrite produces a valid URI by construction (prepending /api to a valid path)"
)]
async fn server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    let path_to_try =
        if leptos::server_fn::axum::get_server_fn_service(&original_path, method.clone()).is_none()
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
        parts.uri = Uri::try_from(&path_to_try).expect("valid URI from path rewrite");
    }

    let req = Request::from_parts(parts, body);
    leptos_axum::handle_server_fns(req).await
}

/// Bootstraps all subsystems and starts the HTTP server.
///
/// Returns a [`ServerHandle`] for graceful shutdown.
///
/// # Errors
/// Returns an error if the database URL is missing, pool creation fails,
/// migrations fail, or the TCP listener cannot bind.
pub async fn run() -> anyhow::Result<ServerHandle> {
    init_tracing();

    let database_url =
        std::env::var("DATABASE_URL").context("DATABASE_URL environment variable must be set")?;

    // ---- database pool & migration ---------------------------------------

    let pool = db::create_pool(&database_url)
        .await
        .context("failed to create database pool")?;

    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    db::set_pool(pool.clone())?;

    // ---- search cache -----------------------------------------------------

    cache::init_cache();
    tracing::debug!("search cache initialized");

    // ---- broadcast channel for SSE ----------------------------------------

    let (tx, _rx) = tokio::sync::broadcast::channel::<SseEvent>(256);
    sse::set_broadcast(tx.clone())?;

    // ---- cancellation token & task set ------------------------------------

    let shutdown = CancellationToken::new();
    let mut tasks = JoinSet::new();

    // ---- PgListener + watchdog --------------------------------------------

    let listener_token = shutdown.child_token();
    let watchdog_token = shutdown.child_token();

    let reconnect_requested = Arc::new(AtomicU64::new(0));
    let last_recv: Arc<Mutex<Option<Instant>>> = Arc::new(Mutex::new(None));

    let pool_for_listener = pool.clone();
    let listener_span = tracing::info_span!("pg_listener");
    let last_recv_for_listener = last_recv.clone();
    let reconnect_for_listener = reconnect_requested.clone();
    tasks.spawn(
        async move {
            db::run_pg_listener(
                pool_for_listener,
                tx,
                listener_token,
                reconnect_for_listener,
                last_recv_for_listener,
            )
            .await;
            Ok(())
        }
        .instrument(listener_span),
    );

    let watchdog_span = tracing::info_span!("pg_listener_watchdog");
    tasks.spawn(
        async move {
            db::run_watchdog(last_recv, reconnect_requested, watchdog_token).await;
            Ok(())
        }
        .instrument(watchdog_span),
    );

    // ---- Leptos configuration & routes ------------------------------------

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let leptos_routes = generate_route_list(app::App);

    // ---- Axum router ------------------------------------------------------

    #[allow(unused_mut)]
    let mut router = Router::new()
        .nest_service("/pkg", ServeDir::new("./pkg"))
        .route("/api/events", get(sse::sse_handler))
        .route("/api/{*fn_name}", any(server_fn_handler))
        .route("/api/api/{*fn_name}", any(server_fn_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options.clone())
        .leptos_routes(&leptos_options, leptos_routes, {
            let lo = leptos_options.clone();
            move || app::shell(lo.clone())
        })
        .fallback(fallback_handler);

    // Prometheus metrics endpoint — available when `otel` feature is active.
    #[cfg(feature = "otel")]
    {
        use axum_prometheus::PrometheusMetricLayer;

        let (prom_layer, metric_handle) = PrometheusMetricLayer::pair();
        router = router.layer(prom_layer).route(
            "/metrics",
            get(move || async move { metric_handle.render() }),
        );
    }

    let router: Router<()> = router.with_state(leptos_options);

    // ---- serve ------------------------------------------------------------

    let port: u16 = std::env::var("PORT")
        .or_else(|_| std::env::var("LIVE_SEARCH_PORT"))
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Live search server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind live-search listener on {addr}"))?;

    let graceful_shutdown_token = shutdown.clone();
    tasks.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                graceful_shutdown_token.cancelled().await;
            })
            .await
            .context("live-search server exited with an error")
    });

    Ok(ServerHandle {
        shutdown,
        tasks,
        pool,
    })
}
