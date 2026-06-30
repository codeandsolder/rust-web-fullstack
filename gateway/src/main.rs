//! Gateway example binary — entry point.
//!
//! Configures the tracing subscriber (with optional `OTel` telemetry behind the
//! `otel` feature), load service modules, build the axum router, and serve
//! HTTP requests with graceful shutdown.
//!
//! # Dev keys
//!
//! Pass `--dev-keys` as the first argument to generate an ephemeral `EdDSA`
//! keypair at startup (keys are logged at `warn!` level).  Do not use
//! `--dev-keys` in production — the keypair changes on every restart and is
//! logged in plain text.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::signal;

use gateway_example::{gateway, module, services};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Handle `--dev-keys` flag before loading settings.
    let dev_keys = std::env::args().any(|arg| arg == "--dev-keys");

    // ---- Telemetry / tracing ----
    //
    // With the `otel` feature we use the canonical layered subscriber
    // (Registry + EnvFilter + fmt + `OTel`).  Without it we fall back to a
    // simple fmt subscriber.
    #[cfg(feature = "otel")]
    let provider =
        gateway_example::otel::init_telemetry().context("failed to initialize `OTel` telemetry")?;

    #[cfg(not(feature = "otel"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("gateway_example=info,tower_http=debug")
            }),
        )
        .init();

    // ---- Settings ----
    //
    // If `--dev-keys` was passed, generate an ephemeral keypair.  Otherwise
    // load from environment variables.
    let settings = if dev_keys {
        gateway_example::settings::Settings::load_dev_keys()?
    } else {
        gateway_example::settings::Settings::load()?
    };

    // ---- Service modules ----
    let service_modules: Vec<Arc<dyn module::ServiceModule>> = vec![
        Arc::new(services::search::SearchService),
        Arc::new(services::proxy::ProxyService),
        Arc::new(services::monitor::MonitorService),
    ];

    let app = gateway::build_gateway_with_settings(service_modules, settings)?;

    let port: u16 = std::env::var("GATEWAY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(3001);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));

    tracing::info!("gateway-example starting on {addr}");
    tracing::info!("  Health: http://{addr}/health");
    tracing::info!("  Login:  http://{addr}/auth/login");
    tracing::info!("  Events: http://{addr}/events");
    tracing::info!("  Docs:   http://{addr}/docs");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gateway listener on {addr}"))?;

    // Catch Ctrl+C and SIGTERM gracefully.
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("gateway server exited with an error")?;

    // ---- Shutdown telemetry ----
    //
    // Per rust-tracing §1.7: force_flush + shutdown (sync calls in
    // opentelemetry_sdk 0.32).  The provider is dropped when `provider`
    // exits scope.
    #[cfg(feature = "otel")]
    {
        let _ = provider.force_flush();
        let _ = provider.shutdown();
    }

    Ok(())
}

/// Return a future that resolves when a shutdown signal is received.
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
