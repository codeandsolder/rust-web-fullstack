//! Gateway example binary — entry point.

use std::net::SocketAddr;
use std::sync::Arc;

use anyhow::Context;
use tokio::signal;

use gateway_example::{gateway, module, services};
use rwf_config::{Config, Server, StartupOptions};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let options = StartupOptions::parse(Server::Gateway, std::env::args_os().skip(1))?;
    let cfg =
        Config::load(options.config_path.as_deref()).context("failed to load workspace config")?;
    if options.dev_keys && !cfg.gateway.allow_dev_keys {
        anyhow::bail!(
            "--dev-keys requires gateway.allow_dev_keys = true in the selected configuration"
        );
    }
    let port = cfg.gateway.port;
    let proxy_upstream_url = cfg.gateway.proxy_upstream_url.clone();

    #[cfg(feature = "otel")]
    let provider = rwf_telemetry::init_telemetry(rwf_telemetry::TelemetryConfig {
        service_name: "gateway-example",
        default_filter: "gateway_example=info,tower_http=debug",
        directives: &[],
    })
    .context("failed to initialize OTel telemetry")?;

    #[cfg(not(feature = "otel"))]
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("gateway_example=info,tower_http=debug")
            }),
        )
        .init();

    let password = std::env::var("ADMIN_PASSWORD").context("ADMIN_PASSWORD must be set")?;
    let settings = if options.dev_keys {
        gateway_example::settings::Settings::generate_dev_keys(&password, &cfg.gateway)?
    } else {
        let private_key =
            std::env::var("JWT_PRIVATE_KEY_PEM").context("JWT_PRIVATE_KEY_PEM must be set")?;
        let public_key =
            std::env::var("JWT_PUBLIC_KEY_PEM").context("JWT_PUBLIC_KEY_PEM must be set")?;
        gateway_example::settings::Settings::from_credentials(
            &private_key,
            &public_key,
            &password,
            &cfg.gateway,
        )?
    };

    let database_url = std::env::var("DATABASE_URL")
        .context("DATABASE_URL must be set; refresh-token storage is required")?;
    let db_pool = create_db_pool(&database_url).await?;
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

async fn create_db_pool(url: &str) -> anyhow::Result<sqlx::PgPool> {
    tracing::info!("creating PostgreSQL pool for refresh-token rotation");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(std::time::Duration::from_secs(5))
        .connect(url)
        .await
        .context("failed to connect to PostgreSQL")
}

async fn run_migrations(pool: &sqlx::PgPool) -> anyhow::Result<()> {
    sqlx::migrate!("../migrations").run(pool).await?;
    Ok(())
}
