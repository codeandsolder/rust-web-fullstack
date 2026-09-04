//! Auth handler functions: login, refresh, logout, and protected.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * The demo password is bound to exactly one configured admin identity.
//! * Rate limiting is applied at the router level via `tower_governor`.
//! * JWT errors are mapped to distinct error variants.
//! * Secrets (passwords, tokens) are never included in log output.
//! * Public auth request payloads are typed and validated with `axum-valid`.

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

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User identifier. In this demo it must equal configured `ADMIN_USER_ID`.
    #[validate(length(min = 1, max = 255))]
    pub user_id: String,
    #[validate(length(min = 12, max = 1024))]
    pub password: String,
}

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    /// Opaque refresh token returned by login or the previous refresh.
    #[validate(length(min = 43, max = 1024))]
    pub refresh_token: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LoginResponse {
    pub token: String,
    pub refresh_token: String,
    pub user_id: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RefreshResponse {
    pub token: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogoutResponse {
    pub status: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ProtectedResponse {
    pub status: String,
    pub protected: bool,
}

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
/// Authenticate the configured admin and create access/refresh credentials.
///
/// # Errors
/// Returns an authentication error for an invalid user ID or password, and an
/// internal error if JWT creation, refresh-token persistence, or session-store
/// persistence fails.
pub async fn login_handler(
    State(state): State<GatewayState>,
    session: tower_sessions::Session,
    axum_valid::Valid(axum::Json(LoginRequest { user_id, password })): axum_valid::Valid<
        axum::Json<LoginRequest>,
    >,
) -> Result<Json<LoginResponse>, AppError> {
    let s = &state.settings;
    let parsed_user_id = UserId::from_str(&user_id)?;

    if parsed_user_id != s.admin_user_id {
        return Err(AppError::AuthError);
    }

    let password_match: bool = password
        .as_bytes()
        .ct_eq(s.default_admin_password.as_bytes())
        .into();
    if !password_match {
        return Err(AppError::AuthError);
    }

    let token = create_jwt(&parsed_user_id, &s.encoding_key, s.access_token_ttl_secs)?;

    let pool = state.db_pool.as_ref().ok_or_else(|| {
        AppError::internal(
            "refresh-token store unavailable",
            std::io::Error::other(
                "DATABASE_URL not configured; gateway cannot issue refresh tokens",
            ),
        )
    })?;

    let (raw_refresh, refresh_jti) = generate_raw_refresh_token()
        .map_err(|e| AppError::internal("refresh token generation", e))?;

    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::seconds(state.refresh_token_ttl_secs);
    let hashed = hash_refresh_token(&raw_refresh).to_vec();

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

    // A pre-authentication session ID may already be known to another party
    // (for example after bootstrapping a CSRF token). Rotate it before attaching
    // authenticated state so possession of the old ID cannot inherit the login.
    if session.id().is_some()
        && let Err(error) = session.cycle_id().await
    {
        remove_unissued_refresh_token(pool, refresh_jti).await;
        return Err(AppError::internal("session id rotation", error));
    }

    if let Err(error) = session
        .insert(
            crate::session::SESSION_USER_KEY,
            crate::session::SessionUser {
                user_id: user_id.clone(),
            },
        )
        .await
    {
        remove_unissued_refresh_token(pool, refresh_jti).await;
        return Err(AppError::internal("session insert", error));
    }

    // tower-sessions normally persists modified state after the handler returns.
    // Persist once here as well so a store failure is observable while we can
    // still compensate the refresh-token insert rather than returning a valid
    // refresh credential for a login whose authenticated session was not saved.
    if let Err(error) = session.save().await {
        // The outer SessionManagerLayer will still inspect this Session after the
        // handler returns. Remove authenticated in-memory state first so a
        // transient retry by that middleware cannot turn this 500 response into
        // a successfully persisted authenticated session/cookie.
        session.clear().await;
        remove_unissued_refresh_token(pool, refresh_jti).await;
        return Err(AppError::internal("session save", error));
    }

    Ok(Json(LoginResponse {
        token,
        refresh_token: raw_refresh,
        user_id,
    }))
}

/// Remove a refresh credential that was persisted for a login which could not
/// complete its session transition. Cleanup is best-effort because the original
/// session-store error remains the request's primary failure.
async fn remove_unissued_refresh_token(pool: &sqlx::PgPool, refresh_jti: Uuid) {
    if let Err(error) = sqlx::query("DELETE FROM refresh_tokens WHERE jti = $1")
        .bind(refresh_jti)
        .execute(pool)
        .await
    {
        tracing::error!(
            refresh_jti = %refresh_jti,
            error = %error,
            "failed to remove refresh token after login session failure"
        );
    }
}

#[utoipa::path(
    post,
    path = "/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Token refreshed", body = RefreshResponse),
        (status = 401, description = "Invalid refresh token"),
        (status = 500, description = "Refresh store unavailable"),
    ),
    tag = "auth",
)]
/// Rotate a refresh token and issue a new access token.
///
/// # Errors
/// Returns an authentication error for an invalid, expired, revoked, or
/// replayed refresh token, and an internal error if token rotation, database
/// access, or JWT creation fails.
pub async fn refresh_handler(
    State(state): State<GatewayState>,
    axum_valid::Valid(axum::Json(RefreshRequest { refresh_token })): axum_valid::Valid<
        axum::Json<RefreshRequest>,
    >,
) -> Result<Json<RefreshResponse>, AppError> {
    let pool = state.db_pool.as_ref().ok_or_else(|| {
        AppError::internal(
            "refresh-token store unavailable",
            std::io::Error::other("DATABASE_URL not configured; /auth/refresh cannot operate"),
        )
    })?;

    let now = chrono::Utc::now();
    let rotation = super::refresh::rotate(pool, &refresh_token, now, state.refresh_token_ttl_secs)
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

#[utoipa::path(
    post,
    path = "/auth/logout",
    responses(
        (status = 200, description = "Subject refresh tokens revoked and current cookie session flushed", body = LogoutResponse),
        (status = 401, description = "Invalid token"),
    ),
    tag = "auth",
)]
/// Revoke every outstanding refresh token for the authenticated subject.
///
/// After this handler succeeds, auth middleware flushes the cookie session
/// attached to this request. Other cookie sessions for the same subject are
/// not enumerated or flushed, and already-issued access JWTs remain valid
/// until `exp` unless a separate access-token revocation mechanism is added.
///
/// # Errors
/// Returns an internal error if the refresh-token store is unavailable or the
/// revocation query fails.
pub async fn logout_handler(
    State(state): State<GatewayState>,
    Extension(claims): Extension<Claims>,
) -> Result<Json<LogoutResponse>, AppError> {
    let pool = state.db_pool.as_ref().ok_or_else(|| {
        AppError::internal(
            "refresh store unavailable",
            std::io::Error::other("DATABASE_URL not configured"),
        )
    })?;
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
