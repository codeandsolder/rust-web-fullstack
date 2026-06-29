use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::signal;
use tracing_subscriber::EnvFilter;

use gateway_example::{gateway, module, services};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("gateway_example=info,tower_http=debug")),
        )
        .init();

    // Register every service module as an `Arc<dyn ServiceModule>`.
    let service_modules: Vec<Arc<dyn module::ServiceModule>> = vec![
        Arc::new(services::search::SearchService),
        Arc::new(services::proxy::ProxyService),
        Arc::new(services::monitor::MonitorService),
    ];

    let app = gateway::build_gateway(service_modules)?;

    let port: u16 = std::env::var("GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!("gateway-example starting on {addr}");
    tracing::info!("  Health: http://{addr}/health");
    tracing::info!("  Login:  http://{addr}/auth/login");
    tracing::info!("  Events: http://{addr}/events");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gateway listener on {addr}"))?;

    // Catch Ctrl+C and SIGTERM gracefully.  We use `with_graceful_shutdown`
    // so axum drains in-flight requests before the process exits.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("gateway server exited with an error")?;

    Ok(())
}

/// Return a future that resolves when a shutdown signal is received.
///
/// Listens for Ctrl+C (all platforms) and SIGTERM (Unix).  Errors during
/// signal installation are logged but do not prevent startup.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = signal::ctrl_c().await {
            tracing::error!("failed to install Ctrl+C handler: {e}");
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
        () = ctrl_c => tracing::info!("Ctrl+C received, shutting down"),
        () = terminate => tracing::info!("SIGTERM received, shutting down"),
    }
}
