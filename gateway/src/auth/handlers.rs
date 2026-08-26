//! Auth handler functions: login, refresh, logout, and protected.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * The demo password is bound to exactly one configured admin identity.
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

#[derive(Debug, Deserialize, Validate, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    /// User identifier. In this demo it must equal configured `ADMIN_USER_ID`.
    #[validate(length(min = 1, max = 255))]
    pub user_id: String,
    #[validate(length(min = 12, max = 1024))]
    pub password: String,
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
pub async fn login_handler(
    State(state): State<GatewayState>,
    session: tower_sessions::Session,
    axum_valid::Valid(axum::Json(LoginRequest { user_id, password })): axum_valid::Valid<
        axum::Json<LoginRequest>,
    >,
) -> Result<Json<LoginResponse>, AppError> {
    let s = &state.settings;
    let parsed_user_id = UserId::from_str(&user_id)?;

    // The intentionally-small demo has one configured credential pair. A
    // shared password must not become a capability to choose an arbitrary JWT
    // subject simply by submitting a different syntactically-valid UUID.
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
            std::io::Error::other("DATABASE_URL not configured; gateway cannot issue refresh tokens"),
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
