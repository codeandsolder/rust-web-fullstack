//! Authentication: JWT creation, validation, login handler, and auth middleware.
//!
//! # Security
//!
//! * Password comparison uses constant-time comparison via `subtle`.
//! * Login is rate-limited per IP address (5 attempts per 60-second window).
//! * JWT errors are mapped to distinct error variants (expired, invalid
//!   signature, etc.) so callers can differentiate.
//! * JWT validation checks `iss` and `aud` claims against `"gateway-example"`.
//! * Timestamp overflow is treated as an internal error rather than silently
//!   producing `exp = 0` (which jsonwebtoken treats as already-expired).
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

/// JWT issuer and audience value — shared between encode and decode so they
/// cannot drift.
const JWT_ISS: &str = "gateway-example";

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

    /// Maximum number of tracked IPs before an opportunistic prune is triggered.
    ///
    /// When the map exceeds this limit, the prune cycles through entries,
    /// finds the median window-start, and drops everything older than it,
    /// halving the map size.  This bounds memory usage during e.g. a
    /// distributed brute-force attack.
    const MAX_TRACKED_IPS: usize = 50_000;

    /// Check whether `ip` has exceeded the rate limit.
    ///
    /// Returns `Ok(())` if the request is allowed, `Err(AppError::AuthError)`
    /// if rate-limited. The counter is incremented **before** the password
    /// check, so a legitimate user with 5 typos in a 60 s window is locked out
    /// along with a real attacker.
    async fn check(&self, ip: IpAddr) -> Result<(), AppError> {
        // Phase 1: short critical section — bump the bucket, optionally
        // record how many entries to drop. The actual collect/sort/retain
        // happens **outside** the lock to keep the critical section O(1)
        // even when the map is at `MAX_TRACKED_IPS`.
        let cutoff = {
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

            if map.len() > Self::MAX_TRACKED_IPS {
                // Find the median window_start under the lock, then drop the
                // older half in a second short critical section. This bounds
                // memory under brute-force probing without serialising every
                // login attempt on an O(N log N) sort.
                let mut starts: Vec<Instant> = map.values().map(|b| b.window_start).collect();
                starts.sort_unstable();
                let median = starts[starts.len() / 2];
                drop(map);
                Some(median)
            } else {
                // Opportunistic stale-entry prune: keep only entries whose
                // window has not fully elapsed. `Instant` is monotonic, so
                // `checked_sub` only fails if `now` predates the epoch —
                // fall back to `now` (no cleanup) in that impossible case.
                let stale = now.checked_sub(Self::WINDOW).unwrap_or(now);
                drop(map);
                Some(stale)
            }
        };

        // Phase 2: a second short lock just for the retain.
        if let Some(stale) = cutoff {
            let mut map = self.inner.lock().await;
            map.retain(|_, bucket| bucket.window_start >= stale);
        }

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
#[must_use = "an AppError must be observed; consider logging or returning it to the caller"]
pub enum AppError {
    /// Authentication failed (bad password, rate-limited, etc.).
    #[error("authentication failed")]
    AuthError,

    /// Generic JWT error.
    #[error("jwt error")]
    Jwt(#[source] jsonwebtoken::errors::Error),

    /// The JWT has expired.
    #[error("token expired")]
    TokenExpired(#[source] jsonwebtoken::errors::Error),

    /// The JWT signature is invalid.
    #[error("invalid signature")]
    InvalidSignature(#[source] jsonwebtoken::errors::Error),

    /// Internal error that should not leak details to the client.
    #[error("internal error: {context}")]
    Internal {
        /// Human-readable context describing what operation failed.
        context: String,
        /// The underlying error.
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },
}

impl AppError {
    /// Construct an [`AppError::Internal`] without pulling in `anyhow`.
    pub fn internal(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Internal {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

/// Render a JWT `ErrorKind` for tracing **without** echoing the underlying
/// message — some `ErrorKind` variants (notably `Json(_)` and `Base64(_)`)
/// wrap `Display` impls that include slices of the offending token payload,
/// which would exfiltrate JWT claims into the log stream.
fn jwt_kind(e: &jsonwebtoken::errors::Error) -> String {
    // `Debug` on `ErrorKind` produces a stable variant name without leaking
    // the inner message bytes.
    format!("{:?}", e.kind())
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
                // Routine attacker probing — keep at `debug!` to avoid drowning
                // the alert channel; never echo the error message itself.
                tracing::debug!(kind = %jwt_kind(&e), "jwt error");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Authentication failed"})),
                )
                    .into_response()
            }
            Self::TokenExpired(e) => {
                tracing::warn!(kind = %jwt_kind(&e), "token expired");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Token expired"})),
                )
                    .into_response()
            }
            Self::InvalidSignature(e) => {
                tracing::warn!(kind = %jwt_kind(&e), "invalid JWT signature");
                (
                    StatusCode::UNAUTHORIZED,
                    Json(json!({"error": "Invalid signature"})),
                )
                    .into_response()
            }
            Self::Internal { context, source } => {
                // `?source` uses `Debug` and walks the chain via `tracing-error`
                // if installed; without it we still see the top-level error.
                // We do NOT include `source` chain contents that may contain
                // secrets — context is the only operator-readable label here.
                tracing::error!(
                    context = %context,
                    error = ?source,
                    "internal error"
                );
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

/// Convert a [`DateTime<Utc>`] to a `u64` Unix timestamp, returning an
/// [`AppError::Internal`] on overflow (e.g. dates before epoch).
fn unix_seconds(t: chrono::DateTime<Utc>) -> Result<u64, AppError> {
    u64::try_from(t.timestamp()).map_err(|e| AppError::internal("timestamp overflow", e))
}

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
    let exp = unix_seconds(now + chrono::Duration::hours(24))?;

    let claims = Claims {
        sub: user_id.to_string(),
        iat: unix_seconds(now)?,
        exp,
        aud: JWT_ISS.to_string(),
        iss: JWT_ISS.to_string(),
    };

    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(settings.jwt_secret.as_bytes()),
    )
    .map_err(|e| AppError::internal("JWT encoding", e))
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

    let mut validation = Validation::new(Algorithm::HS256);
    validation.set_issuer(&[JWT_ISS]);
    validation.set_audience(&[JWT_ISS]);

    decode::<Claims>(
        token,
        &DecodingKey::from_secret(settings.jwt_secret.as_bytes()),
        &validation,
    )
    .map(|data| data.claims)
    .map_err(|e| match e.kind() {
        ErrorKind::ExpiredSignature => AppError::TokenExpired(e),
        ErrorKind::InvalidSignature => AppError::InvalidSignature(e),
        _ => AppError::Jwt(e),
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
#[instrument(skip_all, fields(user_id = %req.user_id))]
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
