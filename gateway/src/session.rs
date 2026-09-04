//! Session-cookie management for the gateway.
//!
//! Sessions are the secondary auth mechanism alongside JWTs — the gateway
//! issues an `HttpOnly`/`Secure`/`SameSite=Lax` session cookie on a successful
//! login, and the `/session/whoami` route reads it back. CSRF protection is
//! layered on top via `axum_tower_sessions_csrf`.
//!
//! This example intentionally uses `MemoryStore`, so session records are
//! process-local: restarting the gateway invalidates them, and separate
//! gateway replicas do not share authenticated session state. A production
//! horizontally scaled deployment should provide a shared `SessionStore`
//! (or omit server-side sessions) rather than treating this demo store as
//! durable or cluster-wide.

use axum::Router;
use serde::{Deserialize, Serialize};
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};

use crate::auth::AppError;
use crate::settings::SessionSettings;

/// Session cookie name.
pub const SESSION_COOKIE_NAME: &str = "rwf_session";

/// Session key under which the authenticated user record is stored.
pub const SESSION_USER_KEY: &str = "user_id";

/// Authenticated user record stored in the session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionUser {
    /// UUID-as-string for cookie-safety (no binary in JSON).
    pub user_id: String,
}

/// Build the process-local demo session-manager layer used by the gateway.
///
/// `MemoryStore` is intentionally suitable for this single-process example,
/// not as shared session state for a horizontally scaled deployment.
#[must_use]
pub fn session_layer(config: &SessionSettings) -> SessionManagerLayer<MemoryStore> {
    SessionManagerLayer::new(MemoryStore::default())
        .with_name(config.cookie_name.clone())
        .with_secure(config.cookie_secure)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
}

/// Mount session-related routes under a common prefix.
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/session/whoami", axum::routing::get(whoami))
        .route("/session/logout", axum::routing::post(logout))
}

/// Return the authenticated user ID from the session, or `null` if no session
/// is active. Session-store failures are server errors, not successful JSON
/// responses carrying an `error` field.
async fn whoami(session: Session) -> Result<axum::Json<serde_json::Value>, AppError> {
    let user = session
        .get::<SessionUser>(SESSION_USER_KEY)
        .await
        .map_err(|e| AppError::internal("session whoami", e))?;

    Ok(match user {
        Some(user) => axum::Json(serde_json::json!({ "user_id": user.user_id })),
        None => axum::Json(serde_json::json!({ "user_id": null })),
    })
}

/// Flush only the current cookie session, removing its cookie and stored data.
///
/// This route does not revoke refresh tokens or access JWTs. Use `/auth/logout`
/// when the caller needs the gateway's combined refresh-token/current-session
/// logout semantics. Do not report success if the backing store failed to
/// persist this session invalidation.
async fn logout(session: Session) -> Result<axum::Json<serde_json::Value>, AppError> {
    session
        .flush()
        .await
        .map_err(|e| AppError::internal("session logout", e))?;
    Ok(axum::Json(serde_json::json!({ "ok": true })))
}
