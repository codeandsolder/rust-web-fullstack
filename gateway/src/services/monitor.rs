use axum::{
    Router,
    response::{Html, Json},
    routing::get,
};
use serde_json::{Value, json};

use crate::gateway::GatewayState;
use crate::module::ServiceModule;

const DASHBOARD_HTML: &str = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <title>Gateway Monitor Dashboard</title>
  <style>
    body { font-family: system-ui, sans-serif; max-width: 720px; margin: 2rem auto; }
    h1 { color: #333; }
    .ok { color: #090; }
    ul { list-style: none; padding: 0; }
    li { padding: .5rem; border-bottom: 1px solid #eee; }
  </style>
</head>
<body>
  <h1>🔍 Gateway Monitor Dashboard</h1>
  <p class="ok">All services operational</p>
  <ul>
    <li>✅ Search  – <span class="ok">healthy</span></li>
    <li>✅ Proxy   – <span class="ok">healthy</span></li>
    <li>✅ Monitor – <span class="ok">healthy</span></li>
  </ul>
  <hr>
  <small>Gateway Example v0.1.0</small>
</body>
</html>"#;

pub struct MonitorService;

impl ServiceModule for MonitorService {
    fn name(&self) -> &'static str {
        "monitor"
    }

    fn description(&self) -> &'static str {
        "Mock monitor dashboard – renders an HTML status page"
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

async fn dashboard_handler() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

async fn monitor_health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "monitor"}))
}
