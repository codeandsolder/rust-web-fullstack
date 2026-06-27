use std::net::SocketAddr;

use anyhow::Context;
use tokio::signal;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::EnvFilter;

mod auth;
mod gateway;
mod module;
mod services;
mod settings;
mod sse;

/// Spawn a task that fires the shutdown token on Ctrl+C (all platforms) or
/// SIGTERM (Unix). The token is observed by every clone, propagating shutdown
/// to all long-running tasks.
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
                .unwrap_or_else(|_| EnvFilter::new("gateway_example=info,tower_http=debug")),
        )
        .init();

    let _settings = settings::Settings::load();

    // Register every service module.
    let service_modules: Vec<Box<dyn module::ServiceModule>> = vec![
        Box::new(services::search::SearchService),
        Box::new(services::proxy::ProxyService),
        Box::new(services::monitor::MonitorService),
    ];

    let app = gateway::build_gateway(service_modules);

    let addr = SocketAddr::from(([0, 0, 0, 0], 3001));

    tracing::info!("gateway-example starting on {addr}");
    tracing::info!("  Health: http://{addr}/health");
    tracing::info!("  Login:  http://{addr}/auth/login");
    tracing::info!("  Events: http://{addr}/events");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gateway listener on {addr}"))?;

    // Race axum::serve against the shutdown token. No JoinSet needed — the
    // gateway has no long-lived background tasks beyond the request handlers
    // themselves, which axum drains gracefully when the listener drops.
    let shutdown = CancellationToken::new();
    spawn_signal_handler(shutdown.clone());

    let server_token = shutdown.clone();
    let server = axum::serve(listener, app);
    tokio::select! {
        result = server => {
            result.context("gateway server exited with an error")?;
        }
        () = server_token.cancelled() => {
            tracing::info!("gateway shutdown requested");
        }
    }

    Ok(())
}
