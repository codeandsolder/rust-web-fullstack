//! Proxy / VPN check service.
//!
//! Forwards IP geolocation / threat queries to a configurable upstream API
//! (default: <https://ipapi.co>) and publishes SSE events for live dashboards.
//!
//! The upstream URL is set via the `PROXY_UPSTREAM_URL` environment variable
//! and stored in [`GatewayState::proxy_upstream_url`].
//!
//! # DTOs
//!
//! All response types implement [`Serialize`], [`Deserialize`], and
//! `utoipa::ToSchema` for `OpenAPI` documentation.

use std::collections::HashMap;

use axum::{
    Router,
    extract::{Query, State},
    response::Json,
    routing::get,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;

use crate::auth::AppError;
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
        "IP proxy / VPN check via upstream API"
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
    /// Country name.
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

/// Run an IP proxy check against the configured upstream API and publish an
/// SSE event so live dashboards can react.
///
/// Accepts an optional `?ip=` query parameter (defaults to `8.8.8.8`).
#[utoipa::path(
    get,
    path = "/proxy/check",
    responses(
        (status = 200, description = "Proxy check result", body = ProxyCheckResponse),
        (status = 400, description = "Invalid request parameters"),
        (status = 503, description = "Upstream API unavailable"),
    ),
    tag = "proxy",
)]
async fn check_handler(
    State(state): State<GatewayState>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<Json<ProxyCheckResponse>, AppError> {
    let ip = params
        .get("ip")
        .cloned()
        .unwrap_or_else(|| "8.8.8.8".to_string());

    // Defensive: reject implausibly long IP strings. The upstream API will
    // validate format; this is a cheap early-exit for abuse.
    if ip.len() > 64 {
        return Err(AppError::BadRequest("ip parameter too long".into()));
    }

    let url = format!("{}/{ip}/json/", state.proxy_upstream_url);
    let upstream = state.http_client.get(&url).send().await.map_err(|e| {
        tracing::warn!(error = %e, upstream_url = %url, "upstream fetch failed");
        AppError::internal("upstream fetch failed", e)
    })?;

    if !upstream.status().is_success() {
        return Err(AppError::internal(
            "upstream returned non-2xx",
            std::io::Error::other(format!("upstream returned status {}", upstream.status())),
        ));
    }

    let body: serde_json::Value = upstream
        .json()
        .await
        .map_err(|e| AppError::internal("upstream response parse failed", e))?;

    let country = body
        .get("country_name")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("Unknown")
        .to_string();
    let proxy = body
        .get("threat")
        .and_then(|v| v.get("is_proxy"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let risk_score = body
        .get("threat")
        .and_then(|v| v.get("score"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0);

    // Publish an SSE event for live dashboards
    sse::publish_event(
        &state.tx,
        GatewayEvent::Custom(
            "proxy_check",
            json!({
                "timestamp": Utc::now().to_rfc3339(),
                "ip": ip,
                "country": country,
                "proxy": proxy,
            }),
        ),
    );

    Ok(Json(ProxyCheckResponse {
        ip,
        country,
        proxy,
        risk_score,
    }))
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
