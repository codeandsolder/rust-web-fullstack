//! Server bootstrap — initialises all subsystems and starts the HTTP listener.
//!
//! This module is compiled only under `feature = "ssr"`. It is the single
//! call that sets up tracing, `PostgreSQL`, migrations, cache, SSE, the
//! `PgListener`, Leptos SSR routes, and graceful shutdown.

use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "otel")]
use std::sync::OnceLock;

use anyhow::Context;
use axum::http::{StatusCode, Uri};
use axum::response::IntoResponse;
use axum::{
    Router,
    routing::{any, get},
};
use leptos::config::get_configuration;
use leptos_axum::handle_server_fns;
use leptos_axum::{LeptosRoutes, generate_route_list};
use tokio::sync::broadcast;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing::Instrument;
use tracing_subscriber::EnvFilter;

use crate::app;
use crate::cache::CacheHandle;
use crate::events::SseEvent;
use crate::{db, sse, state};

/// Handle returned by [`run`] for cooperative shutdown and task draining.
#[derive(Debug)]
#[must_use]
pub struct ServerHandle {
    pub shutdown: CancellationToken,
    pub tasks: JoinSet<anyhow::Result<()>>,
    pub pool: sqlx::PgPool,
}

#[cfg(feature = "otel")]
static PROVIDER: OnceLock<opentelemetry_sdk::trace::SdkTracerProvider> = OnceLock::new();

#[cfg(feature = "otel")]
#[must_use]
pub fn get_tracer_provider() -> Option<&'static opentelemetry_sdk::trace::SdkTracerProvider> {
    PROVIDER.get()
}

fn init_tracing() {
    #[cfg(feature = "otel")]
    {
        match crate::otel::init_telemetry() {
            Ok(provider) => {
                let _ = PROVIDER.set(provider);
                return;
            }
            Err(e) => {
                eprintln!("OTel init failed, falling back to fmt subscriber: {e}");
            }
        }
    }

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,live_search=debug,tower_http=debug,sqlx=warn"));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .compact()
        .init();
}

/// Process liveness: if this handler runs, the HTTP process is alive.
async fn health_handler() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

/// Dependency readiness: verify application state exists and `PostgreSQL` can
/// answer a trivial query. This is intentionally separate from liveness so an
/// orchestrator can stop routing traffic without restart-looping a live process.
async fn readiness_handler() -> impl IntoResponse {
    let Some(ctx) = state::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "app state unavailable");
    };

    match sqlx::query_scalar::<_, i32>("SELECT 1")
        .fetch_one(&ctx.pool)
        .await
    {
        Ok(1) => (StatusCode::OK, "ready"),
        Ok(_) => (StatusCode::SERVICE_UNAVAILABLE, "database readiness failed"),
        Err(e) => {
            tracing::warn!(error = %e, "database readiness probe failed");
            (StatusCode::SERVICE_UNAVAILABLE, "database unavailable")
        }
    }
}

async fn fallback_handler(uri: Uri) -> impl IntoResponse {
    (StatusCode::NOT_FOUND, format!("Not found: {uri}"))
}

/// Bootstraps all subsystems and starts the HTTP server.
///
/// # Errors
/// Returns an error if configuration, database setup/migrations, or listener
/// binding fails.
pub async fn run() -> anyhow::Result<ServerHandle> {
    init_tracing();

    let cfg = rwf_config::Config::load().context("failed to load workspace config")?;
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .unwrap_or_else(|| cfg.live_search.database_url.clone());

    let pool_tunables = db::PoolTunables {
        max_connections: cfg.live_search.pool_max_connections,
        min_connections: cfg.live_search.pool_min_connections,
        acquire_timeout_secs: cfg.live_search.pool_acquire_timeout_secs,
        idle_timeout_secs: cfg.live_search.pool_idle_timeout_secs,
        max_lifetime_secs: cfg.live_search.pool_max_lifetime_secs,
    };
    tracing::info!("{}", cfg.live_search.connection_budget_summary());

    let pool = db::create_pool(&database_url, &pool_tunables)
        .await
        .context("failed to create database pool")?;

    // Every service sharing this database resolves the exact same SQLx
    // migration history.
    sqlx::migrate!("../migrations")
        .run(&pool)
        .await
        .context("failed to run database migrations")?;

    let cache_handle = CacheHandle::default();
    let (tx, _rx) = broadcast::channel::<SseEvent>(cfg.live_search.sse_broadcast_buffer);

    let ctx = Arc::new(state::AppContext::new(
        pool.clone(),
        tx.clone(),
        cache_handle.clone(),
    ));
    state::set(Arc::clone(&ctx))?;

    let shutdown = CancellationToken::new();
    let mut tasks = JoinSet::new();

    let listener_token = shutdown.child_token();
    let pool_for_listener = pool.clone();
    let listener_span = tracing::info_span!("pg_listener");
    let cache_for_listener = cache_handle.clone();
    tasks.spawn(
        async move {
            db::run_pg_listener(pool_for_listener, tx, cache_for_listener, listener_token).await;
            Ok(())
        }
        .instrument(listener_span),
    );

    let conf = get_configuration(None).context("failed to read Leptos configuration")?;
    let leptos_options = conf.leptos_options;
    let leptos_routes = generate_route_list(app::App);

    let broadcast_for_sse = ctx.broadcast.clone();
    let router = Router::new()
        .nest_service("/pkg", ServeDir::new("./pkg"))
        .route(
            "/api/events",
            get(move || {
                let tx = broadcast_for_sse.clone();
                async move { sse::sse_handler(tx).await }
            }),
        )
        .route("/api/{*fn_name}", any(handle_server_fns))
        .with_state(leptos_options.clone())
        .leptos_routes(&leptos_options, leptos_routes, {
            let lo = leptos_options.clone();
            let ctx_for_shell = Arc::clone(&ctx);
            move || {
                leptos::context::provide_context(Arc::clone(&ctx_for_shell));
                app::shell(lo.clone())
            }
        })
        .route("/health", get(health_handler))
        .route("/readyz", get(readiness_handler))
        .fallback(fallback_handler);

    #[cfg(feature = "otel")]
    let router = {
        use axum_prometheus::PrometheusMetricLayer;

        let (prom_layer, metric_handle) = PrometheusMetricLayer::pair();
        router.layer(prom_layer).route(
            "/metrics",
            get(move || async move { metric_handle.render() }),
        )
    };

    // Router::layer affects routes already registered. Apply global tracing
    // only after SSR, health/readiness, fallback, and optional metrics exist.
    let router = router.layer(TraceLayer::new_for_http());

    #[cfg(feature = "otel")]
    let router = router.layer(axum_tracing_opentelemetry::middleware::OtelAxumLayer::default());

    let router: Router<()> = router.with_state(leptos_options);

    let port: u16 = std::env::var("PORT")
        .or_else(|_| std::env::var("LIVE_SEARCH_PORT"))
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(cfg.live_search.port);
    let addr = SocketAddr::from(([0, 0, 0, 0], port));
    tracing::info!("Live search server listening on {addr}");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind live-search listener on {addr}"))?;

    let graceful_shutdown_token = shutdown.clone();
    tasks.spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                graceful_shutdown_token.cancelled().await;
            })
            .await
            .context("live-search server exited with an error")
    });

    Ok(ServerHandle {
        shutdown,
        tasks,
        pool,
    })
}
