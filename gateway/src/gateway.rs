//! Gateway router composition and shared state.
//!
//! Provides [`GatewayState`] (shared, clone-able mutable state for all
//! handlers) and [`build_gateway`] which composes all service modules into a
//! single Axum [`Router`] with:
//!
//! * JWT-based authentication (`EdDSA`) via [`crate::auth`]
//! * Per-route rate limiting via `tower_governor`
//! * Session management via `tower-sessions`
//! * CSRF protection via `axum-tower-sessions-csrf`
//! * Prometheus metrics at `/metrics`
//! * Request timeout safety net
//! * `OpenAPI` / Swagger UI at `/docs`

use std::sync::Arc;
use std::time::Duration;

use axum::middleware as axum_mw;
use axum::{
    Router,
    extract::State,
    middleware,
    response::Json,
    routing::{get, post},
};
use axum_prometheus::PrometheusMetricLayer;
use axum_tower_sessions_csrf::CsrfMiddleware;
use futures::future::join_all;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::cors::{Any, CorsLayer};
use tower_http::timeout::TimeoutLayer;
use tower_http::trace::TraceLayer;
use tower_sessions::SessionManagerLayer;
use tower_sessions_memory_store::MemoryStore;
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
}

impl std::fmt::Debug for GatewayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayState")
            .field("tx", &self.tx)
            .field("services", &self.services)
            .field("modules", &format_args!("[{} modules]", self.modules.len()))
            .field("settings", &self.settings)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Router composition
// ---------------------------------------------------------------------------

/// Compose every `ServiceModule` under its own path prefix and attach
/// gateway-wide routes — `/health`, `/events`, `/auth/*`, `/metrics`, `/docs`.
///
/// Convenience wrapper that loads [`settings::Settings`] from environment
/// variables.  Use [`build_gateway_with_settings`] when you need to inject
/// pre-loaded settings (e.g. for `--dev-keys`).
///
/// # Errors
///
/// Returns an error if application settings cannot be loaded from
/// environment variables or governor configuration fails.
#[instrument(skip(modules))]
pub fn build_gateway(modules: Vec<Arc<dyn ServiceModule>>) -> Result<Router, anyhow::Error> {
    let settings = settings::Settings::load()?;
    build_gateway_with_settings(modules, settings)
}

/// Compose every `ServiceModule` with pre-loaded [`settings`].
///
/// Identical to [`build_gateway`] but accepts an already-constructed
/// [`Settings`] value (useful when `--dev-keys` was passed at startup).
///
/// # Errors
///
/// Returns an error if governor configuration fails.
#[instrument(skip(modules, settings))]
pub fn build_gateway_with_settings(
    modules: Vec<Arc<dyn ServiceModule>>,
    settings: settings::Settings,
) -> Result<Router, anyhow::Error> {
    let (tx, _rx) = broadcast::channel(100);

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

    let state = GatewayState {
        tx,
        services: service_infos,
        modules,
        settings,
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
    let general_governor_cfg = Arc::new(
        GovernorConfigBuilder::default()
            .per_second(10)
            .burst_size(20)
            .finish()
            .ok_or_else(|| anyhow::anyhow!("failed to build general governor config"))?,
    );
    let login_governor = GovernorLayer::new(login_governor_cfg);
    let general_governor = GovernorLayer::new(general_governor_cfg);

    // --- Session layer ---
    let session_store = MemoryStore::default();
    let session_layer = SessionManagerLayer::new(session_store)
        .with_secure(false)
        .with_same_site(tower_sessions::cookie::SameSite::Lax);

    // --- CSRF layer ---
    //
    // The Synchronizer Token Pattern: the server stores a CSRF token in the
    // session, and the client must echo it back (typically via a custom header
    // or form field) on state-changing requests.  `axum-tower-sessions-csrf`
    // handles token generation, session storage, and constant-time comparison.
    let csrf_layer = axum_mw::from_fn(CsrfMiddleware::middleware);

    // --- CORS ---
    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    // --- Login route (with its own strict rate limiter) ---
    let login_router = Router::new()
        .route("/auth/login", post(auth::login_handler))
        .layer(login_governor);

    // --- Everything else ---
    let other_router = Router::new()
        .route("/health", get(health_handler))
        .route("/events", get(sse::sse_handler))
        .route("/auth/refresh", post(auth::refresh_handler))
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
        .layer(general_governor)
        .merge(login_router);

    // --- Assemble final router with shared middleware ---
    //
    // Layer order (from innermost to outermost):
    //   Governor (per-route, on sub-routers) → CSRF → Session → Timeout
    //   → Prometheus → Trace → CORS
    //
    // Middleware added LAST runs FIRST on incoming requests.
    let app = other_router
        .layer(csrf_layer)
        .layer(session_layer)
        .layer(TimeoutLayer::with_status_code(
            axum::http::StatusCode::GATEWAY_TIMEOUT,
            Duration::from_secs(60),
        ))
        .layer(prometheus_layer)
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    Ok(app)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Per-module health probe timeout.
const HEALTH_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Aggregate health check — probes every registered service module in
/// parallel, each capped at [`HEALTH_CHECK_TIMEOUT`].
#[utoipa::path(
    get,
    path = "/health",
    responses(
        (status = 200, description = "Gateway and all services healthy"),
    ),
    tag = "gateway",
)]
#[instrument(skip(state))]
pub(crate) async fn health_handler(State(state): State<GatewayState>) -> Json<Value> {
    let results = join_all(state.modules.iter().map(|module| async {
        let status = match tokio::time::timeout(HEALTH_CHECK_TIMEOUT, module.health_check()).await {
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
    }))
    .await;

    Json(json!({
        "gateway": "ok",
        "services": results,
    }))
}

// ---------------------------------------------------------------------------
// Non-OpenAPI handlers (internal use only)
// ---------------------------------------------------------------------------

/// Root endpoint — returns the list of available services.
pub(crate) async fn root_handler(State(state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "gateway": "Gateway Example",
        "version": env!("CARGO_PKG_VERSION"),
        "services": state.services,
    }))
}
