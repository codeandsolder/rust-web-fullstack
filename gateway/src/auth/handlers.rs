//! Auth handler functions: login, refresh, logout, and protected.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * Rate limiting is applied at the router level via `tower_governor`.
//! * JWT errors are mapped to distinct error variants.
//! * Secrets (passwords, tokens) are never included in log output.
//! * Login request payloads are validated with `axum-valid` + `validator`.

use std::str::FromStr;

use axum::extract::{Extension, State};
use axum::response::Json;
use rwf_domain::UserId;
use serde::{Deserialize, Serialize};
use subtle::ConstantTimeEq;
use utoipa::ToSchema;
use uuid::Uuid;
use validator::Validate;

use crate::gateway::GatewayState;

use super::error::AppError;
use super::jwt::{Claims, create_jwt};
use super::refresh::{generate_raw_refresh_token, hash_refresh_token};

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Login request payload.
///
/// The password must be at least 12 characters long (max 1024).
#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User identifier (must be a UUID in this demo).
    #[validate(length(min = 1, max = 255))]
    pub user_id: String,
    /// User password.
    #[validate(length(min = 12, max = 1024))]
    pub password: String,
}

/// Login response containing the signed JWT and the initial refresh token.
///
/// Refresh tokens are issued at login and rotated on every successful
/// `/auth/refresh` call. Storing the refresh token in the database (as a
/// SHA-256 hash) and the family-id linkage allows the server to revoke the
/// whole chain if a leaked token is replayed.
#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    /// Signed `EdDSA` access JWT (short-lived, e.g. 15 minutes).
    pub token: String,
    /// Opaque refresh token (long-lived, e.g. 30 days). Send verbatim to
    /// `/auth/refresh` to obtain a new access JWT and a new refresh token.
    pub refresh_token: String,
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
    session: tower_sessions::Session,
    // Validated json extraction via axum-valid; consume the payload by
    // destructuring so the extractor wrappers are not held as borrowed
    // bindings (which would trigger `needless_pass_by_value`).
    axum_valid::Valid(axum::Json(LoginRequest { user_id, password })): axum_valid::Valid<
        axum::Json<LoginRequest>,
    >,
) -> Result<Json<LoginResponse>, AppError> {
    let s = &state.settings;

    // Parse user_id as a valid UserId before signing a JWT. Refuses any
    // non-UUID subject — prevents a single shared `default_admin_password`
    // from authenticating as an arbitrary subject.
    let parsed_user_id = UserId::from_str(&user_id)?;

    // Constant-time password comparison.
    let password_match: bool = password
        .as_bytes()
        .ct_eq(s.default_admin_password.as_bytes())
        .into();
    if !password_match {
        return Err(AppError::AuthError);
    }

    let token = create_jwt(&parsed_user_id, &s.encoding_key, s.access_token_ttl_secs)?;

    // Issue the initial refresh token. The DB-backed path is required for
    // any production-grade rotation; the gateway refuses to start login
    // without a configured DB pool. `generate_raw_refresh_token` is
    // infallible on Linux/macOS/Windows in practice but propagates RNG
    // failure as an internal error rather than panicking.
    let pool = state.db_pool.as_ref().ok_or_else(|| {
        AppError::internal(
            "refresh-token store unavailable",
            std::io::Error::other("DATABASE_URL not configured; gateway cannot issue refresh tokens"),
        )
    })?;

    let (raw_refresh, refresh_jti) = generate_raw_refresh_token()
        .map_err(|e| AppError::internal("refresh token generation", e))?;

    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::seconds(state.refresh_token_ttl_secs);
    let hashed = hash_refresh_token(&raw_refresh).to_vec();

    // The first row in a refresh-token family uses its own jti as
    // `family_id`. Subsequent rotations chain by reusing the same
    // `family_id` (see `refresh::rotate`).
    sqlx::query(
        "INSERT INTO refresh_tokens (jti, family_id, subject, hashed_token, expires_at, created_at) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(refresh_jti)
    .bind(refresh_jti)
    .bind(Uuid::from(parsed_user_id))
    .bind(hashed)
    .bind(expires_at)
    .bind(now)
    .execute(pool)
    .await
    .map_err(|e| AppError::internal("refresh token insert", e))?;

    // Persist the authenticated user in the session cookie.  This enables
    // CSRF-protected session-backed routes (e.g. `/session/whoami`,
    // `/session/logout`) alongside the existing JWT path.
    session
        .insert(
            crate::session::SESSION_USER_KEY,
            crate::session::SessionUser {
                user_id: user_id.clone(),
            },
        )
        .await
        .map_err(|e| AppError::internal("session insert", e))?;

    Ok(Json(LoginResponse {
        token,
        refresh_token: raw_refresh,
        user_id,
    }))
}

/// Refresh a JWT token.
///
/// Body is `{"refresh_token": "<opaque>"}`. The refresh token is rotated
/// atomically (old revoked, new issued in the same transaction). If a
/// previously-rotated token is replayed, the entire family is revoked —
/// see [`super::refresh::rotate`].
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the refresh token is missing, unknown,
/// expired, or replayed. Returns [`AppError::Internal`] for DB failures or
/// when the refresh-token store is not configured.
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
    let pool = state.db_pool.as_ref().ok_or_else(|| {
        AppError::internal(
            "refresh-token store unavailable",
            std::io::Error::other("DATABASE_URL not configured; /auth/refresh cannot operate"),
        )
    })?;

    let raw = body
        .get("refresh_token")
        .and_then(|v| v.as_str())
        .ok_or(AppError::AuthError)?;

    let now = chrono::Utc::now();
    let rotation = super::refresh::rotate(pool, raw, now, state.refresh_token_ttl_secs)
        .await
        .map_err(|e| AppError::internal("refresh-token rotation", e))?
        .ok_or(AppError::AuthError)?;

    let token = create_jwt(
        &rotation.subject,
        &state.settings.encoding_key,
        state.settings.access_token_ttl_secs,
    )?;

    Ok(Json(RefreshResponse {
        token,
        refresh_token: Some(rotation.new_raw_token),
    }))
}

/// Logout — invalidate the current refresh-token family.
///
/// **Semantic**: this endpoint revokes the caller's refresh tokens (the DB
/// row, marked `revoked_at = NOW()`) and any active refresh tokens for the
/// same subject. The access JWT remains valid until its (now short,
/// 15-minute) `exp`; clients should discard both the access JWT and the
/// refresh token. Use [`crate::session::router`]'s `/session/logout` to
/// also flush the session cookie.
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
    Extension(claims): Extension<Claims>,
) -> Result<Json<LogoutResponse>, AppError> {
    let pool = state
        .db_pool
        .as_ref()
        .ok_or_else(|| AppError::internal("refresh store unavailable", std::io::Error::other("DATABASE_URL not configured")))?;
    let revoked = sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = NOW() \
         WHERE subject = $1 AND revoked_at IS NULL",
    )
    .bind(Uuid::from(claims.sub))
    .execute(pool)
    .await
    .map_err(|e| AppError::internal("refresh token revocation", e))?
    .rows_affected();
    tracing::info!(
        subject = %claims.sub,
        revoked_count = revoked,
        "refresh tokens revoked; access JWT remains valid until exp",
    );
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
