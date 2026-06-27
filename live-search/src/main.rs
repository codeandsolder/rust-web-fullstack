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

use live_search::app;
use live_search::events::SseEvent;
use live_search::{db, sse};

/// Fallback handler that does not use `State<S>` – works with `Router<()>`.
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Catch-all handler for server function endpoints.
///
/// Delegates to [`leptos_axum::handle_server_fns`] after attempting a
/// path-rewrite workaround: if the exact path is not registered but a
/// doubled-prefix variant exists (e.g. `/api/search` when the macro
/// registered `/api/api/search` due to `endpoint = "/api/search"`), the
/// URI is rewritten before dispatch.
///
/// # Panics
/// If the path-rewritten URI is invalid (should never happen in practice
/// because we only prepend `/api/` to an already-valid `/api/…` path).
#[expect(
    clippy::expect_used,
    reason = "Infallible: prepending /api/ to an already-valid /api/… path always produces a valid URI"
)]
async fn server_fn_handler(req: Request<Body>) -> impl IntoResponse {
    let method = req.method().clone();
    let original_path = req.uri().path().to_string();
    let (mut parts, body) = req.into_parts();

    // If the exact path isn't registered, try a doubled-prefix variant.
    // This handles the case where `#[server(endpoint = "/api/search")]`
    // concatenates the default prefix `/api` with the explicit endpoint
    // path, producing `/api/api/search` in the registry.
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
    tracing_subscriber::fmt().init();

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
    //   * `axum::serve` exits via the same token in `tokio::select!`
    //
    // Dropping `JoinSet` aborts its tasks, but we drain it explicitly with a
    // grace period so in-flight requests can complete cleanly.

    let shutdown = CancellationToken::new();
    spawn_signal_handler(shutdown.clone());

    let mut tasks = JoinSet::new();
    let listener_token = shutdown.child_token();
    tasks.spawn(db::run_pg_listener(pool, tx, listener_token));

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
    // The `/api/{*fn_name}` catch-all uses a custom handler that falls back
    // to a doubled-prefix lookup, working around a common Leptos 0.8 macro
    // interaction where `#[server(endpoint = "/api/…")]` registers the
    // function under `/api/api/…`.

    let app = Router::new()
        // ---- static assets (hydration JS/WASM, CSS) --------------------------
        //
        // Must be before .leptos_routes() so /pkg/* takes priority.
        .nest_service("/pkg", ServeDir::new("./pkg"))
        .route("/api/events", get(sse::sse_handler))
        .route("/api/{*fn_name}", any(server_fn_handler))
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

    // Race axum::serve against the shutdown token. Whichever finishes first wins.
    let server_token = shutdown.clone();
    let server = axum::serve(listener, app);
    tokio::select! {
        result = server => {
            result.context("live-search server exited with an error")?;
        }
        () = server_token.cancelled() => {
            tracing::info!("live-search shutdown requested");
        }
    }

    // Drain background tasks with a grace period so in-flight notifications
    // can flush. A second `cancel()` is idempotent and safe if the signal
    // handler already fired.
    shutdown.cancel();
    if tokio::time::timeout(Duration::from_secs(10), async {
        while tasks.join_next().await.is_some() {}
    })
    .await
    .is_err()
    {
        tracing::warn!("background tasks did not drain within 10s; aborting");
        tasks.abort_all();
    }

    Ok(())
}
