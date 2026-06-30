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
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User identifier (e.g. email or username).
    #[validate(length(min = 1, max = 255))]
    pub user_id: String,
    /// User password.
    #[validate(length(min = 1))]
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
#[allow(clippy::needless_pass_by_value)]
pub async fn login_handler(
    State(state): State<GatewayState>,
    // Validated JSON extraction via axum-valid
    axum_valid::Valid(axum::Json(req)): axum_valid::Valid<axum::Json<LoginRequest>>,
) -> Result<Json<LoginResponse>, AppError> {
    let s = &state.settings;

    // Constant-time password comparison.
    let password_match: bool = req
        .password
        .as_bytes()
        .ct_eq(s.default_admin_password.as_bytes())
        .into();
    if !password_match {
        return Err(AppError::AuthError);
    }

    let token = create_jwt(&req.user_id, &s.jwt_private_key_pem)?;
    Ok(Json(LoginResponse {
        token,
        user_id: req.user_id,
    }))
}

/// Refresh a JWT token.
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the refresh token is missing or
/// invalid.
#[utoipa::path(
    post,
    path = "/auth/refresh",
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResponse),
        (status = 401, description = "Invalid refresh token"),
    ),
    tag = "auth",
)]
pub async fn refresh_handler(
    State(state): State<GatewayState>,
    axum::extract::Json(body): axum::extract::Json<serde_json::Value>,
) -> Result<Json<RefreshResponse>, AppError> {
    // TODO: DB-backed refresh token rotation (Phase 1a post-migration wiring).
    // For now, re-issue using the existing access token claims.
    let token_str = body
        .get("token")
        .and_then(|v| v.as_str())
        .ok_or(AppError::AuthError)?;

    let claims = super::jwt::validate_jwt(token_str, &state.settings.jwt_public_key_pem)?;
    let new_token = create_jwt(&claims.sub, &state.settings.jwt_private_key_pem)?;

    Ok(Json(RefreshResponse { token: new_token }))
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
