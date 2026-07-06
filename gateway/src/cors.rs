//! Shared CORS and CSP configuration for gateway and tests.

use axum::http::{HeaderName, HeaderValue};
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

/// Build a CSP middleware suitable for the gateway's HTML pages.
///
/// Default policy:
/// - `default-src 'self'`
/// - `script-src 'self' 'unsafe-inline'` (Leptos SSR hydration)
/// - `style-src 'self' 'unsafe-inline'`
/// - `img-src 'self' data:`
/// - `connect-src 'self' ws: wss:` (WebSocket + `EventSource`)
/// - `frame-ancestors 'none'`
///
/// Implemented as a plain `axum::middleware::from_fn` rather than
/// `tower_http::set_header::SetResponseHeaderLayer` because the latter's
/// body-type bound (Body: Clone) is not satisfied by axum 0.8's
/// `Body` type. The `if-not-present` semantics of the layer are
/// preserved by checking for an existing header before inserting.
pub async fn csp_middleware(
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    const POLICY: HeaderValue = HeaderValue::from_static(
        "default-src 'self'; \
         script-src 'self' 'unsafe-inline'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; \
         connect-src 'self' ws: wss:; \
         frame-ancestors 'none'",
    );
    const HEADER: HeaderName = HeaderName::from_static("content-security-policy");

    let mut response = next.run(request).await;
    if !response.headers().contains_key(&HEADER) {
        response.headers_mut().insert(HEADER, POLICY);
    }
    response
}
