//! Mock proxy / VPN check service.
//!
//! Simulates IP proxy checks and publishes SSE events with RFC3339
//! timestamps for live dashboards.
//!
//! # DTOs
//!
//! All response types implement [`Serialize`], [`Deserialize`], and
//! [`utoipa::ToSchema`] for `OpenAPI` documentation.

use axum::{Router, extract::State, response::Json, routing::get};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::gateway::GatewayState;
use crate::module::ServiceModule;
use crate::sse::{self, GatewayEvent};

#[derive(Debug)]
pub struct ProxyService;

impl ServiceModule for ProxyService {
    fn name(&self) -> &'static str {
        "proxy"
    }

    fn description(&self) -> &'static str {
        "Mock proxy / VPN check service"
    }

    fn router(&self) -> Router<GatewayState> {
        Router::new()
            .route("/check", get(check_handler))
            .route("/check/history", get(check_history_handler))
            .route("/health", get(proxy_health))
    }
}

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// Proxy check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyCheckResponse {
    /// The checked IP address.
    pub ip: String,
    /// Country code.
    pub country: String,
    /// Whether the IP is a known proxy / VPN.
    pub proxy: bool,
    /// Risk score (0–1).
    pub risk_score: f64,
}

/// A single history entry.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HistoryEntry {
    /// RFC3339 timestamp.
    pub timestamp: String,
    /// Check status.
    pub status: String,
}

/// Proxy check history response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyHistoryResponse {
    /// List of historical check results.
    pub history: Vec<HistoryEntry>,
}

/// Health check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyHealthResponse {
    pub status: String,
    pub service: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Run a mock proxy check and publish an SSE event.
#[utoipa::path(
    get,
    path = "/proxy/check",
    responses(
        (status = 200, description = "Proxy check result", body = ProxyCheckResponse),
    ),
    tag = "proxy",
)]
async fn check_handler(State(state): State<GatewayState>) -> Json<ProxyCheckResponse> {
    // Each check publishes an SSE event so live dashboards can react.
    sse::publish_event(
        &state.tx,
        GatewayEvent::Custom(
            "proxy_check",
            json!({
                "timestamp": Utc::now().to_rfc3339(),
                "status": "ok",
            }),
        ),
    );

    Json(ProxyCheckResponse {
        ip: "192.168.1.1".to_string(),
        country: "US".to_string(),
        proxy: false,
        risk_score: 0.02,
    })
}

/// Return mock proxy check history.
#[utoipa::path(
    get,
    path = "/proxy/check/history",
    responses(
        (status = 200, description = "Proxy check history", body = ProxyHistoryResponse),
    ),
    tag = "proxy",
)]
async fn check_history_handler() -> Json<ProxyHistoryResponse> {
    Json(ProxyHistoryResponse {
        history: vec![
            HistoryEntry {
                timestamp: "2024-01-01T00:00:00Z".to_string(),
                status: "ok".to_string(),
            },
            HistoryEntry {
                timestamp: "2024-01-02T00:00:00Z".to_string(),
                status: "ok".to_string(),
            },
        ],
    })
}

/// Proxy service health check.
#[utoipa::path(
    get,
    path = "/proxy/health",
    responses(
        (status = 200, description = "Proxy service healthy", body = ProxyHealthResponse),
    ),
    tag = "proxy",
)]
async fn proxy_health() -> Json<ProxyHealthResponse> {
    Json(ProxyHealthResponse {
        status: "ok".to_string(),
        service: "proxy".to_string(),
    })
}
