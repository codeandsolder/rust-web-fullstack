//! Gateway router composition and shared state.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::{
    Router,
    extract::State,
    http::StatusCode,
    middleware,
    response::Json,
    routing::{get, post},
};
use axum_prometheus::PrometheusMetricLayer;
use futures::future::join_all;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tracing::instrument;

use crate::auth;
use crate::module::{ServiceInfo, ServiceModule};
use crate::settings;
use crate::sse::{self, GatewayEvent};

#[derive(Clone)]
pub struct GatewayState {
    pub tx: broadcast::Sender<GatewayEvent>,
    pub services: Vec<ServiceInfo>,
    pub modules: Vec<Arc<dyn ServiceModule>>,
    pub settings: settings::Settings,
    pub proxy_upstream_url: Arc<str>,
    pub db_pool: Option<sqlx::PgPool>,
    pub refresh_token_ttl_secs: i64,
    pub access_token_ttl_secs: i64,
    pub http_client: reqwest::Client,
}

impl std::fmt::Debug for GatewayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayState")
            .field("tx", &self.tx)
            .field("services", &self.services)
            .field("modules", &format_args!("[{} modules]", self.modules.len()))
            .field("settings", &self.settings)
            .field("proxy_upstream_url", &self.proxy_upstream_url)
            .field("db_pool", &self.db_pool.as_ref().map(|_| "PgPool { .. }"))
            .field("refresh_token_ttl_secs", &self.refresh_token_ttl_secs)
            .field("access_token_ttl_secs", &self.access_token_ttl_secs)
            .field("http_client", &format_args!("reqwest::Client {{ .. }}"))
            .finish()
    }
}

/// Convenience constructor for non-production callers.
///
/// This synchronous wrapper cannot create a DB pool, so DB-backed auth routes
/// fail closed and `/health` reports the database as not configured. Production
/// startup uses [`build_gateway_with_settings`] after connecting to Postgres.
///
/// # Errors
/// Returns an error for invalid settings/configuration or router setup.
#[instrument(skip(modules))]
pub fn build_gateway(modules: Vec<Arc<dyn ServiceModule>>) -> Result<Router, anyhow::Error> {
    let cfg = rwf_config::Config::load().context("failed to load workspace config")?;
    let refresh_token_ttl_secs = i64::try_from(cfg.gateway.refresh_token_ttl_secs)
        .context("gateway.refresh_token_ttl_secs exceeds i64::MAX")?;
    let access_token_ttl_secs = i64::try_from(cfg.gateway.access_token_ttl_secs)
        .context("gateway.access_token_ttl_secs exceeds i64::MAX")?;

    let mut runtime_settings = settings::Settings::load()?;
    runtime_settings.access_token_ttl_secs = access_token_ttl_secs;
    runtime_settings.allowed_origins = Arc::from(cfg.gateway.cors.allowed_origins.as_str());
    runtime_settings.sse_broadcast_buffer = cfg.gateway.sse_broadcast_buffer;
    runtime_settings.session.cookie_secure = cfg.gateway.session.cookie_secure;

    build_gateway_with_settings(
        modules,
        runtime_settings,
        cfg.gateway.proxy_upstream_url,
        None,
        refresh_token_ttl_secs,
        access_token_ttl_secs,
    )
}

/// Compose every `ServiceModule` with pre-loaded settings and runtime state.
///
/// # Errors
/// Returns an error if TTLs are invalid, the HTTP client cannot be built, or
/// rate-limiter configuration fails.
#[instrument(skip(modules, settings, db_pool))]
#[expect(
    clippy::too_many_arguments,
    reason = "public compatibility constructor; a typed runtime config can replace it in a later API revision"
)]
pub fn build_gateway_with_settings(
    modules: Vec<Arc<dyn ServiceModule>>,
    mut settings: settings::Settings,
    proxy_upstream_url: String,
    db_pool: Option<sqlx::PgPool>,
    refresh_token_ttl_secs: i64,
    access_token_ttl_secs: i64,
) -> Result<Router, anyhow::Error> {
    if refresh_token_ttl_secs <= 0 || access_token_ttl_secs <= 0 {
        anyhow::bail!("gateway token TTLs must be positive");
    }
    if settings.sse_broadcast_buffer == 0 {
        anyhow::bail!("gateway SSE broadcast buffer must be greater than zero");
    }
    settings.access_token_ttl_secs = access_token_ttl_secs;

    let (tx, _rx) = broadcast::channel(settings.sse_broadcast_buffer);

    let service_infos: Vec<ServiceInfo> = modules
        .iter()
        .map(|module| ServiceInfo {
            name: module.name(),
            path: module.path(),
            description: module.description(),
            enabled: module.enabled(),
        })
        .collect();

    let mut service_router: Router<GatewayState> = Router::new();
    for module in &modules {
        if module.enabled() {
            service_router = service_router.nest(&format!("/{}", module.path()), module.router());
        }
    }

    let http_client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .context("failed to build reqwest client")?;

    let state = GatewayState {
        tx,
        services: service_infos,
        modules,
        settings,
        proxy_upstream_url: proxy_upstream_url.into(),
        db_pool,
        refresh_token_ttl_secs,
        access_token_ttl_secs,
        http_client,
    };

    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    use tower_governor::GovernorLayer;
    use tower_governor::governor::GovernorConfigBuilder;

    // Peer-IP limiting is the safe default for directly exposed deployments.
    // Forwarded headers must only be trusted behind a configured trusted proxy;
    // blindly using SmartIpKeyExtractor would let direct clients spoof identity.
    let login_governor_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(1)
            .burst_size(5)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build login governor config"))?,
    );
    let refresh_governor_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(REFRESH_RATE_PER_SECOND)
            .burst_size(REFRESH_BURST_SIZE)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build refresh governor config"))?,
    );
    let general_governor_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(10)
            .burst_size(20)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build general governor config"))?,
    );

    let login_governor = GovernorLayer::new(login_governor_cfg);
    let refresh_governor = GovernorLayer::new(refresh_governor_cfg);
    let general_governor = GovernorLayer::new(general_governor_cfg);

    let cors = crate::cors::cors_layer(state.settings.allowed_origins.as_ref());
    let session_layer = crate::session::session_layer(&state.settings.session);
    let csrf_middleware = axum::middleware::from_fn(crate::csrf::CsrfMiddleware::middleware);

    let login_router = Router::new()
        .route("/auth/login", post(auth::login_handler))
        .layer(login_governor);

    let refresh_router = Router::new()
        .route("/auth/refresh", post(auth::refresh_handler))
        .layer(refresh_governor);

    let other_router = Router::new()
        .route("/health", get(health_handler))
        .route("/events", get(sse::sse_handler))
        .route(
            "/auth/logout",
            post(auth::logout_handler)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    auth::auth_middleware,
                ))
                .route_layer(csrf_middleware.clone()),
        )
        .route("/", get(root_handler))
        .route(
            "/auth/protected",
            get(auth::protected_handler)
                .route_layer(middleware::from_fn_with_state(
                    state.clone(),
                    auth::auth_middleware,
                ))
                .route_layer(csrf_middleware.clone()),
        )
        .route(
            "/metrics",
            get(move || async move { metric_handle.render() }),
        )
        .merge(service_router)
        .merge(crate::openapi::swagger_ui_router())
        .layer(general_governor);

    let session_router =
        crate::session::router::<GatewayState>().route_layer(csrf_middleware.clone());

    async fn csrf_token_handler(
        session: tower_sessions::Session,
    ) -> Result<Json<serde_json::Value>, crate::auth::error::AppError> {
        let token = crate::csrf::get_or_create_token(&session)
            .await
            .map_err(|e| {
                crate::auth::error::AppError::internal(
                    "csrf token bootstrap",
                    std::io::Error::other(e),
                )
            })?;
        Ok(Json(serde_json::json!({
            "csrf_token": token.as_str(),
            "header": crate::csrf::TOKEN_HEADER,
        })))
    }

    let app = Router::new()
        .route("/auth/csrf", get(csrf_token_handler))
        .merge(login_router)
        .merge(refresh_router)
        .merge(other_router)
        .merge(session_router)
        .layer(session_layer)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(prometheus_layer)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(axum::middleware::from_fn(crate::cors::csp_middleware))
        .with_state(state);

    #[cfg(feature = "otel")]
    let app = app.layer(
        axum_tracing_opentelemetry::middleware::OtelAxumLayer::default(),
    );

    Ok(app)
}

pub const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(5);
pub const REFRESH_RATE_PER_SECOND: u64 = 1;
pub const REFRESH_BURST_SIZE: u32 = 5;

/// Aggregate readiness check for modules and the database required by auth.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Gateway and required dependencies healthy"),
        (status = 503, description = "Gateway degraded — a module or required dependency is unhealthy"),
    ),
    tag = "gateway",
)]
#[instrument(skip(state))]
pub async fn health_handler(State(state): State<GatewayState>) -> (StatusCode, Json<Value>) {
    let services = join_all(
        state
            .modules
            .iter()
            .filter(|module| module.enabled())
            .map(|module| async {
                let status =
                    match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, module.health_check()).await {
                        Ok(Ok(())) => "healthy",
                        Ok(Err(e)) => {
                            tracing::warn!(name = module.name(), error = %e, "health check failed");
                            "unhealthy"
                        }
                        Err(_) => {
                            tracing::warn!(
                                name = module.name(),
                                timeout_ms = HEALTH_CHECK_TIMEOUT.as_millis(),
                                "health check timed out"
                            );
                            "unhealthy"
                        }
                    };
                json!({
                    "name": module.name(),
                    "path": module.path(),
                    "enabled": module.enabled(),
                    "status": status,
                })
            }),
    )
    .await;

    let database_status = match state.db_pool.as_ref() {
        Some(pool) => match tokio::time::timeout(
            HEALTH_CHECK_TIMEOUT,
            sqlx::query_scalar::<_, i32>("SELECT 1").fetch_one(pool),
        )
        .await
        {
            Ok(Ok(1)) => "healthy",
            Ok(Ok(_)) => "unhealthy",
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "gateway database health check failed");
                "unhealthy"
            }
            Err(_) => {
                tracing::warn!("gateway database health check timed out");
                "unhealthy"
            }
        },
        None => "not_configured",
    };

    let any_unhealthy = services.iter().any(|result| result["status"] != "healthy")
        || database_status != "healthy";
    let http_status = if any_unhealthy {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };

    (
        http_status,
        Json(json!({
            "gateway": if any_unhealthy { "degraded" } else { "ok" },
            "dependencies": { "database": database_status },
            "services": services,
        })),
    )
}

pub async fn root_handler(State(state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "gateway": "Gateway Example",
        "version": env!("CARGO_PKG_VERSION"),
        "services": state.services,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::extract::State;
    use futures::future::FutureExt;

    struct FailingService;

    impl crate::module::ServiceModule for FailingService {
        fn name(&self) -> &'static str {
            "failing"
        }

        fn description(&self) -> &'static str {
            "always unhealthy (for tests)"
        }

        fn router(&self) -> axum::Router<GatewayState> {
            axum::Router::new()
        }

        fn health_check(
            &self,
        ) -> futures::future::BoxFuture<'_, Result<(), crate::module::ServiceHealthError>> {
            async {
                Err(crate::module::ServiceHealthError {
                    reason: "test-induced failure".into(),
                })
            }
            .boxed()
        }
    }

    #[tokio::test]
    async fn health_endpoint_returns_503_when_service_or_database_unhealthy() -> anyhow::Result<()> {
        let (tx, _rx) = tokio::sync::broadcast::channel(100);
        let settings = crate::settings::Settings::load_dev_keys("test-admin-password")?;
        let modules: Vec<Arc<dyn crate::module::ServiceModule>> = vec![Arc::new(FailingService)];
        let http_client = reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .context("failed to build reqwest client")?;
        let state = GatewayState {
            tx,
            services: vec![],
            modules,
            settings,
            proxy_upstream_url: Arc::from("https://ipapi.co"),
            db_pool: None,
            refresh_token_ttl_secs: 60 * 60 * 24 * 30,
            access_token_ttl_secs: 15 * 60,
            http_client,
        };

        let (status, body) = health_handler(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body.0["dependencies"]["database"], "not_configured");
        Ok(())
    }
}
