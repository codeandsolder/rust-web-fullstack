//! Monitor service — renders a simple status dashboard.
//!
//! The dashboard endpoint now redirects users to the `/health` endpoint
//! instead of showing a hardcoded status page, ensuring displayed status
//! reflects actual service health.

use axum::{
    Router,
    response::{Html, Json},
    routing::get,
};
use serde_json::{Value, json};

use crate::gateway::GatewayState;
use crate::module::ServiceModule;

/// Static HTML page that redirects to `/health`.
const REDIRECT_PAGE: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta http-equiv="refresh" content="0; url=/health">
  <title>Gateway Monitor</title>
</head>
<body>
  <p>Service status has moved to the <a href="/health">/health</a> endpoint.</p>
</body>
</html>"#;

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

/// Renders a redirect page pointing to `/health`.
async fn dashboard_handler() -> Html<&'static str> {
    Html(REDIRECT_PAGE)
}

async fn monitor_health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "monitor"}))
}
