//! Session-cookie management for the gateway.
//!
//! Sessions are the secondary auth mechanism alongside JWTs — the gateway
//! issues an `HttpOnly`/`Secure`/`SameSite=Lax` session cookie on a successful
//! login, and the `/session/whoami` route reads it back. CSRF protection is
//! layered on top via `axum_tower_sessions_csrf`.

use axum::Router;
use serde::{Deserialize, Serialize};
use tower_sessions::{MemoryStore, Session, SessionManagerLayer};

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

/// Build the session-manager layer used by the gateway router.
///
/// The layer is placed innermost in the middleware stack so that
/// [`Session`] is available to both the CSRF middleware and the
/// session handlers.
#[must_use]
pub fn session_layer(config: &SessionSettings) -> SessionManagerLayer<MemoryStore> {
    SessionManagerLayer::new(MemoryStore::default())
        .with_name(config.cookie_name.clone())
        .with_secure(config.cookie_secure)
        .with_http_only(true)
        .with_same_site(tower_sessions::cookie::SameSite::Lax)
}

/// Mount session-related routes under a common prefix.
///
/// Routes:
/// - `GET  /session/whoami` – return the authenticated user ID (or `null`)
/// - `POST /session/logout` – flush the session cookie
///
/// Generic over state so the router can be merged into any [`axum::Router`]
/// regardless of its state type (the handlers only extract [`Session`], which
/// is [`FromRequestParts`] and needs no app state).
pub fn router<S>() -> Router<S>
where
    S: Clone + Send + Sync + 'static,
{
    Router::new()
        .route("/session/whoami", axum::routing::get(whoami))
        .route("/session/logout", axum::routing::post(logout))
}

/// Return the authenticated user ID from the session, or `null` if no
/// session is active.
async fn whoami(session: Session) -> axum::Json<serde_json::Value> {
    match session.get::<SessionUser>(SESSION_USER_KEY).await {
        Ok(Some(u)) => axum::Json(serde_json::json!({ "user_id": u.user_id })),
        Ok(None) => axum::Json(serde_json::json!({ "user_id": null })),
        Err(e) => {
            tracing::warn!(error = %e, "session whoami: get failed");
            axum::Json(serde_json::json!({ "error": "session error" }))
        }
    }
}

/// Flush the session, removing the cookie and all stored data.
async fn logout(session: Session) -> axum::Json<serde_json::Value> {
    if let Err(e) = session.flush().await {
        tracing::warn!(error = %e, "session logout: flush failed");
    }
    axum::Json(serde_json::json!({ "ok": true }))
}
