//! SSR server binary for the i18n-demo application.
//!
//! Sets up Axum + Leptos SSR routes, static assets, and graceful shutdown.

#![cfg(feature = "ssr")]

use std::net::SocketAddr;

use anyhow::Context;
use axum::Router;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::{any, get};
use leptos::config::get_configuration;
use leptos_axum::{LeptosRoutes, generate_route_list, handle_server_fns};
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use i18n_demo::app;

async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

fn spawn_signal_handler(shutdown: CancellationToken) {
    tokio::spawn(async move {
        let ctrl_c = async {
            if let Err(e) = signal::ctrl_c().await {
                tracing::error!(error = %e, "failed to install Ctrl+C handler");
            }
        };

        #[cfg(unix)]
        let terminate = async {
            let Ok(mut sig) = signal::unix::signal(signal::unix::SignalKind::terminate()) else {
                tracing::error!("failed to install SIGTERM handler");
                return;
            };
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

    let shutdown = CancellationToken::new();
    spawn_signal_handler(shutdown.clone());

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let routes = generate_route_list(app::App);

    // `#[server(endpoint = "name")]` endpoints are relative to Leptos's
    // default `/api` prefix. Mount one canonical catch-all; `/api/api/*` was a
    // workaround for a configuration bug, not a Leptos requirement.
    let axum_app = Router::new()
        .nest_service("/pkg", ServeDir::new("./pkg"))
        .route("/ws/chat", get(i18n_demo::ws_chat::chat_handler))
        .route("/api/{*fn_name}", any(handle_server_fns))
        .with_state(leptos_options.clone())
        .leptos_routes(&leptos_options, routes, {
            let lo = leptos_options.clone();
            move || app::shell(lo.clone())
        })
        .fallback(fallback_handler)
        // Router::layer applies to routes already registered, so global
        // tracing belongs after the Leptos routes and fallback are added.
        .layer(TraceLayer::new_for_http());

    let axum_app: Router<()> = axum_app.with_state(leptos_options);

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
