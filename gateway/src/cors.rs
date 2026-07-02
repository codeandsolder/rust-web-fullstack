//! Shared CORS configuration for gateway and tests.

use axum::http::HeaderValue;
use tower_http::cors::{Any, CorsLayer};

/// Default CORS allowlist used when `ALLOWED_ORIGINS` is unset or empty.
///
/// Matches the three ports the showcase binds (live-search :3000,
/// gateway :3001, i18n-demo :3002). Hard-coded rather than env-driven
/// because it's the documented dev default — overriding via env is the
/// user's choice.
fn localhost_default() -> CorsLayer {
    CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(
            [
                "http://localhost:3000",
                "http://localhost:3001",
                "http://localhost:3002",
            ]
            .iter()
            .filter_map(|s| s.parse().ok())
            .collect::<Vec<HeaderValue>>(),
        )
}

/// Build the canonical CORS layer.
///
/// Resolution:
/// - `ALLOWED_ORIGINS` unset or empty → dev default localhost allowlist.
/// - `ALLOWED_ORIGINS=*` → `Any` origin (debug only; emits a `tracing::warn!`).
/// - `ALLOWED_ORIGINS=https://a.com,https://b.com` → parsed list.
///
/// Unparseable entries in the comma list are dropped (not failed at
/// startup) so a typo doesn't take the gateway down.
pub fn cors_layer() -> CorsLayer {
    let raw = std::env::var("ALLOWED_ORIGINS").unwrap_or_default();
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return localhost_default();
    }

    if trimmed == "*" {
        // Loud, single-shot warning so this isn't a silent footgun in prod.
        tracing::warn!(
            "CORS: ALLOWED_ORIGINS=* permits ANY origin. \
             This is a debug-only mode; do not use in production."
        );
        return CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any);
    }

    let origins: Vec<HeaderValue> = trimmed
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse().ok())
        .collect();
    CorsLayer::new()
        .allow_methods(Any)
        .allow_headers(Any)
        .allow_origin(origins)
}
