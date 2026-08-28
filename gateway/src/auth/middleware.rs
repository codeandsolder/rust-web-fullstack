//! Auth middleware — validates Bearer JWT tokens on protected routes.
//!
//! The middleware extracts the `Authorization` header, strips the `Bearer `
//! prefix, validates the token using the configured `EdDSA` public key, and
//! injects the parsed [`super::jwt::Claims`] into request extensions.
//!
//! Successful `/auth/logout` requests also flush the server-side session so
//! the gateway's Bearer-token and cookie-session auth mechanisms cannot remain
//! authenticated independently after the unified logout endpoint succeeds.
//!
//! # Usage
//!
//! ```ignore
//! .route_layer(middleware::from_fn_with_state(state, auth::auth_middleware))
//! ```

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use tower_sessions::Session;

use super::error::AppError;
use super::jwt::validate_jwt;
use crate::gateway::GatewayState;

/// Axum middleware that validates a `Bearer` JWT from the `Authorization`
/// header.
///
/// On success the request is forwarded with [`super::jwt::Claims`] injected
/// into the request extensions (accessible via `Extension<Claims>`). On a
/// successful `/auth/logout` response, the current cookie session is flushed
/// before returning success to the client. On authentication failure a 401 is
/// returned; a session-store failure during logout is returned as a 500.
///
/// Apply it with:
/// ```ignore
/// .route_layer(middleware::from_fn_with_state(state, auth::auth_middleware))
/// ```
pub async fn auth_middleware(
    State(state): State<GatewayState>,
    session: Session,
    mut request: Request,
    next: Next,
) -> Response {
    let is_logout = request.uri().path() == "/auth/logout";
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let claims = auth_header
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(
            |token| match validate_jwt(token, &state.settings.decoding_key) {
                Ok(claims) => Some(claims),
                Err(e) => {
                    // Log the validation failure at debug! — routine attacker
                    // probing with bad tokens would flood warn!. We use `%e`
                    // (Display) on `AppError` (constant strings, no token payload)
                    // rather than `?e` (Debug) to be defensive against upstream
                    // `jsonwebtoken::Error` formatting changes that might one day
                    // echo payload bytes.
                    tracing::debug!(error = %e, "JWT validation failed in middleware");
                    None
                }
            },
        );

    let Some(claims) = claims else {
        return AppError::AuthError.into_response();
    };

    // Inject claims into request extensions so downstream handlers can
    // extract them via `Extension<Claims>`.
    request.extensions_mut().insert(claims);

    let response = next.run(request).await;
    if is_logout && response.status().is_success() {
        if let Err(error) = session.flush().await {
            return AppError::internal("session logout", error).into_response();
        }
    }

    response
}
