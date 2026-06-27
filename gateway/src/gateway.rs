use std::sync::Arc;

use axum::{
    Router,
    extract::State,
    middleware,
    response::Json,
    routing::{get, post},
};
use serde_json::{Value, json};
use tokio::sync::broadcast;
use tower_http::trace::TraceLayer;

use crate::{
    auth,
    module::{ServiceInfo, ServiceModule},
    sse::{self, GatewayEvent},
};

// ---------------------------------------------------------------------------
// Shared gateway state
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct GatewayState {
    /// Broadcast channel for SSE events.
    pub tx: broadcast::Sender<GatewayEvent>,
    /// Read-only service descriptors (for API discovery).
    pub services: Vec<ServiceInfo>,
    /// Module trait objects kept alive for health aggregation.
    pub modules: Vec<Arc<dyn ServiceModule>>,
}

// ---------------------------------------------------------------------------
// Router composition
// ---------------------------------------------------------------------------

/// Compose every `ServiceModule` under its own path prefix and attach
/// gateway-wide routes — `/health`, `/events`, `/auth/login`, and `/`.
pub fn build_gateway(service_modules: Vec<Box<dyn ServiceModule>>) -> Router {
    let (tx, _rx) = broadcast::channel(100);

    // Wrap each module in Arc so we can keep a clone-able reference.
    let modules: Vec<Arc<dyn ServiceModule>> = service_modules.into_iter().map(Arc::from).collect();

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
        modules: modules.clone(),
    };

    // --- gateway-wide routes (note: with_state returns Router<S2>;
    // we assign to a `Router` / `Router<()>` variable to finalise it) ---
    let app: Router = router
        .route("/health", get(health_handler))
        .route("/events", get(sse::sse_handler))
        .route("/auth/login", post(auth::login_handler))
        .route(
            "/auth/protected",
            get(protected_handler).route_layer(middleware::from_fn(auth::auth_middleware)),
        )
        .route("/", get(root_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    app
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Aggregate health check — probes every registered service module.
async fn health_handler(State(state): State<GatewayState>) -> Json<Value> {
    let mut service_statuses = Vec::new();

    for module in &state.modules {
        let status = match module.health_check().await {
            Ok(()) => "healthy",
            Err(e) => {
                tracing::warn!("health check failed for {}: {e}", module.name());
                "unhealthy"
            }
        };
        service_statuses.push(json!({
            "name": module.name(),
            "path": module.path(),
            "enabled": module.enabled(),
            "status": status,
        }));
    }

    Json(json!({
        "gateway": "ok",
        "services": service_statuses,
    }))
}

/// Root endpoint — returns the list of available services.
async fn root_handler(State(state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "gateway": "Gateway Example",
        "version": "0.1.0",
        "services": state.services,
    }))
}

async fn protected_handler() -> Json<Value> {
    Json(json!({
        "status": "ok",
        "protected": true,
    }))
}
