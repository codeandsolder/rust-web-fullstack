//! SSR server binary for the live-search application.
//!
//! Sets up:
//! - Database connection pool and `PostgreSQL` `LISTEN`/`NOTIFY` background task.
//! - Axum HTTP server with Leptos SSR routes, SSE endpoint, and static assets.
//! - Graceful shutdown via `CancellationToken` on `Ctrl+C` / `SIGTERM`.

#![cfg(feature = "ssr")]

use std::net::SocketAddr;
use std::time::Duration;

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
use tokio::signal;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

use live_search::app;
use live_search::events::SseEvent;
use live_search::{db, sse};

/// Fallback handler that does not use `State<S>` – works with `Router<()>`.
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Server-function dispatch handler.
///
/// Leptos 0.8's `#[server(endpoint = "/api/search")]` macro registers each
/// server fn at the **doubled** path `/api/api/search` (the default
/// `/api` prefix concatenated with the explicit endpoint). The compile-time
/// `server_fn` client knows only the original path `/api/search`, so requests
/// from the browser will 404 on `leptos_axum::handle_server_fns` unless we
/// probe for the doubled-prefix variant and rewrite the request URI in place.
///
/// This handler does exactly that probe-then-rewrite and then forwards the
/// (possibly rewritten) request to `handle_server_fns`.
///
/// # Panics
/// Panics only if the path-rewrite produces an invalid URI — in practice this
/// is infallible because we only ever prepend `/api` to an existing valid URI.
#[expect(
    clippy::expect_used,
    reason = "Path rewrite produces a valid URI by construction (prepending /api to a valid path)"
)]
async fn server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    // Probe the registry: is the server fn at the original path or the
    // doubled-prefix path? Rewrite the URI accordingly.
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

/// Spawn a task that fires the shutdown token on Ctrl+C (all platforms) or
/// SIGTERM (Unix). The token is observed by every clone/child, propagating
/// shutdown to all long-running tasks.
fn spawn_signal_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        #[expect(
            clippy::expect_used,
            reason = "signal handler installation can only fail in unrecoverable runtime states"
        )]
        let ctrl_c = async {
            signal::ctrl_c()
                .await
                .expect("failed to install Ctrl+C handler");
        };
        #[cfg(unix)]
        let terminate = async {
            #[expect(
                clippy::expect_used,
                reason = "signal handler installation can only fail in unrecoverable runtime states"
            )]
            let mut sig = signal::unix::signal(signal::unix::SignalKind::terminate())
                .expect("failed to install SIGTERM handler");
            sig.recv().await;
        };
        #[cfg(not(unix))]
        let terminate = std::future::pending::<()>();

        tokio::select! {
            () = ctrl_c => tracing::info!("Ctrl+C received, initiating shutdown"),
            () = terminate => tracing::info!("SIGTERM received, initiating shutdown"),
        }
        shutdown.cancel();
    });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            EnvFilter::new("info,live_search=debug,tower_http=debug,sqlx=warn")
        }))
        .init();

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

    // ---- broadcast channel for SSE ----------------------------------------

    let (tx, _rx) = tokio::sync::broadcast::channel::<SseEvent>(256);
    sse::set_broadcast(tx.clone())?;

    // ---- Structured concurrency: CancellationToken + JoinSet + select! ----
    //
    // One shutdown signal propagates to every long-running task:
    //   * `pg_listener_task` exits via `shutdown.cancelled()`
    //   * `axum::serve` exits via the same token in graceful shutdown
    //
    // Dropping `JoinSet` aborts its tasks, but we drain it explicitly with a
    // grace period so in-flight requests can complete cleanly.

    let shutdown = CancellationToken::new();
    spawn_signal_handler(shutdown.clone());

    let mut tasks = JoinSet::new();
    let listener_token = shutdown.child_token();
    // Attach the span before spawning so logs inside `run_pg_listener` carry
    // the `pg_listener` span (rust-tracing §1.1 — `#[instrument]` alone does
    // not propagate across `JoinSet::spawn`).
    let listener_span = tracing::info_span!("pg_listener");
    tasks.spawn(
        async move { db::run_pg_listener(pool, tx, listener_token).await }
            .instrument(listener_span),
    );

    // ---- Leptos configuration & routes ------------------------------------

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(app::App);

    // ---- Axum router ------------------------------------------------------
    //
    // Build a stateful router `Router<LeptosOptions>` so the `LeptosRoutes`
    // trait bound `LeptosOptions: FromRef<S>` is satisfied (the blanket impl
    // `T: Clone ⇒ FromRef<T> for T` applies).  The handlers registered by
    // `leptos_routes` capture their own `LeptosOptions` copy via closures and
    // do **not** use Axum's `State` extractor, so we can freely switch to
    // `Router<()>` afterwards.
    //
    // Both `/api/{*fn_name}` and `/api/api/{*fn_name}` are registered because
    // the `#[server(endpoint = "/api/…")]` macro may register the function
    // under the doubled prefix.

    let app = Router::new()
        // ---- static assets (hydration JS/WASM, CSS) --------------------------
        //
        // Must be before .leptos_routes() so /pkg/* takes priority.
        .nest_service("/pkg", ServeDir::new("./pkg"))
        .route("/api/events", get(sse::sse_handler))
        .route("/api/{*fn_name}", any(server_fn_handler))
        .route("/api/api/{*fn_name}", any(server_fn_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(leptos_options.clone())
        .leptos_routes(&leptos_options, routes, {
            let lo = leptos_options.clone();
            move || app::shell(lo.clone())
        })
        .fallback(fallback_handler);

    // Convert to `Router<()>` – the only router type that implements the
    // tower `Service` traits needed by `axum::serve`.
    // We must pass the current state value to satisfy the signature; the
    // output state type `S2` is inferred as `()`.
    let app: Router<()> = app.with_state(leptos_options);

    // ---- serve ------------------------------------------------------------

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3000);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Live search server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind live-search listener on {addr}"))?;

    // Serve with graceful shutdown so in-flight SSE handlers can drain.
    let graceful_shutdown_token = shutdown.clone();
    axum::serve(listener, app)
        .with_graceful_shutdown(async move {
            graceful_shutdown_token.cancelled().await;
        })
        .await
        .context("live-search server exited with an error")?;

    // Drain background tasks with a grace period so in-flight notifications
    // can flush. A second `cancel()` is idempotent and safe if the signal
    // handler already fired. Inspect each `JoinError` so a panic is logged
    // rather than silently swallowed.
    shutdown.cancel();
    match tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(joined) = tasks.join_next().await {
            if let Err(err) = joined {
                tracing::error!(
                    error = ?err,
                    is_panic = err.is_panic(),
                    "background task did not complete cleanly"
                );
            }
        }
    })
    .await
    {
        Ok(()) => {}
        Err(_elapsed) => {
            tracing::warn!("background tasks did not drain within 10s; aborting");
            tasks.abort_all();
        }
    }

    Ok(())
}
