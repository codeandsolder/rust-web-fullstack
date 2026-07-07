//! Auth handler functions: login, refresh, logout, and protected.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * Rate limiting is applied at the router level via `tower_governor`.
//! * JWT errors are mapped to distinct error variants.
//! * Secrets (passwords, tokens) are never included in log output.
//! * Login request payloads are validated with `axum-valid` + `validator`.

use axum::extract::{Extension, State};
use axum::response::Json;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;
use validator::Validate;

use crate::gateway::GatewayState;

use super::error::AppError;
use super::jwt::{Claims, create_jwt};

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Login request payload.
///
/// The password must be at least 12 characters long (max 1024).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User identifier (e.g. email or username).
    #[validate(length(min = 1, max = 255))]
    pub user_id: String,
    /// User password.
    #[validate(length(min = 12, max = 1024))]
    pub password: String,
}

/// Login response containing the signed JWT.
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    /// Signed `EdDSA` JWT.
    pub token: String,
    /// Authenticated user identifier.
    pub user_id: String,
}

/// Refresh token response.
#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshResponse {
    /// New signed `EdDSA` JWT.
    pub token: String,
    /// New refresh token (opaque). Rotated on every successful refresh.
    /// Returned only when a DB-backed refresh store is configured
    /// (`GatewayState.db_pool` is `Some`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

/// Logout response.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogoutResponse {
    pub status: String,
}

/// Protected endpoint response.
#[derive(Debug, Serialize, ToSchema)]
pub struct ProtectedResponse {
    pub status: String,
    pub protected: bool,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Authenticate a user with `user_id` and `password`.
///
/// Returns a signed `EdDSA` JWT on success.  The endpoint is rate-limited per
/// source IP by `tower_governor`.
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the password is wrong.
/// Returns [`AppError::Internal`] if JWT signing fails.
#[utoipa::path(
    post,
    path = "/auth/login",
    request_body = LoginRequest,
    responses(
        (status = 200, description = "Login successful", body = LoginResponse),
        (status = 401, description = "Authentication failed"),
    ),
    tag = "auth",
)]
pub async fn login_handler(
    State(state): State<GatewayState>,
    // Validated json extraction via axum-valid; consume the payload by
    // destructuring so the extractor wrappers are not held as borrowed
    // bindings (which would trigger `needless_pass_by_value`).
    axum_valid::Valid(axum::Json(LoginRequest { user_id, password })): axum_valid::Valid<
        axum::Json<LoginRequest>,
    >,
) -> Result<Json<LoginResponse>, AppError> {
    let s = &state.settings;

    // Constant-time password comparison.
    let password_match: bool = password
        .as_bytes()
        .ct_eq(s.default_admin_password.as_bytes())
        .into();
    if !password_match {
        return Err(AppError::AuthError);
    }

    let token = create_jwt(&user_id, &s.encoding_key)?;
    Ok(Json(LoginResponse { token, user_id }))
}

/// Refresh a JWT token.
///
/// If the gateway was started with a DB pool, the request body is
/// interpreted as `{"refresh_token": "<opaque>"}` and the refresh
/// token is rotated atomically (old revoked, new issued). Without a
/// DB pool the legacy semantics are kept: the body is
/// `{"token": "<existing JWT>"}` and the server re-issues a new JWT
/// for the same subject. This dual behaviour lets the example run
/// without `PostgreSQL` while still exercising the production-grade
/// rotation flow when configured.
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the refresh token is missing or
/// invalid. Returns [`AppError::Internal`] for DB failures.
#[utoipa::path(
    post,
    path = "/auth/refresh",
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResponse),
        (status = 401, description = "Invalid refresh token"),
        (status = 503, description = "Refresh store unavailable"),
    ),
    tag = "auth",
)]
pub async fn refresh_handler(
    State(state): State<GatewayState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Json<RefreshResponse>, AppError> {
    // DB-backed rotation path: requires a refresh_token in the body.
    if let Some(pool) = state.db_pool.as_ref() {
        let raw = body
            .get("refresh_token")
            .and_then(|v| v.as_str())
            .ok_or(AppError::AuthError)?;

        let now = chrono::Utc::now();
        let new_raw = super::refresh::rotate(pool, raw, now)
            .await
            .map_err(|e| AppError::internal("refresh-token rotation", e))?
            .ok_or(AppError::AuthError)?;

        // Issue a fresh access JWT. We don't know the subject at this
        // layer without an extra lookup, but rotating always returns
        // a new refresh token; the access JWT is signed for the same
        // subject the user logged in with. The caller is expected to
        // present the JWT they have; we re-use that subject.
        // For this example we sign for the subject encoded in the
        // existing JWT (if any) or "anonymous" otherwise — production
        // callers should chain login() → refresh() without dropping
        // the original claims.
        let subject = body
            .get("token")
            .and_then(|v| v.as_str())
            .and_then(|t| super::jwt::validate_jwt(t, &state.settings.decoding_key).ok())
            .map_or_else(|| "anonymous".to_string(), |claims| claims.sub);
        let token = create_jwt(&subject, &state.settings.encoding_key)?;

        return Ok(Json(RefreshResponse {
            token,
            refresh_token: Some(new_raw),
        }));
    }

    // Legacy fallback (no DB): re-issue using the existing JWT.
    let token_str = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or(AppError::AuthError)?;
    let claims = super::jwt::validate_jwt(token_str, &state.settings.decoding_key)?;
    let token = create_jwt(&claims.sub, &state.settings.encoding_key)?;
    Ok(Json(RefreshResponse {
        token,
        refresh_token: None,
    }))
}

/// Logout — invalidate the current session / token.
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the token is missing or invalid.
#[utoipa::path(
    post,
    path = "/auth/logout",
    responses(
        (status = 200, description = "Logged out", body = LogoutResponse),
        (status = 401, description = "Invalid token"),
    ),
    tag = "auth",
)]
pub async fn logout_handler(
    State(state): State<GatewayState>,
    Extension(_claims): Extension<Claims>,
) -> Result<Json<LogoutResponse>, AppError> {
    // TODO: Blacklist the JWT jti or revoke the refresh token from DB.
    // For now, this is a no-op stub.
    let _ = &state.settings;
    Ok(Json(LogoutResponse {
        status: "ok".to_string(),
    }))
}

/// Protected endpoint — requires a valid JWT.
///
/// Returns the authenticated user's information from the token claims.
#[utoipa::path(
    get,
    path = "/auth/protected",
    responses(
        (status = 200, description = "Access granted", body = ProtectedResponse),
        (status = 401, description = "Unauthorized"),
    ),
    tag = "auth",
)]
pub async fn protected_handler(Extension(_claims): Extension<Claims>) -> Json<ProtectedResponse> {
    Json(ProtectedResponse {
        status: "ok".to_string(),
        protected: true,
    })
}
