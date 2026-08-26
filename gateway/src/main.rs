//! Gateway example binary — entry point.
//!
//! Configures tracing, loads validated configuration, runs the workspace's
//! shared database migrations, builds the Axum router, and serves requests with
//! graceful shutdown.

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
            "--dev-keys requires ALLOW_DEV_KEYS=1 to be set; refusing to start. \
             This is a deliberate guard against accidentally enabling ephemeral keys."
        );
    }

    // Load the typed workspace configuration exactly once. Legacy gateway env
    // variables remain supported where they pre-date the RWF_* config layer.
    let cfg = Config::load().context("failed to load workspace config")?;
    let port = match std::env::var("GATEWAY_PORT") {
        Ok(value) => value
            .parse::<u16>()
            .with_context(|| format!("invalid GATEWAY_PORT value {value:?}"))?,
        Err(std::env::VarError::NotPresent) => cfg.gateway.port,
        Err(e) => return Err(e).context("failed to read GATEWAY_PORT"),
    };
    let proxy_upstream_url = std::env::var("PROXY_UPSTREAM_URL")
        .unwrap_or_else(|_| cfg.gateway.proxy_upstream_url.clone());

    let refresh_token_ttl_secs = i64::try_from(cfg.gateway.refresh_token_ttl_secs)
        .context("gateway.refresh_token_ttl_secs exceeds i64::MAX")?;
    let access_token_ttl_secs = i64::try_from(cfg.gateway.access_token_ttl_secs)
        .context("gateway.access_token_ttl_secs exceeds i64::MAX")?;
    if refresh_token_ttl_secs <= 0 || access_token_ttl_secs <= 0 {
        anyhow::bail!("gateway token TTLs must be positive");
    }

    // ---- Telemetry / tracing ----
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
    let mut settings = if dev_keys {
        gateway_example::settings::Settings::load_dev_keys_from_env()?
    } else {
        gateway_example::settings::Settings::load()?
    };
    // The typed config is the canonical source for token lifetime. This avoids
    // the previous split-brain state where GatewayState and JWT minting used
    // two independently-loaded TTL values.
    settings.access_token_ttl_secs = access_token_ttl_secs;

    // ---- DB pool + migrations ----
    // Refresh-token issuance is part of every successful login, so a gateway
    // without its database is not a useful degraded mode. Fail fast instead of
    // starting a service whose login endpoint can only return 503.
    let db_pool = create_db_pool().await?;
    run_migrations(&db_pool)
        .await
        .context("gateway migrations failed")?;

    // ---- Service modules ----
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

/// Run the same migration history used by live-search. Keeping one SQLx
/// migration set for the shared database prevents either service from treating
/// the other service's applied versions as missing migrations.
async fn run_migrations(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../migrations").run(pool).await?;
    Ok(())
}
