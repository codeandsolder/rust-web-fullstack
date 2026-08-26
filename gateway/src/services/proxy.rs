//! Proxy / VPN check service.
//!
//! Forwards IP geolocation / threat queries to a configurable upstream API
//! (default: <https://ipapi.co>) and publishes SSE events for live dashboards.

use std::net::{IpAddr, Ipv4Addr};

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

/// Typed query parameters for `/proxy/check`.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ProxyCheckQuery {
    /// IP to inspect. Defaults to Google's public DNS address for the demo.
    pub ip: Option<IpAddr>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyCheckResponse {
    pub ip: String,
    pub country: String,
    pub proxy: bool,
    pub risk_score: f64,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HistoryEntry {
    pub timestamp: String,
    pub status: String,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyHistoryResponse {
    pub history: Vec<HistoryEntry>,
}

#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct ProxyHealthResponse {
    pub status: String,
    pub service: String,
}

#[utoipa::path(
    get,
    path = "/proxy/check",
    params(ProxyCheckQuery),
    responses(
        (status = 200, description = "Proxy check result", body = ProxyCheckResponse),
        (status = 400, description = "Invalid request parameters"),
        (status = 502, description = "Upstream API unavailable"),
    ),
    tag = "proxy",
)]
async fn check_handler(
    State(state): State<GatewayState>,
    Query(params): Query<ProxyCheckQuery>,
) -> Result<Json<ProxyCheckResponse>, AppError> {
    let parsed_ip = params
        .ip
        .unwrap_or(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));

    let base = state.proxy_upstream_url.trim_end_matches('/');
    let url = format!("{base}/{parsed_ip}/json/");
    let upstream = state
        .http_client
        .get(&url)
        .send()
        .await
        .map_err(|e| AppError::upstream("proxy request failed", e))?;

    if !upstream.status().is_success() {
        return Err(AppError::upstream(
            "proxy upstream returned non-success status",
            std::io::Error::other(format!("status {}", upstream.status())),
        ));
    }

    let body: serde_json::Value = upstream
        .json()
        .await
        .map_err(|e| AppError::upstream("proxy upstream response parse failed", e))?;

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

    sse::publish_event(
        &state.tx,
        GatewayEvent::Custom(
            "proxy_check",
            json!({
                "timestamp": Utc::now().to_rfc3339(),
                "ip": parsed_ip.to_string(),
                "country": country,
                "proxy": proxy,
            }),
        ),
    );

    Ok(Json(ProxyCheckResponse {
        ip: parsed_ip.to_string(),
        country,
        proxy,
        risk_score,
    }))
}

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

#[utoipa::path(
    get,
    path = "/proxy/health",
    responses(
        (status = 200, description = "Proxy service process healthy", body = ProxyHealthResponse),
    ),
    tag = "proxy",
)]
async fn proxy_health() -> Json<ProxyHealthResponse> {
    Json(ProxyHealthResponse {
        status: "ok".to_string(),
        service: "proxy".to_string(),
    })
}
