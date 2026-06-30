//! Application-level error types.
//!
//! All handlers return [`AppError`] which maps to appropriate HTTP status codes
//! and JSON bodies via [`IntoResponse`].  Errors are logged internally; only
//! safe, generic messages are sent to the client.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;

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

/// Render a JWT `ErrorKind` for tracing using the stable variant name only.
///
/// Using `Display` here would format the underlying message of variants like
/// `Json(_)` / `Base64(_)` / `Utf8(_)`, which (depending on upstream
/// versions) could echo slices of the offending token payload. `Debug` on
/// `ErrorKind` produces just the variant name (`"InvalidToken"`,
/// `"Base64"`, …) without leaking message bytes.
fn jwt_kind(e: &jsonwebtoken::errors::Error) -> String {
    format!("{:?}", e.kind())
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::AuthError => unauthenticated("Authentication failed"),
            Self::Jwt(e) => {
                // Routine attacker probing — keep at `debug!` to avoid drowning
                // the alert channel; never echo the error message itself.
                tracing::debug!(kind = %jwt_kind(&e), "jwt error");
                unauthenticated("Authentication failed")
            }
            Self::TokenExpired(e) => {
                tracing::warn!(kind = %jwt_kind(&e), "token expired");
                unauthenticated("Token expired")
            }
            Self::InvalidSignature(e) => {
                tracing::warn!(kind = %jwt_kind(&e), "invalid JWT signature");
                unauthenticated("Invalid signature")
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
                internal_error()
            }
        }
    }
}

/// Build a generic 401 UNAUTHORIZED response with a static client-safe message.
///
/// Kept private — the only public mapping is [`AppError::into_response`].
/// Centralising the layout lets us adjust headers / structured-log fields in
/// one place without touching each variant.
fn unauthenticated(message: &'static str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": message}))).into_response()
}

/// Build a generic 500 `INTERNAL_SERVER_ERROR` response with a static
/// client-safe message.
///
/// The actual error context and source are logged separately via `tracing`.
fn internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(json!({"error": "internal error"})),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_response() {
        let resp = AppError::AuthError.into_response();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}
