//! `OpenAPI` documentation generation via `utoipa`.
//!
//! The [`ApiDoc`] struct aggregates all endpoints and schemas from every
//! gateway module: health, auth, search, proxy, and monitor.  The
//! [`swagger_ui_router`] function mounts Swagger UI at `/docs`.

use axum::Router;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// `OpenAPI` specification for the gateway-example API.
#[derive(OpenApi)]
#[openapi(
    paths(
        // Gateway
        crate::gateway::health_handler,
        // Auth
        crate::auth::handlers::login_handler,
        crate::auth::handlers::refresh_handler,
        crate::auth::handlers::logout_handler,
        crate::auth::handlers::protected_handler,
        // Search service
        crate::services::search::search_handler,
        crate::services::search::search_health,
        // Proxy service
        crate::services::proxy::check_handler,
        crate::services::proxy::check_history_handler,
        crate::services::proxy::proxy_health,
        // Monitor service
        crate::services::monitor::dashboard_handler,
        crate::services::monitor::monitor_health,
    ),
    components(schemas(
        // Auth DTOs
        crate::auth::handlers::LoginRequest,
        crate::auth::handlers::LoginResponse,
        crate::auth::handlers::RefreshResponse,
        crate::auth::handlers::LogoutResponse,
        crate::auth::handlers::ProtectedResponse,
        // Search DTOs
        crate::services::search::SearchResponse,
        crate::services::search::SearchResultItem,
        crate::services::search::HealthResponse,
        // Proxy DTOs
        crate::services::proxy::ProxyCheckResponse,
        crate::services::proxy::ProxyHistoryResponse,
        crate::services::proxy::HistoryEntry,
        crate::services::proxy::ProxyHealthResponse,
        // Monitor DTOs
        crate::services::monitor::MonitorHealthResponse,
    )),
    tags(
        (name = "gateway", description = "Gateway API — health and discovery"),
        (name = "auth", description = "Authentication — JWT login, refresh, logout"),
        (name = "search", description = "Search service — mock full-text search"),
        (name = "proxy", description = "Proxy service — IP proxy / VPN check"),
        (name = "monitor", description = "Monitor service — status dashboard"),
    )
)]
#[derive(Debug)]
pub struct ApiDoc;

/// Build a router that serves Swagger UI at `/docs` using the generated
/// [`ApiDoc`] `OpenAPI` spec.
pub fn swagger_ui_router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    SwaggerUi::new("/docs")
        .url("/api-docs.json", ApiDoc::openapi())
        .into()
}
