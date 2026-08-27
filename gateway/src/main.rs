//! Gateway example binary — entry point.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::signal;

use gateway_example::{gateway, module, services};
use rwf_config::Config;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dev_keys = std::env::args().any(|arg| arg == "--dev-keys");
    if dev_keys && std::env::var("ALLOW_DEV_KEYS").ok().as_deref() != Some("1") {
        anyhow::bail!(
            "--dev-keys requires ALLOW_DEV_KEYS=1 to be set; refusing to start"
        );
    }

    // Non-secret runtime configuration has one canonical source: rwf-config.
    // Override it with `RWF_GATEWAY__...`, not a parallel set of ad-hoc env
    // variables. Secret key/password material remains in Settings.
    let cfg = Config::load().context("failed to load workspace config")?;
    let port = cfg.gateway.port;
    let proxy_upstream_url = cfg.gateway.proxy_upstream_url.clone();
    let refresh_token_ttl_secs = i64::try_from(cfg.gateway.refresh_token_ttl_secs)
        .context("gateway.refresh_token_ttl_secs exceeds i64::MAX")?;
    let access_token_ttl_secs = i64::try_from(cfg.gateway.access_token_ttl_secs)
        .context("gateway.access_token_ttl_secs exceeds i64::MAX")?;

    #[cfg(feature = "otel")]
    let provider =
        gateway_example::otel::init_telemetry().context("failed to initialize OTel telemetry")?;

    #[cfg(not(feature = "otel"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("gateway_example=info,tower_http=debug")
            }),
        )
        .init();

    let mut settings = if dev_keys {
        gateway_example::settings::Settings::load_dev_keys_from_env()?
    } else {
        gateway_example::settings::Settings::load()?
    };
    settings.access_token_ttl_secs = access_token_ttl_secs;
    settings.allowed_origins = Arc::from(cfg.gateway.cors.allowed_origins.as_str());
    settings.sse_broadcast_buffer = cfg.gateway.sse_broadcast_buffer;
    settings.session.cookie_secure = cfg.gateway.session.cookie_secure;

    let db_pool = create_db_pool().await?;
    run_migrations(&db_pool)
        .await
        .context("gateway migrations failed")?;

    let service_modules: Vec<Arc<dyn module::ServiceModule>> = vec![
        Arc::new(services::search::SearchService),
        Arc::new(services::proxy::ProxyService),
        Arc::new(services::monitor::MonitorService),
    ];

    let app = gateway::build_gateway_with_settings(
        service_modules,
        settings,
        proxy_upstream_url,
        Some(db_pool),
        refresh_token_ttl_secs,
        access_token_ttl_secs,
    )?;

    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("gateway-example starting on {addr}");
    tracing::info!("  Health: http://{addr}/health");
    tracing::info!("  Login:  http://{addr}/auth/login");
    tracing::info!("  Events: http://{addr}/events");
    tracing::info!("  Docs:   http://{addr}/docs");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind gateway listener on {addr}"))?;

    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .context("gateway server exited with an error")?;

    #[cfg(feature = "otel")]
    {
        let _ = provider.force_flush();
        let _ = provider.shutdown();
    }

    Ok(())
}

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

async fn create_db_pool() -> anyhow::Result<sqlx::PgPool> {
    let url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set; refresh-token storage is required")?;
    tracing::info!("creating PostgreSQL pool for refresh-token rotation");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(&url)
        .await
        .context("failed to connect to PostgreSQL")
}

async fn run_migrations(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../migrations").run(pool).await?;
    Ok(())
}
