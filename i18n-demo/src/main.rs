//! SSR server binary for the i18n-demo application.
//!
//! Sets up:
//! - Axum HTTP server with Leptos SSR routes and static assets.
//! - Graceful shutdown via `CancellationToken` on `Ctrl+C` / `SIGTERM`.
//!
//! No database or SSE — this is a pure i18n demonstration.

#![cfg(feature = "ssr")]

use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::any;
use leptos::config::get_configuration;
use leptos_axum::{LeptosRoutes, generate_route_list};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use i18n_demo::app;

/// Fallback handler that does not use `State<S>` – works with `Router<()>`.
async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Server-function dispatch handler.
///
/// Leptos 0.8's `#[server(endpoint = "/api/…")]` macro registers each server fn
/// at the **doubled** path `/api/api/…` (the default `/api` prefix concatenated
/// with the explicit endpoint).  The compile-time `server_fn` client knows only
/// the original path `/api/…`, so requests from the browser will 404 on
/// `leptos_axum::handle_server_fns` unless we probe for the doubled-prefix
/// variant and rewrite the request URI in place.
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
/// SIGTERM (Unix).  The token is observed by every clone/child, propagating
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
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,i18n_demo=debug,tower_http=debug")),
        )
        .init();

    // ---- Graceful shutdown via CancellationToken -------------------------
    //
    // The shutdown token propagates to the signal handler and to axum's
    // graceful shutdown.

    let shutdown = CancellationToken::new();
    spawn_signal_handler(shutdown.clone());

    // ---- Leptos configuration & routes ------------------------------------

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(app::App);

    // ---- Axum router ------------------------------------------------------
    //
    // Build a stateful router `Router<LeptosOptions>` so the `LeptosRoutes`
    // trait bound is satisfied.  Both `/api/{*fn_name}` and
    // `/api/api/{*fn_name}` are registered for the doubled-prefix workaround.

    let axum_app = Router::new()
        // ---- static assets (hydration JS/WASM, CSS) ------------------------
        //
        // Must be before .leptos_routes() so /pkg/* takes priority.
        .nest_service("/pkg", ServeDir::new("./pkg"))
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
    let axum_app: Router<()> = axum_app.with_state(leptos_options);

    // ---- serve ------------------------------------------------------------

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3002);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("i18n-demo server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind i18n-demo listener on {addr}"))?;

    let graceful_shutdown_token = shutdown.clone();
    axum::serve(listener, axum_app)
        .with_graceful_shutdown(async move {
            graceful_shutdown_token.cancelled().await;
        })
        .await
        .context("i18n-demo server exited with an error")?;

    Ok(())
}
