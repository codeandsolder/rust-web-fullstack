//! Gateway router composition and shared state.
//!
//! Provides [`GatewayState`] (shared, clone-able mutable state for all
//! handlers) and [`build_gateway`] which composes all service modules into a
//! single Axum [`Router`] with:
//!
//! * JWT-based authentication (`EdDSA`) via [`crate::auth`]
//! * Per-route rate limiting via `tower_governor`
//! * Prometheus metrics at `/metrics`
//! * Request timeout safety net
//! * `OpenAPI` / Swagger UI at `/docs`

use std::sync::Arc;
use std::time::Duration;

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

// ---------------------------------------------------------------------------
// Shared gateway state
// ---------------------------------------------------------------------------

/// Shared mutable state available to every handler via [`State`] extraction.
#[derive(Clone)]
pub struct GatewayState {
    /// Broadcast channel for SSE events.
    pub tx: broadcast::Sender<GatewayEvent>,
    /// Read-only service descriptors (for API discovery).
    pub services: Vec<ServiceInfo>,
    /// Module trait objects kept alive for health aggregation.
    pub modules: Vec<Arc<dyn ServiceModule>>,
    /// Application settings loaded from environment variables at startup.
    pub settings: settings::Settings,
    /// Base URL for the proxy upstream API (default: <https://ipapi.co>).
    pub proxy_upstream_url: Arc<str>,
    /// Optional `sqlx::PgPool` for refresh-token rotation. `None` means
    /// the legacy stateless `/auth/refresh` semantics are used.
    pub db_pool: Option<sqlx::PgPool>,
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
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Router composition
// ---------------------------------------------------------------------------

/// Compose every `ServiceModule` under its own path prefix and attach
/// gateway-wide routes — `/health`, `/events`, `/auth/*`, `/metrics`, `/docs`.
///
/// Thin wrapper that loads [`settings::Settings`] from environment variables.
/// Use [`build_gateway_with_settings`] when you need to inject pre-loaded
/// settings (e.g. for `--dev-keys` or a DB pool).
///
/// # Errors
///
/// Returns an error if application settings cannot be loaded from
/// environment variables or governor configuration fails.
#[instrument(skip(modules))]
pub fn build_gateway(modules: Vec<Arc<dyn ServiceModule>>) -> Result<Router, anyhow::Error> {
    let settings = settings::Settings::load()?;
    build_gateway_with_settings(modules, settings, "https://ipapi.co".to_string(), None)
}

/// Compose every `ServiceModule` with pre-loaded [`settings::Settings`].
///
/// Identical to [`build_gateway`] but accepts an already-constructed
/// [`settings::Settings`] value (useful when `--dev-keys` was passed at startup).
///
/// # Errors
///
/// Returns an error if governor configuration fails.
#[instrument(skip(modules, settings))]
pub fn build_gateway_with_settings(
    modules: Vec<Arc<dyn ServiceModule>>,
    settings: settings::Settings,
    proxy_upstream_url: String,
    db_pool: Option<sqlx::PgPool>,
) -> Result<Router, anyhow::Error> {
    // 256 matches live-search's broadcast buffer size, providing room for ~256
    // concurrent subscribers before lag events fire.
    let (tx, _rx) = broadcast::channel(256);

    let service_infos: Vec<ServiceInfo> = modules
        .iter()
        .map(|m| ServiceInfo {
            name: m.name(),
            path: m.path(),
            description: m.description(),
            enabled: m.enabled(),
        })
        .collect();

    // --- nest each service's router ---
    let mut service_router: Router<GatewayState> = Router::new();
    for module in &modules {
        if module.enabled() {
            service_router = service_router.nest(&format!("/{}", module.path()), module.router());
        }
    }

    let proxy_upstream_url: Arc<str> = proxy_upstream_url.into();

    let state = GatewayState {
        tx,
        services: service_infos,
        modules,
        settings,
        proxy_upstream_url,
        db_pool,
    };

    // --- Prometheus metrics ---
    let (prometheus_layer, metric_handle) = PrometheusMetricLayer::pair();

    // --- Rate limiters ---
    use tower_governor::GovernorLayer;
    use tower_governor::governor::GovernorConfigBuilder;
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

    // --- CORS ---
    let cors = crate::cors::cors_layer();

    // --- Login route (with its own strict rate limiter) ---
    let login_router = Router::new()
        .route("/auth/login", post(auth::login_handler))
        .layer(login_governor);

    // --- Refresh route (separate rate limiter — mirrors login governor) ---
    let refresh_router = Router::new()
        .route("/auth/refresh", post(auth::refresh_handler))
        .layer(refresh_governor);

    // --- Everything else (general governor wraps non-login routes only) ---
    let other_router = Router::new()
        .route("/health", get(health_handler))
        .route("/events", get(sse::sse_handler))
        .route("/auth/logout", post(auth::logout_handler))
        .route("/", get(root_handler))
        .route(
            "/auth/protected",
            get(auth::protected_handler).route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth::auth_middleware,
            )),
        )
        .route(
            "/metrics",
            get(move || async move { metric_handle.render() }),
        )
        .merge(service_router)
        .merge(crate::openapi::swagger_ui_router())
        .layer(general_governor);

    // --- Assemble final router with shared middleware ---
    //
    // Layer order (from innermost to outermost):
    //   Governor (per-route, on sub-routers) → Timeout → Prometheus
    //   → Trace → CORS → CSP
    //
    // Middleware added LAST runs FIRST on incoming requests.
    let app = other_router
        .merge(login_router) // login governor runs before general governor
        .merge(refresh_router) // refresh governor runs before general governor
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(prometheus_layer)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .layer(axum::middleware::from_fn(crate::cors::csp_middleware))
        .with_state(state);

    Ok(app)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Per-module health probe timeout.
pub const HEALTH_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Maximum number of `/auth/refresh` requests allowed per second per client.
pub const REFRESH_RATE_PER_SECOND: u64 = 1;
/// Maximum burst size for the `/auth/refresh` rate limiter.
pub const REFRESH_BURST_SIZE: u32 = 5;

/// Aggregate health check — probes every registered service module in
/// parallel, each capped at [`HEALTH_CHECK_TIMEOUT`].
///
/// Returns `200 OK` when all services report healthy, `503 SERVICE_UNAVAILABLE`
/// when any service is unhealthy or times out.
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Gateway and all services healthy"),
        (status = 503, description = "Gateway degraded — one or more services unhealthy"),
    ),
    tag = "gateway",
)]
#[instrument(skip(state))]
pub async fn health_handler(State(state): State<GatewayState>) -> (StatusCode, Json<Value>) {
    let results = join_all(
        state
            .modules
            .iter()
            .filter(|m| m.enabled())
            .map(|module| async {
                let status =
                    match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, module.health_check()).await {
                        Ok(Ok(())) => "healthy",
                        Ok(Err(e)) => {
                            tracing::warn!(
                                name = module.name(),
                                error = %e,
                                "health check failed"
                            );
                            "unhealthy"
                        }
                        Err(_elapsed) => {
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

    let any_unhealthy = results.iter().any(|r| r["status"] != "healthy");
    let gateway_status = if any_unhealthy { "degraded" } else { "ok" };
    let http_status = if any_unhealthy {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (
        http_status,
        Json(json!({
            "gateway": gateway_status,
            "services": results,
        })),
    )
}

// ---------------------------------------------------------------------------
// Non-OpenAPI handlers (internal use only)
// ---------------------------------------------------------------------------

/// Root endpoint — returns the list of available services.
pub async fn root_handler(State(state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "gateway": "Gateway Example",
        "version": env!("CARGO_PKG_VERSION"),
        "services": state.services,
    }))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

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
    async fn health_endpoint_returns_503_when_any_service_unhealthy() -> anyhow::Result<()> {
        let (tx, _rx) = tokio::sync::broadcast::channel(100);
        let settings = crate::settings::Settings::load_dev_keys("test-admin-password")?;
        let modules: Vec<Arc<dyn crate::module::ServiceModule>> = vec![Arc::new(FailingService)];
        let state = GatewayState {
            tx,
            services: vec![],
            modules,
            settings,
            proxy_upstream_url: Arc::from("https://ipapi.co"),
            db_pool: None,
        };

        let (status, _body) = health_handler(State(state)).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        Ok(())
    }
}
