//! Monitor service — renders a simple status dashboard.
//!
//! The dashboard endpoint now redirects users to the `/health` endpoint
//! instead of showing a hardcoded status page, ensuring displayed status
//! reflects actual service health.
//!
//! # DTOs
//!
//! Response types implement [`Serialize`], [`Deserialize`], and
//! [`utoipa::ToSchema`] for `OpenAPI` documentation (except HTML responses
//! which are excluded from the `OpenAPI` schema).

use axum::{
    Router,
    response::{Json, Redirect},
    routing::get,
};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gateway::GatewayState;
use crate::module::ServiceModule;

#[derive(Debug)]
pub struct MonitorService;

impl ServiceModule for MonitorService {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn description(&self) -> &'static str {
        "Mock monitor dashboard — redirects to /health"
    }

    fn enabled(&self) -> bool {
        true
    }

    fn router(&self) -> Router<GatewayState> {
        Router::new()
            .route("/dashboard", get(dashboard_handler))
            .route("/health", get(monitor_health))
    }
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Monitor health check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct MonitorHealthResponse {
    pub status: String,
    pub service: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Redirects to the aggregate `/health` endpoint.
#[utoipa::path(
    get,
    path = "/monitor/dashboard",
    responses(
        (status = 303, description = "Redirect (303 SEE_OTHER) to /health"),
    ),
    tag = "monitor",
)]
async fn dashboard_handler() -> Redirect {
    Redirect::to("/health")
}

/// Monitor service health check.
#[utoipa::path(
    get,
    path = "/monitor/health",
    responses(
        (status = 200, description = "Monitor service healthy", body = MonitorHealthResponse),
    ),
    tag = "monitor",
)]
async fn monitor_health() -> Json<MonitorHealthResponse> {
    Json(MonitorHealthResponse {
        status: "ok".to_string(),
        service: "monitor".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::http::StatusCode;
    use tower::ServiceExt;

    use super::*;

    #[tokio::test]
    async fn dashboard_redirects_to_health() -> anyhow::Result<()> {
        let settings = crate::settings::Settings::load_dev_keys("test-admin-password")?;
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()?;
        let state = crate::gateway::GatewayState {
            db_pool: None,
            tx: tokio::sync::broadcast::channel(16).0,
            services: vec![],
            modules: vec![],
            settings,
            proxy_upstream_url: Arc::from("https://ipapi.co"),
            // 30 days, matching refresh.rs::REFRESH_TOKEN_TTL_SECONDS.
            refresh_token_ttl_secs: 60 * 60 * 24 * 30,
            // 24h for the test fixture; production uses 15min via Settings.
            access_token_ttl_secs: 60 * 60 * 24,
            http_client,
        };

        let app = MonitorService.router().with_state(state);
        let response = app
            .oneshot(
                axum::http::Request::builder()
                    .uri("/dashboard")
                    .body(axum::body::Body::empty())?,
            )
            .await?;

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get("location")
                .and_then(|v| v.to_str().ok()),
            Some("/health"),
        );
        Ok(())
    }
}
