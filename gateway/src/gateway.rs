//! Gateway router composition and shared state.
//!
//! Provides [`GatewayState`] (shared, clone-able mutable state for all
//! handlers) and [`build_gateway`] which composes all service modules into a
//! single Axum [`Router`].

use std::sync::Arc;

use axum::{
    Router,
    extract::{Extension, State},
    middleware,
    response::Json,
    routing::{get, post},
};
use futures::future::join_all;
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;
use tracing::instrument;

use crate::{
    auth::{self, Claims, LoginRateLimiter},
    module::{ServiceInfo, ServiceModule},
    settings,
    sse::{self, GatewayEvent},
};

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
    /// Per-IP login rate limiter.
    pub rate_limiter: LoginRateLimiter,
}

impl std::fmt::Debug for GatewayState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GatewayState")
            .field("tx", &self.tx)
            .field("services", &self.services)
            .field("modules", &format_args!("[{} modules]", self.modules.len()))
            .field("settings", &self.settings)
            .field("rate_limiter", &self.rate_limiter)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Router composition
// ---------------------------------------------------------------------------

/// Compose every `ServiceModule` under its own path prefix and attach
/// gateway-wide routes — `/health`, `/events`, `/auth/login`, and `/`.
///
/// # Errors
///
/// Returns an error if application settings cannot be loaded from
/// environment variables.
#[instrument(skip(modules))]
pub fn build_gateway(modules: Vec<Arc<dyn ServiceModule>>) -> Result<Router, anyhow::Error> {
    let settings = settings::Settings::load()?;
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
    let mut router: Router<GatewayState> = Router::new();
    for module in &modules {
        if module.enabled() {
            router = router.nest(&format!("/{}", module.path()), module.router());
        }
    }

    let state = GatewayState {
        tx,
        services: service_infos,
        modules,
        settings,
        rate_limiter: LoginRateLimiter::default(),
    };

    // --- gateway-wide routes ---
    let app: Router = router
        .route("/health", get(health_handler))
        .route("/events", get(sse::sse_handler))
        .route("/auth/login", post(auth::login_handler))
        .route(
            "/auth/protected",
            get(protected_handler).route_layer(middleware::from_fn_with_state(
                state.clone(),
                auth::auth_middleware,
            )),
        )
        .route("/", get(root_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    Ok(app)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Per-module health probe timeout. A single hung module must not be able to
/// stall the aggregated `/health` response indefinitely; load balancers will
/// otherwise drain the gateway out of rotation on the slowest service.
const HEALTH_CHECK_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Aggregate health check — probes every registered service module in
/// parallel, each capped at [`HEALTH_CHECK_TIMEOUT`].
#[instrument(skip(state))]
async fn health_handler(State(state): State<GatewayState>) -> Json<Value> {
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

/// Root endpoint — returns the list of available services.
async fn root_handler(State(state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "gateway": "Gateway Example",
        "version": env!("CARGO_PKG_VERSION"),
        "services": state.services,
    }))
}

/// Protected endpoint — requires a valid JWT.
#[instrument(skip(_claims))]
async fn protected_handler(Extension(_claims): Extension<Claims>) -> Json<Value> {
    Json(json!({
        "status": "ok",
        "protected": true,
    }))
}
