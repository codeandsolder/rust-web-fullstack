//! Shared CORS and CSP configuration for gateway and tests.

use axum::http::{HeaderName, HeaderValue};
use tower_http::cors::{AllowHeaders, AllowMethods, Any, CorsLayer};

fn credentialed(origins: Vec<HeaderValue>) -> CorsLayer {
    CorsLayer::new()
        .allow_credentials(true)
        .allow_methods(AllowMethods::mirror_request())
        .allow_headers(AllowHeaders::mirror_request())
        .allow_origin(origins)
}

/// Build CORS from the already-resolved gateway allowlist.
///
/// `*` is deliberately non-credentialed. Concrete origin lists are
/// credential-enabled so session cookies work across configured origins.
#[must_use]
pub fn cors_layer(allowed_origins: &str) -> CorsLayer {
    let trimmed = allowed_origins.trim();

    if trimmed == "*" {
        tracing::warn!(
            "CORS allowlist is `*`: permitting any origin without credentials; debug only"
        );
        return CorsLayer::new()
            .allow_methods(Any)
            .allow_headers(Any)
            .allow_origin(Any);
    }

    let origins: Vec<HeaderValue> = trimmed
        .split(',')
        .map(str::trim)
        .filter(|origin| !origin.is_empty())
        .filter_map(|origin| match origin.parse() {
            Ok(value) => Some(value),
            Err(e) => {
                tracing::error!(origin, error = %e, "ignoring invalid CORS origin");
                None
            }
        })
        .collect();

    if origins.is_empty() {
        tracing::error!("CORS allowlist contains no valid origins; cross-origin requests denied");
    }

    credentialed(origins)
}

/// Add a baseline CSP if the response has not already supplied one.
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

    // SetResponseHeaderLayer would also work; the custom middleware is kept
    // only because the "if not already supplied" behavior is explicit here.
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
