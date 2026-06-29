//! Mock proxy / VPN check service.
//!
//! Simulates IP proxy checks and publishes SSE events with RFC3339
//! timestamps for live dashboards.

use axum::{Router, extract::State, response::Json, routing::get};
use chrono::Utc;
use serde_json::{Value, json};

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

async fn check_handler(State(state): State<GatewayState>) -> Json<Value> {
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

    Json(json!({
        "ip": "192.168.1.1",
        "country": "US",
        "proxy": false,
        "risk_score": 0.02,
    }))
}

async fn check_history_handler() -> Json<Value> {
    Json(json!({
        "history": [
            {"timestamp": "2024-01-01T00:00:00Z", "status": "ok"},
            {"timestamp": "2024-01-02T00:00:00Z", "status": "ok"},
        ]
    }))
}

async fn proxy_health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "proxy"}))
}
