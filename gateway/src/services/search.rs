//! Mock search service simulating a SearXRS2-style API endpoint.
//!
//! # DTOs
//!
//! All response types implement [`Serialize`], [`Deserialize`], and
//! [`utoipa::ToSchema`] for `OpenAPI` documentation.

use axum::{Router, extract::State, response::Json, routing::get};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::gateway::GatewayState;
use crate::module::ServiceModule;

#[derive(Debug)]
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

// ---------------------------------------------------------------------------
// DTOs
// ---------------------------------------------------------------------------

/// A single search result item.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchResultItem {
    /// Result title.
    pub title: String,
    /// Result URL.
    pub url: String,
    /// Relevance score (0–1).
    pub score: f64,
}

/// Search response payload.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct SearchResponse {
    /// The search query string.
    pub query: String,
    /// List of search result items.
    pub results: Vec<SearchResultItem>,
    /// Total number of results.
    pub total: usize,
}

/// Health check response.
#[derive(Debug, Serialize, Deserialize, ToSchema)]
pub struct HealthResponse {
    pub status: String,
    pub service: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Mock search endpoint — returns hardcoded results.
#[utoipa::path(
    get,
    path = "/search/",
    responses(
        (status = 200, description = "Search results", body = SearchResponse),
    ),
    tag = "search",
)]
async fn search_handler(State(_state): State<GatewayState>) -> Json<SearchResponse> {
    Json(SearchResponse {
        query: "mock search results".to_string(),
        results: vec![
            SearchResultItem {
                title: "Rust async patterns".to_string(),
                url: "https://example.com/1".to_string(),
                score: 0.95,
            },
            SearchResultItem {
                title: "Axum gateway guide".to_string(),
                url: "https://example.com/2".to_string(),
                score: 0.87,
            },
            SearchResultItem {
                title: "ServiceModule trait".to_string(),
                url: "https://example.com/3".to_string(),
                score: 0.72,
            },
        ],
        total: 3,
    })
}

/// Search service health check.
#[utoipa::path(
    get,
    path = "/search/health",
    responses(
        (status = 200, description = "Search service healthy", body = HealthResponse),
    ),
    tag = "search",
)]
async fn search_health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok".to_string(),
        service: "search".to_string(),
    })
}
