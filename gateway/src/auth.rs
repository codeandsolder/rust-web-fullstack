use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::settings;

// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    pub iat: u64,
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub user_id: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("Authentication failed")]
    AuthError,

    #[error("Internal error: {0}")]
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, msg) = match self {
            Self::AuthError => (StatusCode::UNAUTHORIZED, "Authentication failed"),
            Self::Internal(ref s) => (StatusCode::INTERNAL_SERVER_ERROR, s.as_str()),
        };
        (status, Json(json!({"error": msg}))).into_response()
    }
}

// ---------------------------------------------------------------------------
// JWT helpers
// ---------------------------------------------------------------------------

pub fn create_jwt(user_id: &str) -> Result<String, AppError> {
    let settings = settings::Settings::load();
    let now = Utc::now();

    let claims = Claims {
        sub: user_id.to_string(),
        #[expect(
            clippy::cast_sign_loss,
            reason = "Timestamps are clamped to ≥0 with .max(0)"
        )]
        iat: now.timestamp().max(0) as u64,
        #[expect(
            clippy::cast_sign_loss,
            reason = "Timestamps are clamped to ≥0 with .max(0)"
        )]
        exp: (now + Duration::hours(24)).timestamp().max(0) as u64,
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(settings.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal(e.to_string()))
}

pub fn validate_jwt(token: &str) -> Result<Claims, AppError> {
    let settings = settings::Settings::load();
    decode::<Claims>(
        token,
        &DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .map(|data| data.claims)
    .map_err(|_| AppError::AuthError)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Simple login: accepts any `user_id` as long as the password matches the
/// hardcoded default admin password.
pub async fn login_handler(Json(req): Json<LoginRequest>) -> Result<Json<LoginResponse>, AppError> {
    let s = settings::Settings::load();
    if req.password != s.default_admin_password {
        return Err(AppError::AuthError);
    }

    let token = create_jwt(&req.user_id)?;
    Ok(Json(LoginResponse {
        token,
        user_id: req.user_id,
    }))
}

// ---------------------------------------------------------------------------
// Auth middleware (tower Layer)
// ---------------------------------------------------------------------------

/// Axum middleware that validates a `Bearer` JWT from the `Authorization`
/// header.  On success the request is forwarded; on failure a 401 is returned.
///
/// Apply it with:
/// ```ignore
/// .route_layer(middleware::from_fn(auth::auth_middleware))
/// ```
pub async fn auth_middleware(request: Request, next: Next) -> Response {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let valid = auth_header
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|token| validate_jwt(token).is_ok());

    if !valid {
        return (
            StatusCode::UNAUTHORIZED,
            Json(json!({"error": "unauthorized"})),
        )
            .into_response();
    }

    next.run(request).await
}
