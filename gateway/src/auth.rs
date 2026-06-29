//! Authentication: JWT creation, validation, login handler, and auth middleware.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * Login is rate-limited per IP address (5 attempts per 60-second window).
//! * JWT errors are mapped to distinct error variants (expired, invalid
//!   signature, etc.) so callers can differentiate.
//! * Secrets (passwords, tokens, JWT secrets) are never included in log output.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{ConnectInfo, Request, State},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Json, Response},
};
use chrono::Utc;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use subtle::ConstantTimeEq;
use tokio::sync::Mutex;
use tracing::instrument;

use crate::gateway::GatewayState;
use crate::settings;

// ---------------------------------------------------------------------------
// Claims
// ---------------------------------------------------------------------------

/// JWT claims payload.
///
/// Standard fields: `sub` (subject), `exp` (expiration), `iat` (issued-at),
/// `aud` (audience), `iss` (issuer).
#[derive(Debug, Serialize, Deserialize, Clone)]
#[non_exhaustive]
#[must_use]
pub struct Claims {
    pub sub: String,
    pub exp: u64,
    pub iat: u64,
    pub aud: String,
    pub iss: String,
}

// ---------------------------------------------------------------------------
// Request / Response DTOs
// ---------------------------------------------------------------------------

/// Login request payload.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub user_id: String,
    pub password: String,
}

/// Login response containing the signed JWT.
#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub token: String,
    pub user_id: String,
}

// ---------------------------------------------------------------------------
// Per-IP login rate limiter
// ---------------------------------------------------------------------------

/// Per-IP rate limit state for login attempts.
///
/// Tracks request counts per IP within rolling 60-second windows.
#[derive(Clone, Debug)]
pub struct LoginRateLimiter {
    inner: Arc<Mutex<HashMap<IpAddr, RateLimitBucket>>>,
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

#[derive(Clone, Debug)]
struct RateLimitBucket {
    count: u32,
    window_start: Instant,
}

impl LoginRateLimiter {
    /// Maximum login attempts allowed per IP within the rate-limit window.
    const MAX_ATTEMPTS: u32 = 5;

    /// Duration of the rate-limit window in seconds.
    const WINDOW: Duration = Duration::from_secs(60);

    /// Check whether `ip` has exceeded the rate limit.
    ///
    /// Returns `Ok(())` if the request is allowed, `Err(AppError::AuthError)`
    /// if rate-limited.
    #[allow(clippy::significant_drop_tightening)]
    async fn check(&self, ip: IpAddr) -> Result<(), AppError> {
        let now = Instant::now();
        let mut map = self.inner.lock().await;
        let entry = map.entry(ip).or_insert(RateLimitBucket {
            count: 0,
            window_start: now,
        });

        if now.duration_since(entry.window_start) > Self::WINDOW {
            entry.count = 0;
            entry.window_start = now;
        }

        if entry.count >= Self::MAX_ATTEMPTS {
            return Err(AppError::AuthError);
        }

        entry.count += 1;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Error type
// ---------------------------------------------------------------------------

/// Application-level error enum.
///
/// Maps to appropriate HTTP status codes and JSON bodies in
/// [`IntoResponse`]. Errors are logged internally; only safe, generic
/// messages are sent to the client.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// Authentication failed (bad password, rate-limited, etc.).
    #[error("Authentication failed")]
    AuthError,

    /// Generic JWT error.
    #[error("JWT error")]
    Jwt(#[source] jsonwebtoken::errors::Error),

    /// The JWT has expired.
    #[error("Token expired")]
    TokenExpired(#[source] jsonwebtoken::errors::Error),

    /// The JWT signature is invalid.
    #[error("Invalid signature")]
    InvalidSignature(#[source] jsonwebtoken::errors::Error),

    /// Internal error that should not leak details to the client.
    #[error("Internal error")]
    Internal {
        /// The underlying error.
        #[source]
        source: anyhow::Error,
    },
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::AuthError => (
                StatusCode::UNAUTHORIZED,
                Json(json!({"error": "Authentication failed"})),
            )
                .into_response(),
            Self::Jwt(e) => {
                tracing::error!(error = %e, "JWT error");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Authentication failed"})),
                )
                    .into_response()
            }
            Self::TokenExpired(e) => {
                tracing::warn!(error = %e, "Token expired");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Token expired"})),
                )
                    .into_response()
            }
            Self::InvalidSignature(e) => {
                tracing::warn!(error = %e, "Invalid JWT signature");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Invalid signature"})),
                )
                    .into_response()
            }
            Self::Internal { source } => {
                tracing::error!(error = %source, "internal error");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({"error": "internal error"})),
                )
                    .into_response()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// JWT helpers
// ---------------------------------------------------------------------------

/// Create a signed JWT for the given `user_id`.
///
/// The token expires 24 hours from creation.
///
/// # Errors
///
/// Returns [`AppError::Internal`] if token encoding fails.
#[instrument(skip(settings))]
pub fn create_jwt(user_id: &str, settings: &settings::Settings) -> Result<String, AppError> {
    let now = Utc::now();
    let exp = u64::try_from((now + chrono::Duration::hours(24)).timestamp()).unwrap_or(0);

    let claims = Claims {
        sub: user_id.to_string(),
        iat: u64::try_from(now.timestamp()).unwrap_or(0),
        exp,
        aud: "gateway-example".to_string(),
        iss: "gateway-example".to_string(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(settings.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::Internal {
        source: anyhow::anyhow!("JWT encoding failed: {e}"),
    })
}

/// Validate a JWT and return its [`Claims`].
///
/// # Errors
///
/// Returns [`AppError::TokenExpired`] if the token has expired,
/// [`AppError::InvalidSignature`] if the signature is invalid, or
/// [`AppError::Jwt`] for other decoding errors.
#[instrument(skip(settings, token))]
pub fn validate_jwt(token: &str, settings: &settings::Settings) -> Result<Claims, AppError> {
    use jsonwebtoken::errors::ErrorKind;

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
        &Validation::new(Algorithm::HS256),
    )
    .map(|data| data.claims)
    .map_err(|e| {
        use jsonwebtoken::errors::new_error;
        match e.into_kind() {
            ErrorKind::ExpiredSignature => {
                AppError::TokenExpired(new_error(ErrorKind::ExpiredSignature))
            }
            ErrorKind::InvalidSignature => {
                AppError::InvalidSignature(new_error(ErrorKind::InvalidSignature))
            }
            kind => AppError::Jwt(new_error(kind)),
        }
    })
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Authenticate a user with `user_id` and `password`.
///
/// Returns a signed JWT on success.  The endpoint is rate-limited per
/// source IP to 5 attempts per 60 seconds.
///
/// # Errors
///
/// Returns [`AppError::AuthError`] if the password is wrong or the IP is
/// rate-limited.  Returns [`AppError::Internal`] if JWT signing fails.
#[instrument(skip(state, req), fields(user_id = %req.user_id))]
pub async fn login_handler(
    ConnectInfo(addr): ConnectInfo<std::net::SocketAddr>,
    State(state): State<GatewayState>,
    Json(req): Json<LoginRequest>,
) -> Result<Json<LoginResponse>, AppError> {
    // Rate-limit per source IP.
    state.rate_limiter.check(addr.ip()).await?;

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

    let token = create_jwt(&req.user_id, s)?;
    Ok(Json(LoginResponse {
        token,
        user_id: req.user_id,
    }))
}

// ---------------------------------------------------------------------------
// Auth middleware (tower Layer)
// ---------------------------------------------------------------------------

/// Axum middleware that validates a `Bearer` JWT from the `Authorization`
/// header.
///
/// On success the request is forwarded with [`Claims`] injected into the
/// request extensions (accessible via `Extension<Claims>`).  On failure a
/// 401 is returned.
///
/// Apply it with:
/// ```ignore
/// .route_layer(middleware::from_fn_with_state(state, auth::auth_middleware))
/// ```
#[instrument(skip(request, next, state))]
pub async fn auth_middleware(
    State(state): State<GatewayState>,
    mut request: Request,
    next: Next,
) -> Response {
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    let claims = auth_header
        .and_then(|v| v.strip_prefix("Bearer "))
        .and_then(|token| validate_jwt(token, &state.settings).ok());

    let Some(claims) = claims else {
        return AppError::AuthError.into_response();
    };

    // Inject claims into request extensions so downstream handlers can
    // extract them via `Extension<Claims>`.
    request.extensions_mut().insert(claims);

    next.run(request).await
}
