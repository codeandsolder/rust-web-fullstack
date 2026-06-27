use axum::{Router, extract::State, response::Json, routing::get};
use serde_json::{Value, json};

use crate::gateway::GatewayState;
use crate::module::ServiceModule;

pub struct SearchService;

impl ServiceModule for SearchService {
    fn name(&self) -> &'static str {
        "search"
    }

    fn description(&self) -> &'static str {
        "Mock search service — simulates a SearXRS2-style API endpoint"
    }

    fn router(&self) -> Router<GatewayState> {
        Router::new()
            .route("/", get(search_handler))
            .route("/health", get(search_health))
    }
}

async fn search_handler(State(_state): State<GatewayState>) -> Json<Value> {
    Json(json!({
        "query": "mock search results",
        "results": [
            {"title": "Rust async patterns", "url": "https://example.com/1", "score": 0.95},
            {"title": "Axum gateway guide",   "url": "https://example.com/2", "score": 0.87},
            {"title": "ServiceModule trait",   "url": "https://example.com/3", "score": 0.72},
        ],
        "total": 3,
    }))
}

async fn search_health() -> Json<Value> {
    Json(json!({"status": "ok", "service": "search"}))
}
