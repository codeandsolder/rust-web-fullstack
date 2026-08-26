//! Application-level error types.
//!
//! All handlers return [`AppError`] which maps to appropriate HTTP status codes
//! and JSON bodies via [`IntoResponse`]. Errors are logged internally; only
//! safe, generic messages are sent to the client.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Json, Response};
use serde_json::json;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
#[must_use = "an AppError must be observed; consider logging or returning it to the caller"]
pub enum AppError {
    #[error("authentication failed")]
    AuthError,

    #[error("jwt error")]
    Jwt(#[source] jsonwebtoken::errors::Error),

    #[error("token expired")]
    TokenExpired(#[source] jsonwebtoken::errors::Error),

    #[error("invalid signature")]
    InvalidSignature(#[source] jsonwebtoken::errors::Error),

    #[error("bad request: {0}")]
    BadRequest(String),

    /// A configured dependency reached over HTTP failed. This is distinct from
    /// an application bug: callers receive 502 and internal details stay in logs.
    #[error("upstream unavailable: {context}")]
    UpstreamUnavailable {
        context: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    #[error("internal error: {context}")]
    Internal {
        context: String,
        #[source]
        source: Box<dyn std::error::Error + Send + Sync + 'static>,
    },

    #[error("invalid user id")]
    UserId(#[from] rwf_domain::UserIdError),
}

impl AppError {
    pub fn internal(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Internal {
            context: context.into(),
            source: Box::new(source),
        }
    }

    pub fn upstream(
        context: impl Into<String>,
        source: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::UpstreamUnavailable {
            context: context.into(),
            source: Box::new(source),
        }
    }
}

fn jwt_kind(e: &jsonwebtoken::errors::Error) -> String {
    format!("{:?}", e.kind())
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::AuthError => unauthenticated("Authentication failed"),
            Self::Jwt(e) => {
                tracing::debug!(kind = %jwt_kind(&e), "jwt error");
                unauthenticated("Authentication failed")
            }
            Self::TokenExpired(e) => {
                tracing::debug!(kind = %jwt_kind(&e), "token expired");
                unauthenticated("Token expired")
            }
            Self::InvalidSignature(e) => {
                tracing::warn!(kind = %jwt_kind(&e), "invalid JWT signature");
                unauthenticated("Invalid signature")
            }
            Self::BadRequest(msg) => {
                tracing::debug!(message = %msg, "bad request");
                (StatusCode::BAD_REQUEST, Json(json!({"error": msg}))).into_response()
            }
            Self::UserId(e) => {
                tracing::debug!(error = %e, "invalid user id");
                (
                    StatusCode::BAD_REQUEST,
                    Json(json!({"error": "invalid user id"})),
                )
                    .into_response()
            }
            Self::UpstreamUnavailable { context, source } => {
                tracing::warn!(context = %context, error = ?source, "upstream unavailable");
                (
                    StatusCode::BAD_GATEWAY,
                    Json(json!({"error": "upstream unavailable"})),
                )
                    .into_response()
            }
            Self::Internal { context, source } => {
                tracing::error!(context = %context, error = ?source, "internal error");
                internal_error()
            }
        }
    }
}

fn unauthenticated(message: &'static str) -> Response {
    (StatusCode::UNAUTHORIZED, Json(json!({"error": message}))).into_response()
}

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

    #[test]
    fn upstream_error_response() {
        let resp = AppError::upstream("test", std::io::Error::other("offline")).into_response();
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
    }
}
