//! Shared CORS and CSP configuration for gateway and tests.
//!
//! Explicitly allowed origins are credential-enabled so the session-cookie
//! example works across the documented development origins. The wildcard
//! debug mode intentionally does not allow credentials: browsers forbid
//! combining credentialed CORS with wildcard origins/methods/headers.

use axum::http::{HeaderName, HeaderValue};
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};

/// Build credentialed CORS for a concrete origin allowlist.
fn credentialed(origins: Vec<HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        // With credentials, tower-http rejects wildcard methods/headers.
        // Mirroring the preflight request keeps the demo general without
        // producing an invalid `*` + credentials response.
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .allow_origin(origins)
}

/// Default CORS allowlist used when `ALLOWED_ORIGINS` is unset or empty.
fn localhost_default() -> CorsLayer {
    credentialed(
        [
            "http://localhost:3000",
            "http://localhost:3001",
            "http://localhost:3002",
        ]
        .iter()
        .filter_map(|s| s.parse().ok())
        .collect(),
    )
}

/// Build the canonical CORS layer.
///
/// Resolution:
/// - unset/empty → credentialed localhost development allowlist;
/// - `*` → non-credentialed permissive debug mode;
/// - comma-separated origins → credentialed explicit allowlist.
pub fn cors_layer() -> CorsLayer {
    let raw = std::env::var("ALLOWED_ORIGINS").unwrap_or_default();
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return localhost_default();
    }

    if trimmed == "*" {
        tracing::warn!(
            "CORS: ALLOWED_ORIGINS=* permits any origin and disables credentialed CORS. \
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
        .filter_map(|origin| match origin.parse() {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::error!(origin, error = %e, "ignoring invalid CORS origin");
                None
            }
        })
        .collect();

    if origins.is_empty() {
        // Fail closed: an invalid explicit allowlist should not silently turn
        // into the localhost default or an allow-any policy.
        tracing::error!("ALLOWED_ORIGINS contained no valid origins; CORS will deny all origins");
    }

    credentialed(origins)
}

/// Build a CSP middleware suitable for the gateway's HTML pages.
///
/// The showcase still permits inline scripts/styles because Swagger/SSR demo
/// pages rely on them. It does **not** require `unsafe-eval`. The remaining
/// baseline directives explicitly deny plugins/embedding and restrict base/form
/// targets. Production applications should replace inline allowances with
/// nonces or hashes.
#[allow(
    clippy::unused_async,
    reason = "axum middleware handlers await next.run(request)"
)]
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
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'none'; \
         form-action 'self'",
    );
    const HEADER: HeaderName = HeaderName::from_static("content-security-policy");

    // A small custom middleware is kept for explicit "if not present"
    // semantics. `SetResponseHeaderLayer` would also work; axum's Body does
    // not need to implement Clone for that layer.
    let mut response = next.run(request).await;
    if !response.headers().contains_key(&HEADER) {
        response.headers_mut().insert(HEADER, POLICY);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, Response, StatusCode};
    use axum::middleware::from_fn;
    use axum::routing::get;
    use tower::ServiceExt;

    const POLICY: &str = "default-src 'self'; \
         script-src 'self' 'unsafe-inline'; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:; \
         connect-src 'self' ws: wss:; \
         object-src 'none'; \
         base-uri 'self'; \
         frame-ancestors 'none'; \
         form-action 'self'";

    #[expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "synthetic test response/request construction"
    )]
    #[tokio::test]
    async fn csp_middleware_inserts_default_header() {
        async fn handler() -> Response<Body> {
            Response::builder()
                .status(StatusCode::OK)
                .body(Body::empty())
                .expect("static response build")
        }

        let app = Router::new()
            .route("/", get(handler))
            .layer(from_fn(csp_middleware));
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let header = response
            .headers()
            .get("content-security-policy")
            .expect("CSP header missing");
        assert_eq!(header.to_str().unwrap(), POLICY);
    }

    #[expect(
        clippy::expect_used,
        clippy::unwrap_used,
        reason = "synthetic test response/request construction"
    )]
    #[tokio::test]
    async fn csp_middleware_does_not_override_existing_header() {
        async fn handler() -> Response<Body> {
            Response::builder()
                .status(StatusCode::OK)
                .header("content-security-policy", "default-src 'none'")
                .body(Body::empty())
                .expect("static response build")
        }

        let app = Router::new()
            .route("/", get(handler))
            .layer(from_fn(csp_middleware));
        let response = app
            .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(
            response
                .headers()
                .get("content-security-policy")
                .unwrap()
                .to_str()
                .unwrap(),
            "default-src 'none'",
        );
    }
}
